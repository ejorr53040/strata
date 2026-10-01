//! Rust plugin. Items and calls are normalised into the dotted-name model the resolver uses:
//! module `crates/x-y/src/a/b.rs` is `x_y.a.b`, `impl T { fn m }` defines `T.m`, and a path
//! `crate::a::f` becomes `x_y.a.f`.

use super::{Call, Def, DefKind, FileFacts, Import, Language, Ref};
use crate::graph::EdgeKind;
use std::collections::HashSet;
use tree_sitter::{Node, Parser};

pub struct Rust;

impl Language for Rust {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn module_name(&self, rel_path: &str) -> String {
        let comps: Vec<&str> = rel_path.split('/').collect();
        let marker = comps
            .iter()
            .position(|c| matches!(*c, "src" | "tests" | "benches" | "examples"));
        let (krate, rest): (String, Vec<&str>) = match marker {
            Some(i) => {
                let krate = if i > 0 { comps[i - 1] } else { "crate" };
                // Only `src/` files live in the crate's own module tree.
                let from = if comps[i] == "src" { i + 1 } else { i };
                (krate.replace('-', "_"), comps[from..].to_vec())
            }
            None => ("crate".to_string(), comps),
        };
        let mut parts: Vec<String> = rest
            .iter()
            .map(|c| c.strip_suffix(".rs").unwrap_or(c).to_string())
            .collect();
        if parts.last().is_some_and(|l| l == "lib" || l == "mod") {
            parts.pop();
        }
        std::iter::once(krate)
            .chain(parts)
            .collect::<Vec<_>>()
            .join(".")
    }

    fn extract(&self, rel_path: &str, src: &str) -> FileFacts {
        let module = self.module_name(rel_path);
        let mut facts = FileFacts::default();
        let mut impls = Vec::new();
        let children = child_base(rel_path, &module);
        let _ = parse_into(
            src,
            Names {
                module: &module,
                children: &children,
            },
            Vec::new(),
            0,
            &[],
            &mut facts,
            &mut impls,
        );
        qualify_nested_calls(&mut facts, &module);
        for (ty, tr) in impls {
            if let Some(d) = facts.defs.iter_mut().find(|d| d.qualname == ty) {
                d.bases.push(tr);
            }
        }
        facts
    }

    fn is_builtin(&self, name: &str) -> bool {
        BUILTINS.contains(&name)
    }

    fn unindexed_members_are_external(&self) -> bool {
        true
    }
}

/// Prelude items and std crates: always in scope, never indexed.
const BUILTINS: &[&str] = &[
    "std", "core", "alloc", "Some", "None", "Ok", "Err", "Box", "String", "Vec", "Option",
    "Result", "Default", "From", "Into", "ToString", "Clone", "Iterator", "drop", "panic",
];

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Mod,
    Type,
    Func,
    /// Struct, enum, type alias: owns members but no methods.
    Item,
}

struct State<'a> {
    src: &'a [u8],
    module: &'a str,
    /// Base module for `mod x;` declarations (see [`child_base`]).
    children: &'a str,
    krate: &'a str,
    row_off: usize,
    scope: Vec<(String, Scope)>,
    /// Names bound by `let`, parameters and patterns, per enclosing function.
    locals: Vec<HashSet<String>>,
    /// Generic parameter names in scope (`T` in `fn f<T>`): never references to an item.
    generics: Vec<HashSet<String>>,
    /// `mod x;` declared in this file: `use x::f` means `use self::x::f`.
    declared_mods: HashSet<String>,
    facts: &'a mut FileFacts,
    impls: &'a mut Vec<(String, String)>,
}

/// Module that owns the files named by `mod x;` in this file. A crate root other than
/// `lib.rs` (`main.rs`, `tests/foo.rs`, ...) is named after its file, but its children sit
/// beside it, not under it.
fn child_base(rel_path: &str, module: &str) -> String {
    let comps: Vec<&str> = rel_path.split('/').collect();
    let file = comps.last().copied().unwrap_or_default();
    let direct_target =
        comps.len() >= 2 && matches!(comps[comps.len() - 2], "tests" | "benches" | "examples");
    let src_main = file == "main.rs" && comps.len() >= 2 && comps[comps.len() - 2] == "src";
    if src_main || direct_target {
        module.rsplit_once('.').map_or(module, |p| p.0).to_string()
    } else {
        module.to_string()
    }
}

/// The module a file defines and the module its `mod x;` declarations hang off.
#[derive(Clone, Copy)]
struct Names<'a> {
    module: &'a str,
    children: &'a str,
}

