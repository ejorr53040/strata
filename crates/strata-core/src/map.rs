//! `strata map`: the whole repo as compact text, built so that reading it is enough to know
//! what exists, what it looks like, and what connects to what.
//!
//! One block per module (`== module path`), listing its definitions with signatures and doc
//! summaries, and under each definition the connections strata resolved: `>` what it calls,
//! `<` what calls it, then (higher levels) `uses`, `impl`, `types`, `fields`. Names are the
//! shortest suffix of the full id that is unique in the repo, so lines stay short and stay
//! unambiguous. A token budget picks the richest level that fits.

use crate::graph::{self, EdgeKind, EdgeState};
use crate::lang::{self, DefKind};
use crate::walk::{self, WalkOptions};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

pub struct Map {
    pub text: String,
    pub tokens: usize,
    pub level: u8,
}

/// Rough token count: about 3.5 characters per token for code and identifiers.
pub fn estimate_tokens(s: &str) -> usize {
    (s.len() * 2).div_ceil(7)
}

/// What to include: richest first. Each step drops detail: first the long lists, then docs,
/// then signatures shrink, then the connections go entirely.
const STEPS: &[(u8, usize, bool)] = &[
    (3, 200, true),
    (2, 160, true),
    (2, 100, false),
    (1, 100, false),
    (0, 80, false),
];
/// Longest connection list (calls, uses, ...) before it is cut with a `+n` note.
const LIST_CAP: usize = 25;
/// Callers and module dependencies are shorter: they are the inverse of lists shown elsewhere.
const INVERSE_CAP: usize = 12;
/// A candidate set (`a|b|c`) of more than this many targets is cut with a `+n` note.
const ALT_CAP: usize = 4;

struct Item {
    id: String,
    kind: DefKind,
    line: usize,
    depth: usize,
    sig: String,
    doc: Option<String>,
}

struct ModuleBlock {
    id: String,
    path: String,
    doc: Option<String>,
    items: Vec<Item>,
}

/// Build the map. `level` fixes the detail; otherwise `budget` (tokens) picks it; with
/// neither, the richest level is used.
pub fn build(root: &Path, opts: &WalkOptions, level: Option<u8>, budget: Option<usize>) -> Map {
    let w = walk::walk(root, opts);
    let sources: Vec<graph::Source> = w
        .indexed
        .iter()
        .map(|f| graph::Source {
            path: f.rel.clone(),
            text: f.text.clone(),
        })
        .collect();
    let graph = graph::build(&sources, &[]);
    let blocks = blocks(&sources);
    let ctx = Ctx::new(&blocks, &graph);

    let steps: Vec<(u8, usize, bool)> = match level {
        Some(l) => vec![(l.min(3), 200, true)],
        None => STEPS.to_vec(),
    };
    let mut last = None;
    for (lvl, sig, docs) in steps {
        let map = ctx.render(&blocks, lvl, sig, docs, graph.edges.len());
        let fits = budget.is_none_or(|b| map.tokens <= b);
        last = Some(map);
        if fits {
            break;
        }
    }
    last.expect("at least one step")
}

