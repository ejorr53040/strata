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
    let q = query.to_lowercase();
    let mut out = Vec::new();
    for file in crate::walk::walk(root, opts).indexed {
        let Some(language) = lang::for_path(&file.rel) else {
            continue;
        };
        let facts = language.extract(&file.rel, &file.text);
        out.extend(facts.defs.into_iter().map(|d| Symbol {
            kind: d.kind.label(),
            name: d.qualname,
            path: file.rel.clone(),
            line: d.line,
        }));
    }
    out.retain(|s| s.name.to_lowercase().contains(&q));
    out
}