fn parse_into(
    src: &str,
    names: Names,
    scope: Vec<(String, Scope)>,
    row_off: usize,
    outer_locals: &[HashSet<String>],
    facts: &mut FileFacts,
    impls: &mut Vec<(String, String)>,
) -> bool {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let Some(tree) = parser.parse(src, None) else {
        return true;
    };
    let Names { module, children } = names;
    let krate = module.split('.').next().unwrap_or(module);
    let mut st = State {
        src: src.as_bytes(),
        module,
        children,
        krate,
        row_off,
        scope,
        locals: outer_locals.to_vec(),
        generics: Vec::new(),
        declared_mods: HashSet::new(),
        facts,
        impls,
    };
    st.declared_mods = declared_mods(tree.root_node(), src.as_bytes());
    st.walk(tree.root_node());
    tree.root_node().has_error()
}

fn declared_mods(root: Node, src: &[u8]) -> HashSet<String> {
    let mut c = root.walk();
    root.children(&mut c)
        .filter(|n| n.kind() == "mod_item" && n.child_by_field_name("body").is_none())
        .filter_map(|n| n.child_by_field_name("name")?.utf8_text(src).ok())
        .map(str::to_string)
        .collect()
}

impl State<'_> {
    fn text(&self, n: Node) -> String {
        n.utf8_text(self.src).unwrap_or_default().to_string()
    }

    fn line(&self, n: Node) -> usize {
        n.start_position().row + 1 + self.row_off
    }

    fn walk(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }

    fn visit(&mut self, child: Node) {
        match child.kind() {
            "function_item" | "function_signature_item" => self.function(child),
            "struct_item" => self.item(child, DefKind::Struct, Scope::Item),
            "enum_item" => self.item(child, DefKind::Enum, Scope::Item),
            "trait_item" => self.item(child, DefKind::Trait, Scope::Type),
            "type_item" => self.item(child, DefKind::TypeAlias, Scope::Item),
            "impl_item" => self.impl_block(child),
            "mod_item" => self.module_item(child),
            "use_declaration" => {
                if let Some(arg) = child.child_by_field_name("argument") {
                    self.use_tree(arg, Vec::new());
                }
            }
            "call_expression" => {
                self.call(child);
                self.call_rest(child);
            }
            "macro_invocation" => {
                if !self.define_macro_fn(child) {
                    self.macro_call(child);
                }
            }
            "const_item" => self.item(child, DefKind::Const, Scope::Item),
            "static_item" => self.item(child, DefKind::Static, Scope::Item),
            "identifier" => self.ident_ref(child),
            "field_declaration" => self.field_def(child),
            "enum_variant" => self.variant_def(child),
            "field_expression" => self.field_ref(child),
            "struct_expression" | "struct_pattern" => self.struct_fields(child),
            "attribute_item" | "inner_attribute_item" => self.attribute(child),
            "visibility_modifier" => self.visibility(child),
            "type_identifier" => self.type_ref(child),
            "scoped_type_identifier" => self.scoped_ref(child, EdgeKind::Type, true),
            "scoped_identifier" => self.scoped_ref(child, EdgeKind::Value, true),
            _ => self.walk(child),
        }
    }

    /// Visit a call's children except its callee, whose own references `call` recorded.
    fn call_rest(&mut self, node: Node) {
        let callee = node.child_by_field_name("function").map(|f| f.id());
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Some(child.id()) != callee {
                self.visit(child);
                continue;
            }
            // The receiver of `a.b.m()` is an expression that may hold references of its own.
            if child.kind() == "field_expression" {
                if let Some(recv) = child.child_by_field_name("value") {
                    self.visit(recv);
                }
            }
            if child.kind() == "generic_function" {
                if let Some(args) = child.child_by_field_name("type_arguments") {
                    self.walk(args);
                }
            }
        }
    }

    fn push_ref(&mut self, expr: String, line: usize, kind: EdgeKind) {
        self.push_ref_flagged(expr, line, kind, false);
    }

    fn push_ref_flagged(&mut self, expr: String, line: usize, kind: EdgeKind, local: bool) {
        let enclosing = self.scope.last().map(|s| s.0.clone());
        self.facts.refs.push(Ref {
            enclosing,
            expr,
            line,
            local,
            kind,
        });
    }

    /// A bare name in an expression: a function, constant or static if one is in scope.
    /// Locals and definition names are not references; names that resolve to nothing are
    /// dropped by the graph.
    fn ident_ref(&mut self, node: Node) {
        let is_def_name = node.parent().is_some_and(|p| {
            matches!(
                p.kind(),
                "function_item"
                    | "function_signature_item"
                    | "const_item"
                    | "static_item"
                    | "mod_item"
                    | "enum_variant"
                    | "union_item"
            ) && p
                .child_by_field_name("name")
                .is_some_and(|n| n.id() == node.id())
        });
        let name = self.text(node);
        if is_def_name {
            return;
        }
        if self.is_local(&name) {
            // Only an uppercase local can really be a variant or constant in a pattern.
            if name.starts_with(|c: char| c.is_uppercase()) {
                self.push_ref_flagged(name, self.line(node), EdgeKind::Value, true);
            }
            return;
        }
        self.push_ref(name, self.line(node), EdgeKind::Value);
    }

    fn field_def(&mut self, node: Node) {
        if let Some(name) = node.child_by_field_name("name") {
            let qualname = self.qualify(&self.text(name));
            self.facts.defs.push(Def {
                qualname,
                kind: DefKind::Field,
                line: self.line(name),
                bases: Vec::new(),
            });
        }
        self.walk(node);
    }

    fn variant_def(&mut self, node: Node) {
        let Some(name) = node.child_by_field_name("name") else {
            return self.walk(node);
        };
        let qualname = self.qualify(&self.text(name));
        self.facts.defs.push(Def {
            qualname: qualname.clone(),
            kind: DefKind::Variant,
            line: self.line(name),
            bases: Vec::new(),
        });
        self.scope.push((qualname, Scope::Item));
        self.walk(node);
        self.scope.pop();
    }

    /// `recv.field` outside a call: a field of the receiver's type. `self.` and indexed
    /// paths name their owner; anything else is a receiver of unknown type (`_`).
    fn field_ref(&mut self, node: Node) {
        let (Some(value), Some(field)) = (
            node.child_by_field_name("value"),
            node.child_by_field_name("field"),
        ) else {
            return self.walk(node);
        };
        if field.kind() == "field_identifier" {
            let name = self.text(field);
            let expr = if simple_receiver(value) && !self.is_local(&root_ident(self, value)) {
                let recv: String = self.text(value).split_whitespace().collect();
                format!("{recv}.{name}")
            } else {
                format!("_.{name}")
            };
            self.push_ref(expr, self.line(field), EdgeKind::Field);
        }
        self.visit(value);
    }

    /// `Type { a: 1, b }` and `Type { a, .. }`: a field reference per named field, written
    /// as `Type.field` (variants: `Enum.Variant.field`).
    fn struct_fields(&mut self, node: Node) {
        let ty = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("type"));
        let Some(ty) = ty else {
            return self.walk(node);
        };
        let ty_path = self.type_name(ty);
        self.visit(ty);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.id() != ty.id() {
                self.struct_entry(child, &ty_path);
            }
        }
    }

    fn struct_entry(&mut self, node: Node, ty_path: &str) {
        match node.kind() {
            "field_initializer_list" => {
                let mut c = node.walk();
                for e in node.named_children(&mut c) {
                    self.struct_entry(e, ty_path);
                }
            }
            "field_initializer" | "field_pattern" => {
                let name = node
                    .child_by_field_name("field")
                    .or_else(|| node.child_by_field_name("name"));
                if let Some(n) = name {
                    self.push_ref(
                        format!("{ty_path}.{}", self.text(n)),
                        self.line(n),
                        EdgeKind::Field,
                    );
                }
                let mut c = node.walk();
                for part in node.children(&mut c) {
                    if Some(part.id()) != name.map(|n| n.id()) {
                        self.visit(part);
                    }
                }
            }
            // `Type { a }`: the identifier names the field (and reads a local of that name).
            "shorthand_field_initializer" => {
                if let Some(n) = node.named_child(0) {
                    self.push_ref(
                        format!("{ty_path}.{}", self.text(n)),
                        self.line(n),
                        EdgeKind::Field,
                    );
                }
            }
            _ => self.visit(node),
        }
    }

    /// `pub(crate)`, `pub(super)`, `pub(in a::b)`: the restriction names a module.
    fn visibility(&mut self, node: Node) {
        let mut c = node.walk();
        for ch in node.children(&mut c) {
            match ch.kind() {
                "crate" | "super" | "self" => {
                    let expr = self.path(ch.kind());
                    let expr = if ch.kind() == "self" {
                        self.module.to_string()
                    } else {
                        expr
                    };
                    self.push_ref(expr, self.line(ch), EdgeKind::Path);
                }
                "scoped_identifier" => self.scoped_ref(ch, EdgeKind::Path, true),
                _ => {}
            }
        }
    }

    /// `#[arg(value_parser = parse_env)]`: the path after `=` names an item.
    fn attribute(&mut self, node: Node) {
        fn token_trees<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
            if n.kind() == "token_tree" {
                out.push(n);
            }
            let mut c = n.walk();
            for ch in n.children(&mut c) {
                token_trees(ch, out);
            }
        }
        let mut trees = Vec::new();
        token_trees(node, &mut trees);
        for tt in &trees {
            // Paths and calls anywhere in the arguments (`range(i64::from(a::MAX))`).
            if tt.parent().is_some_and(|p| p.kind() != "token_tree") {
                self.scan_tokens(*tt);
            }
        }
        for tt in trees {
            let mut c = tt.walk();
            let kids: Vec<Node> = tt.children(&mut c).collect();
            for (i, k) in kids.iter().enumerate() {
                if k.kind() != "=" {
                    continue;
                }
                let Some(first) = kids.get(i + 1).filter(|n| n.kind() == "identifier") else {
                    continue;
                };
                let mut parts = vec![self.text(*first)];
                let mut j = i + 1;
                while j + 2 < kids.len()
                    && kids[j + 1].kind() == "::"
                    && kids[j + 2].kind() == "identifier"
                {
                    parts.push(self.text(kids[j + 2]));
                    j += 2;
                }
                let segs: Vec<&str> = parts.iter().map(String::as_str).collect();
                let line = self.line(kids[j]);
                if segs.len() == 1 {
                    self.push_ref(parts[0].clone(), line, EdgeKind::Value);
                } else {
                    self.path_refs(&segs, self.line(*first), line, Some(EdgeKind::Value));
                }
            }
        }
    }

    fn is_generic(&self, name: &str) -> bool {
        self.generics.iter().any(|g| g.contains(name))
    }

    /// A bare type name in a type position. Definition names and generic parameters are not
    /// references.
    fn type_ref(&mut self, node: Node) {
        let is_def_name = node.parent().is_some_and(|p| {
            matches!(
                p.kind(),
                "struct_item"
                    | "enum_item"
                    | "trait_item"
                    | "type_item"
                    | "union_item"
                    | "type_parameter"
                    | "associated_type"
            ) && p
                .child_by_field_name("name")
                .is_some_and(|n| n.id() == node.id())
        });
        let name = self.text(node);
        if is_def_name || self.is_generic(&name) {
            return;
        }
        self.push_ref(name, self.line(node), EdgeKind::Type);
    }

    /// `a::b::C`: one `Path` reference per prefix (`a`, `a::b`) and, when `with_last`, the
    /// whole path as `kind`.
    fn scoped_ref(&mut self, node: Node, kind: EdgeKind, with_last: bool) {
        let raw = strip_generics(&self.text(node));
        let segs: Vec<&str> = raw.split("::").collect();
        let first = self.line(node);
        let last = node.end_position().row + 1 + self.row_off;
        self.path_refs(&segs, first, last, with_last.then_some(kind));
    }

    fn path_refs(
        &mut self,
        segs: &[&str],
        first_line: usize,
        last_line: usize,
        last: Option<EdgeKind>,
    ) {
        if segs.len() == 1 || segs[0].is_empty() {
            return;
        }
        let first_generic = self.is_generic(segs[0]);
        for i in 0..segs.len() - 1 {
            if i == 0 && first_generic {
                continue;
            }
            self.push_ref(
                self.path(&segs[..=i].join("::")),
                first_line,
                EdgeKind::Path,
            );
        }
        if let (Some(kind), false) = (last, first_generic) {
            self.push_ref(self.path(&segs.join("::")), last_line, kind);
        }
    }

    fn collect_generics(&self, node: Node) -> HashSet<String> {
        let mut out = HashSet::new();
        if let Some(tp) = node.child_by_field_name("type_parameters") {
            let mut c = tp.walk();
            for p in tp.named_children(&mut c) {
                let name = p
                    .child_by_field_name("name")
                    .or_else(|| p.named_child(0).filter(|n| n.kind() == "type_identifier"));
                if let Some(n) = name {
                    out.insert(self.text(n));
                }
            }
        }
        out
    }

    /// Define a struct/enum/trait/alias, then visit everything in it with it as the scope.
    fn item(&mut self, node: Node, kind: DefKind, scope: Scope) {
        self.type_def(node, kind);
        let q = self.qualify(&self.name_of(node));
        self.enter_item(node, q, scope);
    }

    fn enter_item(&mut self, node: Node, qualname: String, scope: Scope) {
        self.scope.push((qualname, scope));
        self.generics.push(self.collect_generics(node));
        self.walk(node);
        self.generics.pop();
        self.scope.pop();
    }

    fn qualify(&self, name: &str) -> String {
        match self.scope.last() {
            Some((q, _)) => format!("{q}.{name}"),
            None => name.to_string(),
        }
    }

    fn name_of(&self, node: Node) -> String {
        node.child_by_field_name("name")
            .map(|n| self.text(n))
            .unwrap_or_default()
    }

    fn type_def(&mut self, node: Node, kind: DefKind) {
        let qualname = self.qualify(&self.name_of(node));
        self.facts.defs.push(Def {
            qualname,
            kind,
            line: self.line(node),
            bases: Vec::new(),
        });
    }

    fn function(&mut self, node: Node) {
        let qualname = self.qualify(&self.name_of(node));
        let kind = match self.scope.last() {
            Some((_, Scope::Type)) => DefKind::Method,
            _ => DefKind::Function,
        };
        self.facts.defs.push(Def {
            qualname: qualname.clone(),
            kind,
            line: self.line(node),
            bases: Vec::new(),
        });
        self.scope.push((qualname, Scope::Func));
        self.locals.push(self.collect_locals(node));
        self.generics.push(self.collect_generics(node));
        self.walk(node);
        self.generics.pop();
        self.locals.pop();
        self.scope.pop();
    }

    /// Every name a pattern inside this function binds (over-approximate: ignores block scope).
    fn collect_locals(&self, func: Node) -> HashSet<String> {
        fn idents(n: Node, src: &[u8], out: &mut HashSet<String>) {
            if matches!(n.kind(), "identifier" | "shorthand_field_identifier") {
                out.insert(n.utf8_text(src).unwrap_or_default().to_string());
            }
            // `Some(x) if guard(x)`: the guard is an expression, not a binding.
            let guard = n.child_by_field_name("condition").map(|g| g.id());
            let mut c = n.walk();
            for ch in n.children(&mut c) {
                if Some(ch.id()) != guard {
                    idents(ch, src, out);
                }
            }
        }
        fn go(n: Node, src: &[u8], out: &mut HashSet<String>) {
            if let Some(p) = n.child_by_field_name("pattern") {
                idents(p, src, out);
            }
            if n.kind() == "closure_parameters" {
                idents(n, src, out);
            }
            let mut c = n.walk();
            for ch in n.children(&mut c) {
                go(ch, src, out);
            }
        }
        let mut out = HashSet::new();
        go(func, self.src, &mut out);
        out
    }

    /// Closures capture enclosing locals, and macro bodies are re-parsed inside a synthetic
    /// fn, so look through every enclosing function's bindings.
    fn is_local(&self, name: &str) -> bool {
        self.locals.iter().any(|l| l.contains(name))
    }

    fn impl_block(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let ty_name = self.type_name(ty);
        let qual = self.qualify(&ty_name);
        if let Some(tr) = node.child_by_field_name("trait") {
            let tr = self.type_name(tr);
            self.impls.push((qual.clone(), tr));
        }
        self.enter_item(node, qual, Scope::Type);
    }

    /// `Foo<T>` -> `Foo`, `a::b::Foo` -> `a.b.Foo`.
    fn type_name(&self, n: Node) -> String {
        match n.kind() {
            "generic_type" => n
                .child_by_field_name("type")
                .map(|t| self.type_name(t))
                .unwrap_or_default(),
            "scoped_type_identifier" | "scoped_identifier" => self.path(&self.text(n)),
            _ => self.text(n),
        }
    }

    fn module_item(&mut self, node: Node) {
        let name = self.name_of(node);
        let name_line = node
            .child_by_field_name("name")
            .map_or(self.line(node), |n| self.line(n));
        let mut c = node.walk();
        let vis = node
            .children(&mut c)
            .find(|n| n.kind() == "visibility_modifier");
        if let Some(v) = vis {
            self.visibility(v);
        }
        match node.child_by_field_name("body") {
            Some(body) => {
                let q = self.qualify(&name);
                self.facts.defs.push(Def {
                    qualname: q.clone(),
                    kind: DefKind::Module,
                    line: name_line,
                    bases: Vec::new(),
                });
                self.scope.push((q, Scope::Mod));
                self.walk(body);
                self.scope.pop();
            }
            // `mod x;` binds `x` to the file `x.rs` / `x/mod.rs` next to this one.
            None => {
                let module = format!("{}.{name}", self.children);
                self.push_ref(module.clone(), name_line, EdgeKind::Path);
                self.facts.imports.push(Import {
                    local: name.clone(),
                    module,
                    name: None,
                });
            }
        }
    }

    /// Rewrite a `::` path into the dotted model: `crate`, `self`, `super`, `Self` resolved.
    fn path(&self, raw: &str) -> String {
        let mut segs: Vec<String> = raw.split("::").map(str::to_string).collect();
        match segs[0].as_str() {
            "crate" => segs[0] = self.krate.to_string(),
            "self" if segs.len() > 1 => segs[0] = self.module.to_string(),
            "super" => {
                // Inside an inline `mod tests { .. }` the parent is the file's own module.
                let inline = self.scope.iter().any(|s| s.1 == Scope::Mod);
                let parent = if inline {
                    self.module
                } else {
                    self.module.rsplit_once('.').map_or(self.module, |p| p.0)
                };
                segs[0] = parent.to_string();
            }
            "Self" => segs[0] = "self".to_string(),
            _ => {}
        }
        // Chained `super::super::x`.
        while segs.len() > 1 && segs[1] == "super" {
            let parent = segs[0].rsplit_once('.').map_or(segs[0].as_str(), |p| p.0);
            segs[0] = parent.to_string();
            segs.remove(1);
        }
        segs.join(".")
    }

    fn use_tree(&mut self, node: Node, prefix: Vec<String>) {
        let join = |prefix: &[String], tail: &str| {
            let mut v = prefix.to_vec();
            v.push(tail.to_string());
            v.join("::")
        };
        match node.kind() {
            "identifier" | "scoped_identifier" | "self" | "crate" | "super" => {
                let text = self.text(node);
                self.import_refs(&prefix, &text, self.line(node));
                self.bind(&join(&prefix, &text), None);
            }
            "use_as_clause" => {
                let path = node.child_by_field_name("path").map(|n| self.text(n));
                let alias = node.child_by_field_name("alias").map(|n| self.text(n));
                if let Some(p) = &path {
                    self.import_refs(&prefix, p, self.line(node));
                }
                if let (Some(p), Some(a)) = (path, alias) {
                    self.bind(&join(&prefix, &p), Some(a));
                }
            }
            "scoped_use_list" => {
                let mut pre = prefix;
                if let Some(p) = node.child_by_field_name("path") {
                    let text = self.text(p);
                    self.import_refs(&pre, &text, self.line(p));
                    pre.push(text);
                }
                if let Some(list) = node.child_by_field_name("list") {
                    self.use_list(list, pre);
                }
            }
            "use_list" => self.use_list(node, prefix),
            "use_wildcard" => {
                let mut c = node.walk();
                let path = node.named_children(&mut c).next().map(|p| self.text(p));
                if let Some(p) = path {
                    self.import_refs(&prefix, &p, self.line(node));
                    let module = self.use_path(&join(&prefix, &p));
                    self.facts.imports.push(Import {
                        local: "*".to_string(),
                        module,
                        name: Some("*".to_string()),
                    });
                }
            }
            _ => {}
        }
    }

    /// One `Import` reference per segment of `tail` (the part of a `use` path this node
    /// spells), each naming the path so far: `use a::b::c` gives `a`, `a::b`, `a::b::c`.
    fn import_refs(&mut self, prefix: &[String], tail: &str, line: usize) {
        let mut full: Vec<String> = prefix.to_vec();
        for seg in tail.split("::") {
            if seg == "self" && !full.is_empty() {
                // `use a::{self}` names module `a` again.
                let expr = self.use_path(&full.join("::"));
                self.push_ref(expr, line, EdgeKind::Import);
                continue;
            }
            full.push(seg.to_string());
            let expr = self.use_path(&full.join("::"));
            self.push_ref(expr, line, EdgeKind::Import);
        }
    }

    fn use_list(&mut self, list: Node, prefix: Vec<String>) {
        let mut c = list.walk();
        for item in list.named_children(&mut c) {
            self.use_tree(item, prefix.clone());
        }
    }

    /// A `use` path as a dotted module path. `use x::f` where `mod x;` is declared here
    /// names the file beside/under this one.
    fn use_path(&self, full: &str) -> String {
        let first = full.split("::").next().unwrap_or(full);
        if self.declared_mods.contains(first) {
            format!("{}.{}", self.children, full.replace("::", "."))
        } else {
            self.path(full)
        }
    }

    /// Bind `a::b::c` (optionally aliased): the last segment is the imported name.
    fn bind(&mut self, full: &str, alias: Option<String>) {
        let full = full.strip_suffix("::self").unwrap_or(full);
        let dotted = self.use_path(full);
        match dotted.rsplit_once('.') {
            Some((module, name)) if !full.ends_with("::self") => {
                self.facts.imports.push(Import {
                    local: alias.unwrap_or_else(|| name.to_string()),
                    module: module.to_string(),
                    name: Some(name.to_string()),
                });
            }
            _ => {
                let local = alias.unwrap_or_else(|| dotted.rsplit('.').next().unwrap().to_string());
                self.facts.imports.push(Import {
                    local,
                    module: dotted,
                    name: None,
                });
            }
        }
    }

    fn enclosing_fn(&self) -> Option<String> {
        self.scope
            .iter()
            .rev()
            .find(|s| s.1 == Scope::Func)
            .map(|s| s.0.clone())
    }

    fn call(&mut self, node: Node) {
        let Some(f) = node.child_by_field_name("function") else {
            return;
        };
        let callee = self.callee(f);
        if callee != "__strata_macro" {
            self.push_call(callee, self.line(node));
        }
        let path_node = if f.kind() == "generic_function" {
            f.child_by_field_name("function")
        } else {
            Some(f)
        };
        if let Some(p) = path_node.filter(|p| p.kind() == "scoped_identifier") {
            self.scoped_ref(p, EdgeKind::Value, false);
        }
    }

    fn push_call(&mut self, callee: String, line: usize) {
        // `Enum::Variant(..)` and `Wrapper(..)` build values: a reference to the type or
        // variant, not a call edge.
        let last = callee.rsplit('.').next().unwrap_or(&callee);
        if last.starts_with(|c: char| c.is_uppercase()) {
            self.push_ref(callee, line, EdgeKind::Value);
            return;
        }
        let local = !callee.contains('.') && self.is_local(&callee);
        let enclosing = self.enclosing_fn();
        self.facts.calls.push(Call {
            enclosing,
            callee,
            line,
            local,
        });
    }

    fn callee(&self, f: Node) -> String {
        match f.kind() {
            "identifier" => self.text(f),
            "scoped_identifier" => self.path(&strip_generics(&self.text(f))),
            "generic_function" => f
                .child_by_field_name("function")
                .map(|g| self.callee(g))
                .unwrap_or_default(),
            "field_expression" => {
                let name = f
                    .child_by_field_name("field")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let recv = f.child_by_field_name("value");
                match recv {
                    Some(r) if simple_receiver(r) && !self.is_local(&root_ident(self, r)) => {
                        let recv: String = self.text(r).split_whitespace().collect();
                        format!("{recv}.{name}")
                    }
                    // Unknown receiver expression: leave only the method name to match on.
                    _ => format!("_.{name}"),
                }
            }
            _ => self.text(f),
        }
    }

    /// References in a macro body that is not valid Rust: walk the tokens, reading
    /// `a::b::c (..)` as a call and other identifier paths as references.
    fn scan_tokens(&mut self, tt: Node) {
        let mut c = tt.walk();
        let kids: Vec<Node> = tt.children(&mut c).collect();
        let mut i = 0;
        while i < kids.len() {
            let k = kids[i];
            if k.kind() == "token_tree" {
                self.scan_tokens(k);
                i += 1;
                continue;
            }
            if k.kind() != "identifier" {
                i += 1;
                continue;
            }
            let mut parts = vec![self.text(k)];
            let mut j = i;
            while j + 2 < kids.len()
                && kids[j + 1].kind() == "::"
                && kids[j + 2].kind() == "identifier"
            {
                parts.push(self.text(kids[j + 2]));
                j += 2;
            }
            let called = kids
                .get(j + 1)
                .is_some_and(|n| n.kind() == "token_tree" && self.text(*n).starts_with('('));
            let line = self.line(kids[j]);
            let segs: Vec<&str> = parts.iter().map(String::as_str).collect();
            if i >= 1 && kids[i - 1].kind() == "." {
                if called {
                    self.push_call(format!("_.{}", parts[0]), line);
                } else {
                    self.push_ref(format!("_.{}", parts[0]), line, EdgeKind::Field);
                }
            } else if called {
                self.path_refs(&segs, self.line(k), line, None);
                self.push_call(self.path(&segs.join("::")), line);
            } else if parts.len() > 1 {
                self.path_refs(&segs, self.line(k), line, Some(EdgeKind::Value));
            } else if parts[0].starts_with(|ch: char| ch.is_uppercase()) {
                self.push_ref(parts[0].clone(), line, EdgeKind::Value);
            }
            i = j + 1;
        }
    }

    /// Item-position macros that define a function named by their first argument.
    /// Only a known family is recognised; other macro-generated items stay invisible.
    fn define_macro_fn(&mut self, node: Node) -> bool {
        if self.enclosing_fn().is_some() {
            return false;
        }
        let Some(mac) = node.child_by_field_name("macro") else {
            return false;
        };
        let name = self.text(mac);
        if !name
            .rsplit("::")
            .next()
            .unwrap_or(&name)
            .starts_with("ioctl_")
        {
            return false;
        }
        let mut c = node.walk();
        let Some(tt) = node.children(&mut c).find(|n| n.kind() == "token_tree") else {
            return false;
        };
        let mut c2 = tt.walk();
        let Some(first) = tt
            .named_children(&mut c2)
            .find(|n| n.kind() == "identifier")
        else {
            return false;
        };
        let qualname = self.qualify(&self.text(first));
        self.facts.defs.push(Def {
            qualname,
            kind: DefKind::Function,
            line: self.line(node),
            bases: Vec::new(),
        });
        // The remaining arguments may name types (`ioctl_read!(name, 0x54, 1, Config)`).
        self.scan_tokens(tt);
        true
    }

    /// Macro arguments are opaque tokens to tree-sitter. Most std macros take expression
    /// lists, so re-parse them as call arguments to recover the calls inside.
    fn macro_call(&mut self, node: Node) {
        let mut c = node.walk();
        let Some(tt) = node.children(&mut c).find(|n| n.kind() == "token_tree") else {
            return;
        };
        let inner = self.text(tt);
        let inner = &inner[1..inner.len().saturating_sub(1).max(1)];
        let snippet = format!("fn __strata_fn() {{ __strata_macro({inner}); }}");
        let row = tt.start_position().row + self.row_off;
        let mut tmp = FileFacts::default();
        let mut impls = Vec::new();
        let scope = self.scope.clone();
        let locals = self.locals.clone();
        let had_error = parse_into(
            &snippet,
            Names {
                module: self.module,
                children: self.children,
            },
            scope,
            row,
            &locals,
            &mut tmp,
            &mut impls,
        );
        if had_error {
            // Not an expression list (`json!({ "k": f(x) })`): tree-sitter's error recovery
            // invents calls, so read `name(..)` token pairs instead.
            self.scan_tokens(tt);
            return;
        }
        // Keep calls only; the synthetic fn is not a definition.
        let enclosing = self.enclosing_fn();
        let here = self.scope.last().map(|s| s.0.clone());
        for mut r in tmp.refs {
            if r.enclosing
                .as_deref()
                .is_some_and(|e| e.ends_with("__strata_fn"))
            {
                r.enclosing = here.clone();
            }
            self.facts.refs.push(r);
        }
        for mut call in tmp.calls {
            if call
                .enclosing
                .as_deref()
                .is_some_and(|e| e.ends_with("__strata_fn"))
            {
                call.enclosing = enclosing.clone();
            }
            self.facts.calls.push(call);
        }
    }
}

