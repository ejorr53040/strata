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

/// Maps built so far, by (level, budget).
type MapCache = Mutex<HashMap<(Option<u8>, Option<usize>), Arc<Map>>>;

pub struct Snapshot {
    pub(crate) sources: Vec<graph::Source>,
    pub(crate) unindexed: Vec<Skipped>,
    pub(crate) symbols: Vec<Symbol>,
    pub(crate) graph: Graph,
    pub(crate) markup: Markup,
    maps: MapCache,
    /// (file, size, mtime) of every indexed file, for [`Snapshot::is_stale`].
    stamps: Vec<(PathBuf, Option<(u64, SystemTime)>)>,
    /// Each parsed file's facts by path, reused by [`Snapshot::refresh`].
    extracted: HashMap<String, graph::Extracted>,
    /// Full builds in this snapshot's lineage, and files re-parsed by refreshes since.
    builds: usize,
    reindexed: usize,
}

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}

impl Snapshot {
    pub fn build(root: &Path, opts: &WalkOptions) -> Snapshot {
        let w = walk::walk(root, opts);
        let extracted = w
            .indexed
            .iter()
            .filter_map(|f| {
                let s = graph::Source {
                    path: f.rel.clone(),
                    text: f.text.clone(),
                };
                graph::extract(&s).map(|e| (f.rel.clone(), e))
            })
            .collect();
        Snapshot::assemble(root, w, extracted, 1, 0)
    }

    /// The snapshot for the tree as it is now: a refresh when only file contents changed,
    /// otherwise a full build (counted in [`Snapshot::builds`]).
    pub fn update(&self, root: &Path, opts: &WalkOptions) -> Snapshot {
        self.refresh(root, opts).unwrap_or_else(|| {
            let mut s = Snapshot::build(root, opts);
            s.builds += self.builds;
            s.reindexed = self.reindexed;
            s
        })
    }

    /// Parses only the files whose text changed since this snapshot was built and
    /// re-resolves every edge over the result. `None` when the set of files differs (one
    /// appeared, vanished or is newly skipped): that needs a full [`Snapshot::build`].
    fn refresh(&self, root: &Path, opts: &WalkOptions) -> Option<Snapshot> {
        let w = walk::walk(root, opts);
        let same_files = w.indexed.len() == self.sources.len()
            && w.indexed
                .iter()
                .zip(&self.sources)
                .all(|(a, b)| a.rel == b.path)
            && w.unindexed.len() == self.unindexed.len()
            && w.unindexed
                .iter()
                .zip(&self.unindexed)
                .all(|(a, b)| a.rel == b.rel);
        if !same_files {
            return None;
        }
        let mut reindexed = 0;
        let mut extracted = HashMap::new();
        for (new, old) in w.indexed.iter().zip(&self.sources) {
            let kept = (new.text == old.text)
                .then(|| self.extracted.get(&new.rel))
                .flatten();
            let facts = match kept {
                Some(e) => Some(e.clone()),
                None => {
                    reindexed += 1;
                    graph::extract(&graph::Source {
                        path: new.rel.clone(),
                        text: new.text.clone(),
                    })
                }
            };
            if let Some(e) = facts {
                extracted.insert(new.rel.clone(), e);
            }
        }
        Some(Snapshot::assemble(
            root,
            w,
            extracted,
            self.builds,
            self.reindexed + reindexed,
        ))
    }

    fn assemble(
        root: &Path,
        w: walk::Walk,
        extracted: HashMap<String, graph::Extracted>,
        builds: usize,
        reindexed: usize,
    ) -> Snapshot {
        // ponytail: stamps are taken after the read, so an edit landing mid-build is missed
        // until the next one. #6's watcher replaces this check.
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
        let (symbols, graph) = {
            let ordered: Vec<&graph::Extracted> = sources
                .iter()
                .filter_map(|s| extracted.get(&s.path))
                .collect();
            (
                index::from_extracted(&ordered),
                graph::resolve(&ordered, &[]),
            )
        };
        Snapshot {
            symbols,
            graph,
            // ponytail: markup (HTML/CSS) is rebuilt from every source on each refresh.
            markup: markup::build(&sources),
            sources,
            unindexed: w.unindexed,
            maps: Mutex::default(),
            stamps,
            extracted,
            builds,
            reindexed,
        }
    }

    /// Full builds behind this snapshot (a refresh does not add one).
    pub fn builds(&self) -> usize {
        self.builds
    }

    /// Files re-parsed by refreshes since the last full build.
    pub fn reindexed(&self) -> usize {
        self.reindexed
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
