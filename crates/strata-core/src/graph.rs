use crate::lang::{self, Call, DefKind, FileFacts, Language, TypeHint};
use std::collections::BTreeMap;
use std::path::Path;

/// How confidently a call site was tied to its target(s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeState {
    /// Exactly one known target.
    Resolved,
    /// Target lives outside the indexed repo; `evidence` names the import.
    External,
    /// Several possible targets; `targets` lists every one, none is guessed.
    Candidate,
    /// Could not be tied to anything; `reason` says why.
    Unresolved,
    /// Target lives in code that was skipped by indexing.
    Unindexed,
    /// Seen at runtime by the tracer (not produced by static analysis).
    Observed,
}

impl EdgeState {
    pub fn label(self) -> &'static str {
        match self {
            EdgeState::Resolved => "resolved",
            EdgeState::External => "external",
            EdgeState::Candidate => "candidate",
            EdgeState::Unresolved => "unresolved",
            EdgeState::Unindexed => "unindexed",
            EdgeState::Observed => "observed",
        }
    }
}

/// What kind of connection an edge records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    /// A call site.
    Call,
    /// A type named in a signature, field, binding, impl header or expression.
    Type,
    /// A function, const or static used as a value (not called).
    Value,
    /// A struct field access or initialiser.
    Field,
    /// An enum variant named in an expression or pattern.
    Variant,
    /// `impl Trait for Type`: one edge to the trait and one to the type.
    Impl,
    /// A `use` declaration binding a name.
    Import,
    /// A module (or crate) named as a prefix of a longer path.
    Path,
}

impl EdgeKind {
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Call => "call",
            EdgeKind::Type => "type",
            EdgeKind::Value => "value",
            EdgeKind::Field => "field",
            EdgeKind::Variant => "variant",
            EdgeKind::Impl => "impl",
            EdgeKind::Import => "import",
            EdgeKind::Path => "path",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub caller: String,
    /// The callee expression as written at the call site.
    pub callee: String,
    pub state: EdgeState,
    pub targets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub path: String,
    pub line: usize,
}

impl Edge {
    /// One-fact-per-line rendering used by the CLI.
    pub fn to_line(&self) -> String {
        let what = if self.targets.is_empty() {
            self.callee.clone()
        } else {
            self.targets.join(",")
        };
        let detail = match (&self.evidence, &self.reason) {
            (Some(e), _) => format!(" ({e})"),
            (_, Some(r)) => format!(" ({r})"),
            _ => String::new(),
        };
        let state = match self.kind {
            EdgeKind::Call => self.state.label().to_string(),
            k => format!("{}[{}]", self.state.label(), k.label()),
        };
        format!(
            "{} {} -> {}{} {}:{}",
            state, self.caller, what, detail, self.path, self.line
        )
    }
}

pub struct Source {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub edges: Vec<Edge>,
}

/// Everything the resolver knows about the indexed repo.
struct Index<'a> {
    /// module name -> (language, facts, repo-relative path)
    modules: BTreeMap<String, (&'static dyn Language, FileFacts, String)>,
    /// fully qualified id -> definition
    symbols: BTreeMap<String, Sym>,
    /// Ids of struct/variant fields. Kept apart from `symbols` because a field and a method
    /// may share a name (`self.level` and `self.level()`).
    fields: BTreeMap<String, Sym>,
    /// (function id, name) -> what a parameter or `let` is known to be.
    bindings: BTreeMap<(String, String), (String, TypeHint)>,
    /// class id -> indexed base class ids
    bases: BTreeMap<String, Vec<String>>,
    /// class id -> indexed direct subclass ids
    subclasses: BTreeMap<String, Vec<String>>,
    unindexed: &'a [String],
}

struct Sym {
    kind: DefKind,
    /// Declared type (field/const) or return type (fn), as written in `module`.
    ty: Option<String>,
    module: String,
    /// Qualname inside `module`.
    qual: String,
}

/// What a name means in a module's scope.
enum Lookup {
    Def(String),
    Module(String),
    External(String),
    Unindexed(String),
    NotInModule(String, String),
    Missing,
}

enum Outcome {
    Resolved(Vec<String>),
    Candidate(Vec<String>),
    External(String),
    Unindexed(String),
    Unresolved(String),
}

