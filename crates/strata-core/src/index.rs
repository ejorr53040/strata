use crate::lang;
use crate::walk::WalkOptions;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Symbol {
    pub kind: &'static str,
    pub name: String,
    pub path: String,
    pub line: usize,
}

/// Index every non-skipped source file under `root` and return symbols whose name contains
/// `query` (case-insensitive), sorted by path then line.
pub fn symbols(root: &Path, query: &str) -> Vec<Symbol> {
    symbols_with(root, query, &WalkOptions::default())
}

/// Like [`symbols`], with explicit walk options (e.g. `index.include`).
pub fn symbols_with(root: &Path, query: &str, opts: &WalkOptions) -> Vec<Symbol> {
    let sources: Vec<crate::graph::Source> = crate::walk::walk(root, opts)
        .indexed
        .into_iter()
        .map(|f| crate::graph::Source {
            path: f.rel,
            text: f.text,
        })
        .collect();
    filter(&all(&sources), query)
}

/// Every module and definition in `sources`, in source order.
pub fn all(sources: &[crate::graph::Source]) -> Vec<Symbol> {
    let mut out = Vec::new();
    for file in sources {
        let Some(language) = lang::for_path(&file.path) else {
            continue;
        };
        let facts = language.extract(&file.path, &file.text);
        out.push(Symbol {
            kind: "module",
            name: language.module_name(&file.path),
            path: file.path.clone(),
            line: 1,
        });
        out.extend(facts.defs.into_iter().map(|d| Symbol {
            kind: d.kind.label(),
            name: d.qualname,
            path: file.path.clone(),
            line: d.line,
        }));
    }
    out
}

/// Symbols whose name contains `query`, case-insensitive.
pub fn filter(symbols: &[Symbol], query: &str) -> Vec<Symbol> {
    let q = query.to_lowercase();
    symbols
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&q))
        .cloned()
        .collect()
}