fn blocks(sources: &[graph::Source]) -> Vec<ModuleBlock> {
    let mut out = Vec::new();
    for s in sources {
        let Some(language) = lang::for_path(&s.path) else {
            continue;
        };
        let module = language.module_name(&s.path);
        let facts = language.extract(&s.path, &s.text);
        let desc = language.describe(&s.path, &s.text);
        let mut by_key: HashMap<(usize, &str), &lang::Described> = HashMap::new();
        for d in &desc {
            by_key.entry((d.line, d.name.as_str())).or_insert(d);
        }
        let mut items: Vec<Item> = Vec::new();
        let mut seen = BTreeSet::new();
        for d in &facts.defs {
            let id = if module.is_empty() {
                d.qualname.clone()
            } else {
                format!("{module}.{}", d.qualname)
            };
            // A field and a method may share an id (`self.level` / `self.level()`): keep both.
            if !seen.insert((id.clone(), d.kind == DefKind::Field)) {
                continue;
            }
            let name = d.qualname.rsplit('.').next().unwrap_or(&d.qualname);
            let described = by_key.get(&(d.line, name));
            items.push(Item {
                id,
                kind: d.kind,
                line: d.line,
                depth: d.qualname.matches('.').count(),
                sig: described.map(|x| x.sig.clone()).unwrap_or_default(),
                doc: described.and_then(|x| x.doc.clone()),
            });
        }
        items.sort_by_key(|i| i.line);
        out.push(ModuleBlock {
            id: module,
            path: s.path.clone(),
            doc: desc
                .iter()
                .find(|d| d.name == "<module>")
                .and_then(|d| d.doc.clone()),
            items,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

struct Ctx<'a> {
    graph: &'a graph::Graph,
    /// id -> shortest unique suffix
    short: HashMap<String, String>,
    modules: BTreeSet<String>,
}

impl<'a> Ctx<'a> {
    fn new(blocks: &[ModuleBlock], graph: &'a graph::Graph) -> Self {
        let mut kinds = HashMap::new();
        let mut all: BTreeSet<String> = BTreeSet::new();
        for b in blocks {
            all.insert(b.id.clone());
            for i in &b.items {
                kinds.insert(i.id.clone(), i.kind);
                all.insert(i.id.clone());
            }
        }
        let ids: Vec<String> = all.into_iter().collect();
        // How many ids end in each dotted suffix.
        let mut count: HashMap<String, usize> = HashMap::new();
        for id in &ids {
            let segs: Vec<&str> = id.split('.').collect();
            for k in 1..=segs.len() {
                *count.entry(segs[segs.len() - k..].join(".")).or_default() += 1;
            }
        }
        let mut short = HashMap::new();
        for id in &ids {
            let segs: Vec<&str> = id.split('.').collect();
            let min = match kinds.get(id) {
                Some(DefKind::Method | DefKind::Field | DefKind::Variant) => 2.min(segs.len()),
                _ => 1,
            };
            let pick = (min..=segs.len())
                .map(|k| segs[segs.len() - k..].join("."))
                .find(|s| count.get(s) == Some(&1))
                .unwrap_or_else(|| id.clone());
            short.insert(id.clone(), pick);
        }
        Ctx {
            graph,
            short,
            modules: blocks.iter().map(|b| b.id.clone()).collect(),
        }
    }

    fn name(&self, id: &str) -> String {
        self.short
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    /// The module an id lives in: its longest module prefix.
    fn module_of<'b>(&'b self, id: &'b str) -> Option<&'b str> {
        let mut cur = id;
        loop {
            if self.modules.contains(cur) {
                return Some(cur);
            }
            cur = cur.rsplit_once('.')?.0;
        }
    }

    /// Render at one level of detail.
    fn render(
        &self,
        blocks: &[ModuleBlock],
        level: u8,
        sig_chars: usize,
        docs: bool,
        edges: usize,
    ) -> Map {
        // Group connections by the item they start from and the item they reach.
        let mut calls: HashMap<&str, BTreeSet<String>> = HashMap::new();
        let mut callers: HashMap<&str, BTreeSet<String>> = HashMap::new();
        let mut uses: HashMap<&str, BTreeSet<String>> = HashMap::new();
        let mut impls: HashMap<&str, BTreeSet<String>> = HashMap::new();
        let mut types: HashMap<&str, BTreeSet<String>> = HashMap::new();
        let mut fields: HashMap<&str, BTreeSet<String>> = HashMap::new();
        let mut deps: HashMap<&str, BTreeSet<String>> = HashMap::new();

        for e in &self.graph.edges {
            let internal = matches!(e.state, EdgeState::Resolved | EdgeState::Candidate)
                && !e.targets.is_empty();
            let from_mod = self.module_of(&e.caller);
            if internal {
                if let Some(fm) = from_mod {
                    for t in &e.targets {
                        if let Some(tm) = self.module_of(t) {
                            if tm != fm {
                                deps.entry(fm).or_default().insert(tm.to_string());
                            }
                        }
                    }
                }
            }
            let one_of = || {
                let mut names: Vec<String> = e
                    .targets
                    .iter()
                    .take(ALT_CAP)
                    .map(|t| self.name(t))
                    .collect();
                if e.targets.len() > ALT_CAP {
                    names.push(format!("+{}", e.targets.len() - ALT_CAP));
                }
                names.join("|")
            };
            match (e.kind, internal) {
                (EdgeKind::Call, true) => {
                    calls.entry(&e.caller).or_default().insert(one_of());
                    let who = self.name(&e.caller);
                    for t in &e.targets {
                        callers.entry(t).or_default().insert(who.clone());
                    }
                }
                (EdgeKind::Value | EdgeKind::Variant, true) => {
                    uses.entry(&e.caller).or_default().insert(one_of());
                }
                (EdgeKind::Impl, true) => {
                    impls.entry(&e.caller).or_default().insert(one_of());
                }
                (EdgeKind::Impl, false) => {
                    impls
                        .entry(&e.caller)
                        .or_default()
                        .insert(format!("ext:{}", e.callee));
                }
                (EdgeKind::Type, true) => {
                    types.entry(&e.caller).or_default().insert(one_of());
                }
                (EdgeKind::Field, true) => {
                    fields.entry(&e.caller).or_default().insert(one_of());
                }
                _ => {}
            }
        }

        let list = |prefix: &str,
                    set: Option<&BTreeSet<String>>,
                    cap: usize,
                    indent: &str,
                    out: &mut String| {
            let Some(set) = set.filter(|s| !s.is_empty()) else {
                return;
            };
            let shown: Vec<&str> = set.iter().take(cap).map(String::as_str).collect();
            out.push_str(indent);
            out.push_str(prefix);
            out.push(' ');
            out.push_str(&shown.join(", "));
            if set.len() > LIST_CAP {
                out.push_str(&format!(" … +{}", set.len() - LIST_CAP));
            }
            out.push('\n');
        };

        let mut body = String::new();
        let mut symbols = 0;
        for b in blocks {
            body.push_str(&format!("== {} {}", b.id, b.path));
            if let (true, Some(d)) = (docs, &b.doc) {
                body.push_str(&format!(" — {d}"));
            }
            body.push('\n');
            if level >= 1 {
                list("deps:", deps.get(b.id.as_str()), INVERSE_CAP, "", &mut body);
            }
            for i in &b.items {
                symbols += 1;
                let indent = "  ".repeat(i.depth);
                body.push_str(&format!(
                    "{indent}{} {} :{}",
                    kind_word(i.kind),
                    self.name(&i.id),
                    i.line
                ));
                if !i.sig.is_empty() {
                    body.push(' ');
                    body.push_str(&clip(&i.sig, sig_chars));
                }
                if let (true, Some(d)) = (docs, &i.doc) {
                    body.push_str(&format!(" — {d}"));
                }
                body.push('\n');
                if level == 0 || i.kind == DefKind::Field {
                    continue;
                }
                let inner = format!("{indent}  ");
                let id = i.id.as_str();
                list(">", calls.get(id), LIST_CAP, &inner, &mut body);
                list("<", callers.get(id), INVERSE_CAP, &inner, &mut body);
                if level >= 2 {
                    list("uses:", uses.get(id), LIST_CAP, &inner, &mut body);
                    list("impl:", impls.get(id), LIST_CAP, &inner, &mut body);
                }
                if level >= 3 {
                    list("types:", types.get(id), LIST_CAP, &inner, &mut body);
                    list("fields:", fields.get(id), LIST_CAP, &inner, &mut body);
                }
            }
        }
        let header = |tokens: usize| {
            format!(
                "# strata map | modules {} symbols {} edges {} | level {} | ~{} tokens\n",
                blocks.len(),
                symbols,
                edges,
                level,
                tokens
            )
        };
        let tokens = estimate_tokens(&body) + estimate_tokens(&header(0)) + 3;
        let text = format!("{}{}", header(tokens), body);
        Map {
            tokens: estimate_tokens(&text),
            text,
            level,
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn kind_word(k: DefKind) -> &'static str {
    match k {
        DefKind::Method | DefKind::Function => "fn",
        other => other.label(),
    }
}

/// The map as a JSON-friendly summary for the agent tool.
pub fn summary(m: &Map) -> BTreeMap<&'static str, serde_json::Value> {
    BTreeMap::from([
        ("level", serde_json::json!(m.level)),
        ("tokens", serde_json::json!(m.tokens)),
        ("text", serde_json::json!(m.text)),
    ])
}