pub fn build(sources: &[Source], unindexed_modules: &[String]) -> Graph {
    let mut modules = BTreeMap::new();
    for s in sources {
        if let Some(lang) = lang::for_path(&s.path) {
            let facts = lang.extract(&s.path, &s.text);
            modules.insert(lang.module_name(&s.path), (lang, facts, s.path.clone()));
        }
    }
    let mut symbols = BTreeMap::new();
    let mut fields = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    for (module, (_, facts, _)) in &modules {
        for d in &facts.defs {
            let id = qualify(module, &d.qualname);
            let sym = || Sym {
                kind: d.kind,
                ty: d.ty.clone(),
                module: module.clone(),
                qual: d.qualname.clone(),
            };
            if d.kind == DefKind::Field {
                fields.insert(id.clone(), sym());
                symbols.entry(id).or_insert_with(sym);
            } else {
                symbols.insert(id, sym());
            }
        }
        for b in &facts.bindings {
            bindings.insert(
                (qualify(module, &b.enclosing), b.name.clone()),
                (module.clone(), b.hint.clone()),
            );
        }
    }
    let mut index = Index {
        modules,
        symbols,
        fields,
        bindings,
        bases: BTreeMap::new(),
        subclasses: BTreeMap::new(),
        unindexed: unindexed_modules,
    };
    index.register_foreign_impls();
    index.link_classes();
    let mut edges = Vec::new();
    for (module, (_, facts, path)) in &index.modules {
        for call in &facts.calls {
            if call.local && matches!(index.lookup(module, &call.callee), Lookup::Missing) {
                continue; // a closure or fn-pointer call: no function of that name is in scope
            }
            edges.push(index.edge(module, path, call));
        }
        for r in &facts.refs {
            edges.extend(index.ref_edge(module, path, r));
        }
    }
    edges.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
    Graph { edges }
}

