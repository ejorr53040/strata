//! The agent-facing tool surface: one catalog and one dispatcher shared by the CLI
//! (`strata tools` / `strata call`) and, later, the MCP server and `strata ask`.
//! Every tool takes a JSON object and returns JSON, so a model can call it directly.

use crate::walk::WalkOptions;
use crate::{graph, index, walk};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub fn catalog() -> Vec<Value> {
    let none = json!({"type": "object", "properties": {}});
    let sym = |what: &str| {
        json!({
            "type": "object",
            "properties": {"symbol": {"type": "string", "description": what}},
            "required": ["symbol"]
        })
    };
    vec![
        json!({
            "name": "overview",
            "description": "Start here. Counts of indexed/unindexed files, symbols by kind and call edges by state, so you know how far to trust the graph before querying it.",
            "input_schema": none,
        }),
        json!({
            "name": "symbols",
            "description": "Find definitions (functions, methods, classes, structs, enums, traits) whose name contains the query, case-insensitive. An empty query lists everything.",
            "input_schema": {
                "type": "object",
                "properties": {"query": {"type": "string", "description": "Name substring, e.g. `walk` or `Circle.`"}},
                "required": ["query"]
            },
        }),
        json!({
            "name": "callers",
            "description": "Call sites whose resolved or candidate targets include the symbol. Use before changing a signature to find what breaks. Each edge carries a state: resolved, external, candidate (every possible target, never a guess), unresolved (with reason), unindexed, observed.",
            "input_schema": sym("Fully qualified id or a unique dotted suffix, e.g. `walk.walk`"),
        }),
        json!({
            "name": "callees",
            "description": "Every call site inside the symbol, each with its edge state and targets. Use to understand what a function depends on.",
            "input_schema": sym("Fully qualified id or a unique dotted suffix, e.g. `graph.build`"),
        }),
        json!({
            "name": "references",
            "description": "Every connection of any kind (call, and as depth grows: type use, value use, field, variant, impl, import) whose targets include the symbol.",
            "input_schema": sym("Fully qualified id or a unique dotted suffix"),
        }),
        json!({
            "name": "edges",
            "description": "Dump the whole connection graph: every edge of every kind with state, targets and location. Large; prefer callers/callees/references for a single symbol.",
            "input_schema": none,
        }),
        json!({
            "name": "unindexed_files",
            "description": "Source files that were skipped and why (gitignore, strataignore, vendored, unsupported-language, parse-failure). Check this when a symbol you expect is missing.",
            "input_schema": none,
        }),
    ]
}

/// Run one tool against the repo at `root`. Errors are plain strings meant for the caller
/// (a human or a model) to act on.
pub fn call(root: &Path, opts: &WalkOptions, name: &str, args: &Value) -> Result<Value, String> {
    let str_arg = |key: &str| -> Result<&str, String> {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("tool `{name}` requires a string argument `{key}`"))
    };
    match name {
        "overview" => Ok(overview(root, opts)),
        "symbols" => Ok(json!(index::symbols_with(root, str_arg("query")?, opts))),
        "callers" => {
            let g = graph::build(&graph::load_with(root, opts), &[]);
            Ok(json!(g.callers(str_arg("symbol")?)))
        }
        "callees" => {
            let g = graph::build(&graph::load_with(root, opts), &[]);
            Ok(json!(g.callees(str_arg("symbol")?)))
        }
        "references" => {
            let g = graph::build(&graph::load_with(root, opts), &[]);
            Ok(json!(g.references(str_arg("symbol")?)))
        }
        "edges" => Ok(json!(
            graph::build(&graph::load_with(root, opts), &[]).edges
        )),
        "unindexed_files" => {
            let w = walk::walk(root, opts);
            Ok(json!(w
                .unindexed
                .iter()
                .map(|s| json!({"path": s.rel, "reason": s.reason.label()}))
                .collect::<Vec<_>>()))
        }
        _ => Err(format!(
            "unknown tool `{name}`; available: {}",
            catalog()
                .iter()
                .filter_map(|t| t["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn overview(root: &Path, opts: &WalkOptions) -> Value {
    let w = walk::walk(root, opts);
    let sources = graph::load_with(root, opts);
    let mut symbols: BTreeMap<&str, usize> = BTreeMap::new();
    for s in index::symbols_with(root, "", opts) {
        *symbols.entry(s.kind).or_default() += 1;
    }
    let mut edges: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &graph::build(&sources, &[]).edges {
        *edges.entry(e.state.label()).or_default() += 1;
    }
    json!({
        "files": {"indexed": w.indexed.len(), "unindexed": w.unindexed.len()},
        "symbols": symbols,
        "edges": edges,
    })
}
