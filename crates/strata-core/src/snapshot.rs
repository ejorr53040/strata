//! One in-memory index of a repo, built once and queried many times (by the daemon, and by
//! one-shot callers through [`crate::tools::call`]).

use crate::graph::{self, Graph};
use crate::index::{self, Symbol};
use crate::map::{self, Map};
use crate::markup::{self, Markup};
use crate::walk::{self, Skipped, WalkOptions};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub struct Snapshot {
    pub(crate) sources: Vec<graph::Source>,
    pub(crate) unindexed: Vec<Skipped>,
    pub(crate) symbols: Vec<Symbol>,
    pub(crate) graph: Graph,
    pub(crate) markup: Markup,
    maps: Mutex<HashMap<(Option<u8>, Option<usize>), Arc<Map>>>,
    /// (file, size, mtime) of every indexed file, for [`Snapshot::is_stale`].
    stamps: Vec<(PathBuf, Option<(u64, SystemTime)>)>,
}

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}

impl Snapshot {
    pub fn build(root: &Path, opts: &WalkOptions) -> Snapshot {
        // ponytail: stamps are taken after the read, so an edit landing mid-build is missed
        // until the next one. #6's watcher replaces this check.
        let w = walk::walk(root, opts);
        let stamps = w
            .indexed
            .iter()
            .map(|f| {
                let p = root.join(&f.rel);
                let s = stamp(&p);
                (p, s)
            })
            .collect();
        let sources: Vec<graph::Source> = w
            .indexed
            .into_iter()
            .map(|f| graph::Source {
                path: f.rel,
                text: f.text,
            })
            .collect();
        Snapshot {
            symbols: index::all(&sources),
            graph: graph::build(&sources, &[]),
            markup: markup::build(&sources),
            sources,
            unindexed: w.unindexed,
            maps: Mutex::default(),
            stamps,
        }
    }

    /// Run one agent tool (see [`crate::tools::catalog`]).
    pub fn call(&self, tool: &str, args: &Value) -> Result<Value, String> {
        crate::tools::dispatch(self, tool, args)
    }

    /// True once any indexed file changed size or mtime, or disappeared. New files are not
    /// seen: that needs a walk.
    pub fn is_stale(&self) -> bool {
        self.stamps.iter().any(|(p, s)| stamp(p) != *s)
    }

    pub fn indexed_files(&self) -> usize {
        self.sources.len()
    }

    pub fn unindexed(&self) -> &[Skipped] {
        &self.unindexed
    }

    pub fn symbols(&self, query: &str) -> Vec<Symbol> {
        index::filter(&self.symbols, query)
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The map at `level` / within `budget`, built on first use and kept.
    pub fn map(&self, level: Option<u8>, budget: Option<usize>) -> Arc<Map> {
        let mut maps = self.maps.lock().unwrap();
        maps.entry((level, budget))
            .or_insert_with(|| Arc::new(map::build_from(&self.sources, &self.graph, level, budget)))
            .clone()
    }
}