impl Index<'_> {
    fn edge(&self, module: &str, path: &str, call: &Call) -> Edge {
        let caller = match &call.enclosing {
            Some(q) => qualify(module, q),
            None => qualify(module, "<module>"),
        };
        let mut edge = Edge {
            kind: EdgeKind::Call,
            caller,
            callee: call.callee.clone(),
            state: EdgeState::Unresolved,
            targets: Vec::new(),
            evidence: None,
            reason: None,
            path: path.to_string(),
            line: call.line,
        };
        match self.resolve_d(
            module,
            call.enclosing.as_deref(),
            &call.callee,
            call.recv_local,
            0,
        ) {
            Outcome::Resolved(t) => {
                edge.state = EdgeState::Resolved;
                edge.targets = t;
            }
            Outcome::Candidate(t) => {
                edge.state = EdgeState::Candidate;
                edge.targets = t;
            }
            Outcome::External(ev) => {
                edge.state = EdgeState::External;
                edge.evidence = Some(ev);
            }
            Outcome::Unindexed(ev) => {
                edge.state = EdgeState::Unindexed;
                edge.evidence = Some(ev);
            }
            Outcome::Unresolved(why) => edge.reason = Some(why),
        }
        edge
    }

    fn is_unindexed(&self, module: &str) -> bool {
        self.unindexed
            .iter()
            .any(|u| module == u || module.starts_with(&format!("{u}.")))
    }

    fn module_ref(&self, module: &str, evidence: String) -> Lookup {
        if self.modules.contains_key(module) {
            Lookup::Module(module.to_string())
        } else if self.is_unindexed(module) {
            Lookup::Unindexed(evidence)
        } else {
            Lookup::External(evidence)
        }
    }

    fn lookup(&self, module: &str, name: &str) -> Lookup {
        self.lookup_in(module, name, 0)
    }

    fn lookup_in(&self, module: &str, name: &str, depth: usize) -> Lookup {
        let (lang, facts, _) = &self.modules[module];
        let own = qualify(module, name);
        // An inline module (or a `#[path]` alias) is a module, not a type: an import of the
        // same name wins, else it is reached as a module.
        let is_module_def = self
            .symbols
            .get(&own)
            .is_some_and(|s| s.kind == DefKind::Module);
        if !name.contains('.') && self.symbols.contains_key(&own) && !is_module_def {
            return Lookup::Def(own);
        }
        if let Some(imp) = facts.imports.iter().find(|i| i.local == name) {
            return match &imp.name {
                None => self.module_ref(&imp.module, format!("import {}", imp.module)),
                Some(n) => {
                    let full = qualify(&imp.module, n);
                    if self.symbols.contains_key(&full) {
                        Lookup::Def(full)
                    } else if self.modules.contains_key(&full) {
                        Lookup::Module(full)
                    } else if self.modules.contains_key(&imp.module) {
                        self.follow_reexport(&imp.module, n, 0)
                    } else if self.is_unindexed(&imp.module) {
                        Lookup::Unindexed(format!("from {} import {n}", imp.module))
                    } else {
                        Lookup::External(format!("from {} import {n}", imp.module))
                    }
                }
            };
        }
        // `import a.b` also binds the top-level name `a`.
        if let Some(imp) = facts.imports.iter().find(|i| {
            i.name.is_none() && i.local == i.module && i.local.split('.').next() == Some(name)
        }) {
            return self.module_ref(name, format!("import {}", imp.module));
        }
        if !name.contains('.') && is_module_def {
            return Lookup::Module(own);
        }
        // `use m::*`: names the glob module defines.
        for imp in facts.imports.iter().filter(|i| i.local == "*") {
            let full = qualify(&imp.module, name);
            if self.symbols.contains_key(&full) {
                return Lookup::Def(full);
            }
            if self.modules.contains_key(&full) {
                return Lookup::Module(full);
            }
            // A glob also brings in what that module itself imports (`use super::*`).
            if depth < 4 && self.modules.contains_key(&imp.module) {
                let l = self.lookup_in(&imp.module, name, depth + 1);
                if !matches!(l, Lookup::Missing) {
                    return l;
                }
            }
        }
        if lang.is_builtin(name) {
            return Lookup::External("builtin".into());
        }
        Lookup::Missing
    }

    /// `from m import n` where `m` itself imports `n` (`pub use`, `__init__` re-exports).
    fn follow_reexport(&self, module: &str, name: &str, depth: usize) -> Lookup {
        let not_found = || Lookup::NotInModule(module.to_string(), name.to_string());
        if depth > 8 {
            return not_found();
        }
        match self.lookup(module, name) {
            Lookup::Missing => not_found(),
            // Importing from a module that imports from itself must not loop.
            Lookup::NotInModule(..) => not_found(),
            other => other,
        }
    }

    /// Resolve each class's base expressions in its module's scope, then invert to subclasses.
    fn link_classes(&mut self) {
        let mut bases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (module, (_, facts, _)) in &self.modules {
            for d in facts.defs.iter().filter(|d| d.kind.is_type()) {
                let resolved: Vec<String> = d
                    .bases
                    .iter()
                    .filter_map(|b| match self.resolve(module, None, b) {
                        Outcome::Resolved(t) => t.into_iter().find(|id| self.is_class(id)),
                        _ => None,
                    })
                    .collect();
                bases.insert(qualify(module, &d.qualname), resolved);
            }
        }
        let mut subclasses: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (class, bs) in &bases {
            for b in bs {
                subclasses.entry(b.clone()).or_default().push(class.clone());
            }
        }
        self.bases = bases;
        self.subclasses = subclasses;
    }

    fn is_class(&self, id: &str) -> bool {
        self.symbols.get(id).is_some_and(|s| s.kind.is_type())
    }

    /// The method `name` as seen from `class`: its own, else the nearest base's.
    fn find_method(&self, class: &str, name: &str) -> Option<String> {
        let mut seen = Vec::new();
        self.find_method_in(class, name, &mut seen)
    }

    fn find_method_in(&self, class: &str, name: &str, seen: &mut Vec<String>) -> Option<String> {
        self.find_in(class, name, seen, true)
    }

    /// Follow `a.b.c` through members: `Enum.Variant.field`.
    fn walk_members(&self, id: &str, path: &[&str]) -> Option<String> {
        path.iter()
            .try_fold(id.to_string(), |cur, seg| self.find_member(&cur, seg))
    }

    /// A field, variant, constant or method of `class` (or its bases) by name.
    fn find_member(&self, class: &str, name: &str) -> Option<String> {
        self.find_in(class, name, &mut Vec::new(), false)
    }

    fn find_in(
        &self,
        class: &str,
        name: &str,
        seen: &mut Vec<String>,
        callable: bool,
    ) -> Option<String> {
        if seen.iter().any(|s| s == class) {
            return None;
        }
        seen.push(class.to_string());
        let id = format!("{class}.{name}");
        let ok = self
            .symbols
            .get(&id)
            .is_some_and(|s| !callable || matches!(s.kind, DefKind::Method | DefKind::Function));
        if ok {
            return Some(id);
        }
        let bases = self.bases.get(class)?;
        bases
            .iter()
            .find_map(|b| self.find_in(b, name, seen, callable))
    }

    /// Every subclass of `class`, transitively.
    fn descendants(&self, class: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut stack = vec![class.to_string()];
        while let Some(c) = stack.pop() {
            for sub in self.subclasses.get(&c).into_iter().flatten() {
                if !out.contains(sub) {
                    out.push(sub.clone());
                    stack.push(sub.clone());
                }
            }
        }
        out
    }

    /// `self.name()` inside `class`: the inherited definition plus every override below it.
    fn dispatch(&self, lang: &dyn Language, class: &str, name: &str) -> Outcome {
        let mut targets: Vec<String> = self.find_method(class, name).into_iter().collect();
        for d in self.descendants(class) {
            let id = format!("{d}.{name}");
            if self
                .symbols
                .get(&id)
                .is_some_and(|s| matches!(s.kind, DefKind::Method | DefKind::Function))
            {
                targets.push(id);
            }
        }
        targets.sort();
        targets.dedup();
        match targets.len() {
            0 if lang.unindexed_members_are_external() => Outcome::External(format!(
                "no indexed method `{name}` on `{class}`; derived or trait method"
            )),
            0 => Outcome::Unresolved(format!(
                "no indexed method `{name}` on `{class}` or its bases"
            )),
            1 => Outcome::Resolved(targets),
            _ => Outcome::Candidate(targets),
        }
    }

    /// The class lexically enclosing a function qualname, if any.
    fn enclosing_class(&self, module: &str, enclosing: &str) -> Option<String> {
        let mut q = enclosing;
        while let Some((parent, _)) = q.rsplit_once('.') {
            let id = qualify(module, parent);
            if self.is_class(&id) {
                return Some(id);
            }
            q = parent;
        }
        None
    }

    fn resolve(&self, module: &str, enclosing: Option<&str>, callee: &str) -> Outcome {
        self.resolve_d(module, enclosing, callee, false, 0)
    }

    fn resolve_d(
        &self,
        module: &str,
        enclosing: Option<&str>,
        callee: &str,
        recv_local: bool,
        depth: usize,
    ) -> Outcome {
        let lang = self.modules[module].0;
        if !callee
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return Outcome::Unresolved("callee is a dynamic expression".into());
        }
        let segs: Vec<&str> = callee.split('.').collect();
        if let (["self" | "cls", name], Some(enc)) = (segs.as_slice(), enclosing) {
            if let Some(class) = self.enclosing_class(module, enc) {
                return self.dispatch(lang, &class, name);
            }
        }
        // `x.a.m()`: when the receiver's type is known the call is to that type's `m`.
        if segs.len() >= 2 {
            let recv = &segs[..segs.len() - 1];
            if let Some(class) = self.chain_type(module, enclosing, recv, depth) {
                return self.dispatch(lang, &class, segs[segs.len() - 1]);
            }
        }
        // A local of unknown type: its name is not a module or item.
        let skip_lookup = recv_local && segs.len() >= 2;
        for k in (1..=segs.len()).rev() {
            if skip_lookup {
                break;
            }
            let l = self.lookup(module, &segs[..k].join("."));
            if !matches!(l, Lookup::Missing) {
                // `STATIC.method()`: a value of unknown type, not a path into the item.
                let is_value = matches!(&l, Lookup::Def(id) if self.symbols.get(id)
                    .is_some_and(|s| matches!(s.kind, DefKind::Const | DefKind::Static)));
                if is_value && k < segs.len() {
                    continue;
                }
                return self.resolve_from(lang, l, &segs[k..], false);
            }
        }
        if segs.len() == 1 {
            return Outcome::Unresolved(format!(
                "`{callee}` is not defined in module scope or imports"
            ));
        }
        // A fully written module path (`crate::util::f` is rewritten to `app.util.f`).
        for k in (1..segs.len()).rev() {
            if skip_lookup {
                break;
            }
            let m = segs[..k].join(".");
            if self.modules.contains_key(&m) {
                return self.resolve_in_module(&m, &segs[k..], false);
            }
        }
        // Receiver type is unknown: every indexed method with that name is a possible target.
        let name = segs[segs.len() - 1];
        let suffix = format!(".{name}");
        let targets: Vec<String> = self
            .symbols
            .iter()
            .filter(|(id, s)| s.kind == DefKind::Method && id.ends_with(&suffix))
            .map(|(id, _)| id.clone())
            .collect();
        if targets.is_empty() && lang.unindexed_members_are_external() {
            Outcome::External(format!("no indexed method `{name}`; receiver type unknown"))
        } else if targets.is_empty() {
            Outcome::Unresolved(format!(
                "receiver of `{callee}` has unknown type and no indexed method is named `{name}`"
            ))
        } else {
            Outcome::Candidate(targets)
        }
    }

    /// `impl Type { fn m }` in a module other than `Type`'s own: also register `m` under the
    /// type's id, so a call on a `Type` finds it.
    fn register_foreign_impls(&mut self) {
        let mut aliases = Vec::new();
        for (module, (_, facts, _)) in &self.modules {
            for d in &facts.defs {
                let Some(owner) = &d.owner else { continue };
                let at_home = self
                    .symbols
                    .get(&qualify(module, owner))
                    .is_some_and(|s| s.kind.is_type());
                if at_home {
                    continue;
                }
                let Some(class) = self.type_class(module, None, owner) else {
                    continue;
                };
                let name = d.qualname.rsplit('.').next().unwrap_or(&d.qualname);
                let sym = Sym {
                    kind: d.kind,
                    ty: d.ty.clone(),
                    module: module.clone(),
                    qual: d.qualname.clone(),
                };
                aliases.push((format!("{class}.{name}"), sym));
            }
        }
        for (id, sym) in aliases {
            self.symbols.entry(id).or_insert(sym);
        }
    }

    /// The class a type path names, as seen from `enclosing` in `module`.
    fn type_class(&self, module: &str, enclosing: Option<&str>, path: &str) -> Option<String> {
        match self.resolve_ref(module, enclosing, path, false)? {
            Outcome::Resolved(t) if t.len() == 1 && self.is_class(&t[0]) => Some(t[0].clone()),
            _ => None,
        }
    }

    /// The class a local binding is known to hold, from its parameter or `let` hint.
    fn binding_type(
        &self,
        module: &str,
        enclosing: &str,
        name: &str,
        depth: usize,
    ) -> Option<String> {
        let mut scope = enclosing;
        loop {
            if let Some((m, hint)) = self
                .bindings
                .get(&(qualify(module, scope), name.to_string()))
            {
                return self.hint_class(m, scope, hint, depth);
            }
            scope = scope.rsplit_once('.')?.0;
        }
    }

    fn hint_class(
        &self,
        module: &str,
        scope: &str,
        hint: &TypeHint,
        depth: usize,
    ) -> Option<String> {
        if depth > 6 {
            return None;
        }
        match hint {
            TypeHint::Declared(p) | TypeHint::Struct(p) => self.type_class(module, Some(scope), p),
            TypeHint::Call(callee) => {
                let fid = match self.resolve_d(module, Some(scope), callee, false, depth + 1) {
                    Outcome::Resolved(t) if t.len() == 1 => t[0].clone(),
                    _ => return None,
                };
                let sym = self.symbols.get(&fid)?;
                let ret = sym.ty.as_deref()?;
                self.type_class(&sym.module, Some(&sym.qual), ret)
            }
        }
    }

    /// The class of a receiver chain `x.a.b`: `x` is `self` or a typed local, each further
    /// segment a field with a declared type.
    fn chain_type(
        &self,
        module: &str,
        enclosing: Option<&str>,
        segs: &[&str],
        depth: usize,
    ) -> Option<String> {
        let enclosing = enclosing?;
        let mut class = if segs[0] == "self" {
            self.class_of(module, enclosing)?
        } else {
            self.binding_type(module, enclosing, segs[0], depth)?
        };
        for seg in &segs[1..] {
            let id = self.find_member(&class, seg)?;
            let field = self.fields.get(&id)?;
            class = self.type_class(&field.module, Some(&field.qual), field.ty.as_deref()?)?;
        }
        Some(class)
    }

    /// An item defined inside an enclosing function or inline module (`static N` in `fn f`,
    /// `const K` in `mod tests`), found by name.
    fn local_def(&self, module: &str, enclosing: &str, name: &str) -> Option<String> {
        let mut scope = enclosing;
        loop {
            let is_scope = self.symbols.get(&qualify(module, scope)).is_some_and(|s| {
                matches!(
                    s.kind,
                    DefKind::Function | DefKind::Method | DefKind::Module
                )
            });
            if is_scope {
                let id = qualify(module, &format!("{scope}.{name}"));
                if self.symbols.contains_key(&id) {
                    return Some(id);
                }
            }
            scope = scope.rsplit_once('.')?.0;
        }
    }

    /// The type an item's qualname sits in or is: `Circle.new` and `Circle` both give `Circle`.
    fn class_of(&self, module: &str, enclosing: &str) -> Option<String> {
        let own = qualify(module, enclosing);
        if self.is_class(&own) {
            return Some(own);
        }
        self.enclosing_class(module, enclosing)
    }

    /// Turn a non-call reference into an edge, or drop it: only connections that resolve,
    /// plus unresolved type/field uses (which flag gaps), are worth recording.
    fn ref_edge(&self, module: &str, path: &str, r: &lang::Ref) -> Option<Edge> {
        let caller = qualify(module, r.enclosing.as_deref().unwrap_or("<module>"));
        let outcome = self.resolve_ref(module, r.enclosing.as_deref(), &r.expr, r.recv_local)?;
        let (state, targets, reason) = match outcome {
            Outcome::Resolved(t) => (EdgeState::Resolved, t, None),
            Outcome::Candidate(t) => (EdgeState::Candidate, t, None),
            Outcome::Unresolved(why) if matches!(r.kind, EdgeKind::Type | EdgeKind::Field) => {
                (EdgeState::Unresolved, Vec::new(), Some(why))
            }
            // `impl Display for X` is worth keeping even though the trait is not ours.
            Outcome::External(ev) if r.kind == EdgeKind::Impl => {
                (EdgeState::External, Vec::new(), Some(ev))
            }
            _ => return None,
        };
        let (evidence, reason) = match state {
            EdgeState::External => (reason, None),
            _ => (None, reason),
        };
        let kind = match r.kind {
            // A `use` line stays an import whatever it imports.
            EdgeKind::Import | EdgeKind::Impl => r.kind,
            k => targets.first().map_or(k, |t| self.refine(k, t)),
        };
        if r.local
            && !matches!(kind, EdgeKind::Variant | EdgeKind::Type)
            && !self.is_const_target(&targets)
        {
            return None;
        }
        Some(Edge {
            kind,
            caller,
            callee: r.expr.clone(),
            state,
            targets,
            evidence,
            reason,
            path: path.to_string(),
            line: r.line,
        })
    }

    fn is_const_target(&self, targets: &[String]) -> bool {
        targets.first().is_some_and(|t| {
            self.symbols
                .get(t)
                .is_some_and(|s| matches!(s.kind, DefKind::Const | DefKind::Static))
        })
    }

    /// What a reference really is, from what it points at.
    fn refine(&self, initial: EdgeKind, target: &str) -> EdgeKind {
        if initial == EdgeKind::Field && self.fields.contains_key(target) {
            return EdgeKind::Field;
        }
        match self.symbols.get(target).map(|s| s.kind) {
            Some(DefKind::Module) => EdgeKind::Path,
            Some(k) if k.is_type() || k == DefKind::TypeAlias => EdgeKind::Type,
            Some(DefKind::Variant) => EdgeKind::Variant,
            Some(DefKind::Field) => EdgeKind::Field,
            Some(_) => EdgeKind::Value,
            None if self.modules.contains_key(target) => EdgeKind::Path,
            None => initial,
        }
    }

    fn resolve_ref(
        &self,
        module: &str,
        enclosing: Option<&str>,
        expr: &str,
        recv_local: bool,
    ) -> Option<Outcome> {
        let lang = self.modules[module].0;
        if self.modules.contains_key(expr) {
            return Some(Outcome::Resolved(vec![expr.to_string()]));
        }
        // `crate` of a binary: the crate name stands for the `main.rs` module.
        let main = format!("{expr}.main");
        if !expr.contains('.') && self.modules.contains_key(&main) {
            return Some(Outcome::Resolved(vec![main]));
        }
        let segs: Vec<&str> = expr.split('.').collect();
        if segs.len() >= 2 && (segs[0] == "self" || recv_local) {
            let recv = &segs[..segs.len() - 1];
            let name = segs[segs.len() - 1];
            if let Some(class) = self.chain_type(module, enclosing, recv, 0) {
                return Some(match self.find_member(&class, name) {
                    Some(m) => Outcome::Resolved(vec![m]),
                    None => self.unknown_member(lang, &class, name),
                });
            }
            if recv_local {
                return Some(self.by_member_name(lang, name));
            }
        }
        if segs[0] == "_" && segs.len() > 1 {
            // A receiver whose type we do not know.
            return Some(self.by_member_name(lang, segs[segs.len() - 1]));
        }
        if let Some(id) = enclosing.and_then(|e| self.local_def(module, e, segs[0])) {
            return Some(self.resolve_from(lang, Lookup::Def(id), &segs[1..], true));
        }
        if matches!(segs[0], "self" | "Self") {
            let class = enclosing.and_then(|e| self.class_of(module, e))?;
            return Some(match segs.as_slice() {
                [_] => Outcome::Resolved(vec![class]),
                [_, name] => match self.find_member(&class, name) {
                    Some(m) => Outcome::Resolved(vec![m]),
                    None => self.unknown_member(lang, &class, name),
                },
                [.., name] => self.by_member_name(lang, name),
                [] => return None,
            });
        }
        for k in (1..=segs.len()).rev() {
            match self.lookup(module, &segs[..k].join(".")) {
                Lookup::Missing => {}
                Lookup::Module(m) if k == segs.len() => return Some(Outcome::Resolved(vec![m])),
                l => return Some(self.resolve_from(lang, l, &segs[k..], true)),
            }
        }
        for k in (1..segs.len()).rev() {
            let m = segs[..k].join(".");
            if self.modules.contains_key(&m) {
                return Some(self.resolve_in_module(&m, &segs[k..], true));
            }
        }
        if segs.len() > 1 {
            // `recv.field` with a receiver whose type we do not know.
            return Some(self.by_member_name(lang, segs[segs.len() - 1]));
        }
        Some(Outcome::Unresolved(format!(
            "`{expr}` is not defined in module scope or imports"
        )))
    }

    /// `Type.name` where `Type` is indexed but has no member `name`.
    fn unknown_member(&self, lang: &dyn Language, class: &str, name: &str) -> Outcome {
        if lang.unindexed_members_are_external() {
            Outcome::External(format!(
                "no indexed member `{name}` on `{class}`; derived or trait item"
            ))
        } else {
            Outcome::Unresolved(format!("no indexed member `{name}` on `{class}`"))
        }
    }

    /// A member access on a receiver of unknown type: every indexed field or variant so named.
    fn by_member_name(&self, lang: &dyn Language, name: &str) -> Outcome {
        let suffix = format!(".{name}");
        let targets: Vec<String> = self
            .fields
            .keys()
            .filter(|id| id.ends_with(&suffix))
            .cloned()
            .collect();
        if !targets.is_empty() {
            Outcome::Candidate(targets)
        } else if lang.unindexed_members_are_external() {
            Outcome::External(format!("no indexed field `{name}`; receiver type unknown"))
        } else {
            Outcome::Unresolved(format!("no indexed field `{name}`"))
        }
    }

    fn resolve_from(&self, lang: &dyn Language, l: Lookup, rest: &[&str], any: bool) -> Outcome {
        match l {
            Lookup::Def(id) if rest.is_empty() => Outcome::Resolved(vec![id]),
            Lookup::Def(id) => {
                // `Class.method(...)` names its target statically, so inheritance applies but overrides don't.
                let found = match rest {
                    rest if any && self.is_class(&id) => self.walk_members(&id, rest),
                    [name] if self.is_class(&id) => self.find_method(&id, name),
                    _ => None,
                };
                match found {
                    Some(m) => Outcome::Resolved(vec![m]),
                    None if lang.unindexed_members_are_external() && self.is_class(&id) => {
                        Outcome::External(format!(
                            "`{id}` has no indexed member `{}`; derived or trait method",
                            rest.join(".")
                        ))
                    }
                    None => Outcome::Unresolved(format!(
                        "`{}` has no indexed member `{}`",
                        id,
                        rest.join(".")
                    )),
                }
            }
            Lookup::Module(m) => self.resolve_in_module(&m, rest, any),
            Lookup::External(ev) => Outcome::External(ev),
            Lookup::Unindexed(ev) => Outcome::Unindexed(ev),
            Lookup::NotInModule(m, n) => {
                Outcome::Unresolved(format!("`{n}` is not defined in module `{m}`"))
            }
            Lookup::Missing => unreachable!("callers filter Missing"),
        }
    }

    fn resolve_in_module(&self, module: &str, rest: &[&str], any: bool) -> Outcome {
        if rest.is_empty() {
            return Outcome::Unresolved(format!("module `{module}` is not callable"));
        }
        let full = qualify(module, &rest.join("."));
        if self.symbols.contains_key(&full) {
            return Outcome::Resolved(vec![full]);
        }
        let ty = qualify(module, rest[0]);
        if rest.len() > 1 && self.is_class(&ty) {
            // `module::Type::member`: the member lookup rules apply, derives included.
            return self.resolve_from(self.modules[module].0, Lookup::Def(ty), &rest[1..], any);
        }
        let sub = qualify(module, rest[0]);
        if rest.len() > 1 && self.modules.contains_key(&sub) {
            return self.resolve_in_module(&sub, &rest[1..], any);
        }
        // `pub use` re-exports: the module imports the name from elsewhere.
        if let Lookup::Def(id) = self.follow_reexport(module, rest[0], 0) {
            if let Some(t) = self.walk_members(&id, &rest[1..]) {
                return Outcome::Resolved(vec![t]);
            }
        }
        Outcome::Unresolved(format!(
            "`{}` is not defined in module `{module}`",
            rest.join(".")
        ))
    }
}

