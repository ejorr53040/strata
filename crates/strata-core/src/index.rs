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
    let sources = crate::graph::load_with(root, opts);
    filter(&all(&sources), query)
}

/// Every module and definition in `sources`, in source order.
pub fn all(sources: &[crate::graph::Source]) -> Vec<Symbol> {
    let extracted: Vec<_> = sources.iter().filter_map(crate::graph::extract).collect();
    from_extracted(&extracted.iter().collect::<Vec<_>>())
}

/// Like [`all`], over facts that are already parsed.
pub fn from_extracted(files: &[&crate::graph::Extracted]) -> Vec<Symbol> {
    let mut out = Vec::new();
    for file in files {
        let Some(language) = lang::for_path(&file.path) else {
            continue;
        };
        out.push(Symbol {
            kind: "module",
            name: language.module_name(&file.path),
            path: file.path.clone(),
            line: 1,
        });
        out.extend(file.facts.defs.iter().map(|d| Symbol {
            kind: d.kind.label(),
            name: d.qualname.clone(),
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