/// `Vec::<T>::new` -> `Vec::new`: generic arguments never change which function is named.
fn strip_generics(s: &str) -> String {
    let mut depth = 0;
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("::::", "::")
        .trim_start_matches("::")
        .to_string()
}

/// `x`, `self`, `self.a.b`: receivers we can name. Anything else (calls, indexing) is unknown.
fn simple_receiver(n: Node) -> bool {
    match n.kind() {
        "identifier" | "self" => true,
        "field_expression" => n.child_by_field_name("value").is_some_and(simple_receiver),
        _ => false,
    }
}

/// Leftmost identifier of a receiver chain (`a` in `a.b.c`).
fn root_ident(st: &State, n: Node) -> String {
    match n.kind() {
        "field_expression" => n
            .child_by_field_name("value")
            .map(|v| root_ident(st, v))
            .unwrap_or_default(),
        _ => st.text(n),
    }
}

/// A bare `inner()` inside `outer` names the nested fn `outer.inner`; spell it out as an
/// absolute path so the resolver can find it.
fn qualify_nested_calls(facts: &mut FileFacts, module: &str) {
    let defs: HashSet<&str> = facts.defs.iter().map(|d| d.qualname.as_str()).collect();
    for call in &mut facts.calls {
        let (Some(enc), false) = (&call.enclosing, call.callee.contains('.')) else {
            continue;
        };
        let mut scope = enc.as_str();
        loop {
            let cand = format!("{scope}.{}", call.callee);
            if defs.contains(cand.as_str()) {
                call.callee = format!("{module}.{cand}");
                break;
            }
            match scope.rsplit_once('.') {
                Some((parent, _)) => scope = parent,
                None => break,
            }
        }
    }
}