fn qualify(module: &str, qualname: &str) -> String {
    if module.is_empty() {
        qualname.to_string()
    } else {
        format!("{module}.{qualname}")
    }
}

/// True if `id` is `sym` or ends with `.sym` (so `util.helper` finds `app.util.helper`).
fn names(id: &str, sym: &str) -> bool {
    id == sym || id.strip_suffix(sym).is_some_and(|p| p.ends_with('.'))
}

impl Graph {
    /// Call sites whose resolved or candidate targets include `sym`.
    pub fn callers(&self, sym: &str) -> Vec<&Edge> {
        self.references(sym)
            .into_iter()
            .filter(|e| e.kind == EdgeKind::Call)
            .collect()
    }

    /// Every connection of any kind whose resolved or candidate targets include `sym`.
    pub fn references(&self, sym: &str) -> Vec<&Edge> {
        self.edges
            .iter()
            .filter(|e| e.targets.iter().any(|t| names(t, sym)))
            .collect()
    }

    /// Every call site inside `sym`, in any state.
    pub fn callees(&self, sym: &str) -> Vec<&Edge> {
        self.edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Call && names(&e.caller, sym))
            .collect()
    }
}

/// Read every source file of a supported language under `root`.
pub fn load(root: &Path) -> Vec<Source> {
    load_with(root, &crate::walk::WalkOptions::default())
}

