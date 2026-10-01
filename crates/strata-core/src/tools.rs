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
            "name": "map",
            "description": "The whole repo as compact text: every module, definition (with signature and doc summary) and, under each, what it calls and what calls it (plus uses, impls, types, fields at higher levels). Read this first to know the codebase without opening files. `level` 0-3 sets detail; `budget_tokens` picks the richest level that fits.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "level": {"type": "integer", "minimum": 0, "maximum": 3},
                    "budget_tokens": {"type": "integer", "minimum": 1}
                }
            },
        }),
        json!({
            "name": "matched_elements",
            "description": "HTML elements a CSS selector (or comma-separated list) can apply to, by path and line of the start tag. State pseudo-classes (:hover) are assumed to match; template syntax in class/id ({{ }}, {% %}) is skipped.",
            "input_schema": {
                "type": "object",
                "properties": {"selector": {"type": "string", "description": "A CSS selector, e.g. `#main > .card`"}},
                "required": ["selector"]
            },
        }),
        json!({
            "name": "matching_rules",
            "description": "CSS rules that can apply to the HTML element(s) whose start tag is at `path:line`, from lowest to highest precedence (specificity, then source order across the page's linked and inline stylesheets), and for each property the rule that wins (!important first; state rules like :hover and pseudo-elements are listed but never win).",
            "input_schema": {
                "type": "object",
                "properties": {"element": {"type": "string", "description": "`path:line` of the element's start tag, e.g. `templates/index.html:12`"}},
                "required": ["element"]
            },
        }),
        json!({
            "name": "unused_selectors",
            "description": "CSS selectors that match no element on any indexed HTML page that loads their stylesheet (a stylesheet no page links is unused throughout). Elements built by JavaScript are not seen, so check before deleting.",
            "input_schema": none,
        }),
        json!({
            "name": "selector_refs",
            "description": "DOM lookups in JavaScript (querySelector(All), getElementById, getElementsByClassName/TagName, closest, matches, classList.*) linked to the HTML elements they can match on the pages whose <script src> loads the file, and the CSS rules that mention the class, id or tag they target. Selectors built at runtime are `unresolved` with the expression.",
            "input_schema": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "Only lookups in this script file, e.g. `static/app.js`"}}
            },
        }),
        json!({
            "name": "pages",
            "description": "Every indexed HTML page: the stylesheet files that apply to it (cascade order), its <style> block count, the indexed scripts its <script src> tags load, and the ids and classes its elements carry.",
            "input_schema": none,
        }),
        json!({
            "name": "stylesheets",
            "description": "Every stylesheet (a .css file, or a page's <style> blocks under the page's path) with each rule's selector, line, and whether it is unused (see unused_selectors).",
            "input_schema": none,
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
        "map" => {
            let level = args.get("level").and_then(Value::as_u64).map(|l| l as u8);
            let budget = args
                .get("budget_tokens")
                .and_then(Value::as_u64)
                .map(|b| b as usize);
            Ok(json!(crate::map::summary(&crate::map::build(
                root, opts, level, budget
            ))))
        }
        "matched_elements" => {
            let m = crate::markup::build(&graph::load_with(root, opts));
            Ok(json!(m.matched_elements(str_arg("selector")?)))
        }
        "matching_rules" => {
            let element = str_arg("element")?;
            let (path, line) = element
                .rsplit_once(':')
                .and_then(|(p, l)| Some((p, l.parse::<usize>().ok()?)))
                .ok_or_else(|| format!("`element` must be `path:line`, got `{element}`"))?;
            let m = crate::markup::build(&graph::load_with(root, opts));
            Ok(json!(m.matching_rules(path, line)))
        }
        "unused_selectors" => Ok(json!(
            crate::markup::build(&graph::load_with(root, opts)).unused_selectors()
        )),
        "selector_refs" => {
            let m = crate::markup::build(&graph::load_with(root, opts));
            Ok(json!(
                m.selector_refs(args.get("path").and_then(Value::as_str))
            ))
        }
        "pages" => Ok(json!(
            crate::markup::build(&graph::load_with(root, opts)).pages()
        )),
        "stylesheets" => Ok(json!(
            crate::markup::build(&graph::load_with(root, opts)).stylesheets()
        )),
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