/// Read every file the walker indexes (honouring ignore rules and `index.include`).
pub fn load_with(root: &Path, opts: &crate::walk::WalkOptions) -> Vec<Source> {
    crate::walk::walk(root, opts)
        .indexed
        .into_iter()
        .map(|f| Source {
            path: f.rel,
            text: f.text,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(files: &[(&str, &str)]) -> Vec<String> {
        let sources: Vec<Source> = files
            .iter()
            .map(|(p, t)| Source {
                path: p.to_string(),
                text: t.to_string(),
            })
            .collect();
        build(&sources, &[])
            .edges
            .iter()
            .map(Edge::to_line)
            .collect()
    }

    const UTIL: (&str, &str) = (
        "app/util.py",
        "def helper():\n    return 1\n\n\nclass Tool:\n    def go(self):\n        return 1\n",
    );

    #[test]
    fn absolute_and_aliased_imports_resolve_and_unknown_modules_are_external() {
        let main = "\
import os
import app.util
import app.util as u
from app.util import helper as h
from app.util import Tool
import requests


def main():
    h()
    u.helper()
    app.util.helper()
    os.getcwd()
    requests.get(1)
    Tool.go(1)
    Tool()
";
        let got = lines(&[UTIL, ("app/__init__.py", ""), ("app/main.py", main)]);
        assert_eq!(
            got,
            vec![
                "resolved app.main.main -> app.util.helper app/main.py:10",
                "resolved app.main.main -> app.util.helper app/main.py:11",
                "resolved app.main.main -> app.util.helper app/main.py:12",
                "external app.main.main -> os.getcwd (import os) app/main.py:13",
                "external app.main.main -> requests.get (import requests) app/main.py:14",
                "resolved app.main.main -> app.util.Tool.go app/main.py:15",
                "resolved app.main.main -> app.util.Tool app/main.py:16",
            ]
        );
    }

    #[test]
    fn relative_imports_resolve_from_modules_and_packages() {
        let deep = "\
from .. import util
from ..util import helper
from . import sibling


def go():
    util.helper()
    helper()
    sibling.f()
";
        let got = lines(&[
            UTIL,
            ("app/sub/__init__.py", ""),
            ("app/sub/sibling.py", "def f():\n    return 1\n"),
            ("app/sub/deep.py", deep),
        ]);
        assert_eq!(
            got,
            vec![
                "resolved app.sub.deep.go -> app.util.helper app/sub/deep.py:7",
                "resolved app.sub.deep.go -> app.util.helper app/sub/deep.py:8",
                "resolved app.sub.deep.go -> app.sub.sibling.f app/sub/deep.py:9",
            ]
        );
    }

    #[test]
    fn self_calls_follow_inheritance_and_ambiguous_dispatch_lists_every_override() {
        let base = "\
class Base:
    def run(self):
        self.step()
        self.shared()

    def step(self):
        return 0

    def shared(self):
        return 0
";
        let sub = "\
from app.base import Base


class Sub(Base):
    def step(self):
        return 1


class Leaf(Sub):
    def go(self):
        self.shared()
        self.step()
        self.nope()
        Sub.shared(self)
";
        let got = lines(&[("app/base.py", base), ("app/sub.py", sub)]);
        assert_eq!(
            got,
            vec![
                "candidate app.base.Base.run -> app.base.Base.step,app.sub.Sub.step app/base.py:3",
                "resolved app.base.Base.run -> app.base.Base.shared app/base.py:4",
                "resolved app.sub.Leaf.go -> app.base.Base.shared app/sub.py:11",
                "resolved app.sub.Leaf.go -> app.sub.Sub.step app/sub.py:12",
                "unresolved app.sub.Leaf.go -> self.nope (no indexed method `nope` on `app.sub.Leaf` or its bases) app/sub.py:13",
                "resolved app.sub.Leaf.go -> app.base.Base.shared app/sub.py:14",
            ]
        );
    }

    #[test]
    fn unknown_receivers_yield_every_same_named_method_as_candidates() {
        let src = "\
class A:
    def run(self):
        return 1


class B:
    def run(self):
        return 2


def f(x):
    x.run()
    x.fly()
";
        assert_eq!(
            lines(&[("m.py", src)]),
            vec![
                "candidate m.f -> m.A.run,m.B.run m.py:12",
                "unresolved m.f -> x.fly (receiver of `x.fly` has unknown type and no indexed method is named `fly`) m.py:13",
            ]
        );
    }

    #[test]
    fn skipped_modules_give_unindexed_edges() {
        let sources = [Source {
            path: "m.py".into(),
            text: "from vendor.lib import thing\n\n\ndef f():\n    thing()\n".into(),
        }];
        let got: Vec<String> = build(&sources, &["vendor".to_string()])
            .edges
            .iter()
            .map(Edge::to_line)
            .collect();
        assert_eq!(
            got,
            vec!["unindexed m.f -> thing (from vendor.lib import thing) m.py:5"]
        );
    }

    #[test]
    fn observed_is_a_valid_state_in_the_output_format() {
        let edge = Edge {
            kind: EdgeKind::Call,
            caller: "m.f".into(),
            callee: "g".into(),
            state: EdgeState::Observed,
            targets: vec!["m.g".into()],
            evidence: None,
            reason: None,
            path: "m.py".into(),
            line: 3,
        };
        assert_eq!(edge.to_line(), "observed m.f -> m.g m.py:3");
        let json = serde_json::to_value(&edge).unwrap();
        assert_eq!(json["state"], "observed");
    }

    #[test]
    fn plain_calls_resolve_locally_or_are_external_builtin_or_unresolved() {
        let got = lines(&[(
            "app.py",
            "def helper():\n    return 1\n\n\ndef main():\n    helper()\n    print(1)\n    mystery()\n",
        )]);
        assert_eq!(
            got,
            vec![
                "resolved app.main -> app.helper app.py:6",
                "external app.main -> print (builtin) app.py:7",
                "unresolved app.main -> mystery (`mystery` is not defined in module scope or imports) app.py:8",
            ]
        );
    }
}
