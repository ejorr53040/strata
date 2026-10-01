use super::{Call, Def, DefKind, FileFacts, Import, Language};
use tree_sitter::{Node, Parser};

pub struct Javascript;

impl Language for Javascript {
    fn name(&self) -> &'static str {
        "javascript"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["js", "mjs", "cjs"]
    }

    /// `web/util.js` → `web.util`; `web/index.js` is the directory's module, like `__init__.py`.
    fn module_name(&self, rel_path: &str) -> String {
        let stem = rel_path
            .rsplit_once('.')
            .map_or(rel_path, |(s, _)| s);
        let stem = stem.strip_suffix("/index").unwrap_or(stem);
        let stem = if stem == "index" { "" } else { stem };
        stem.replace('/', ".")
    }

    fn extract(&self, rel_path: &str, src: &str) -> FileFacts {
        let mut facts = FileFacts::default();
        let Some(tree) = parse(src) else {
            return facts;
        };
        let mut st = State {
            src: src.as_bytes(),
            dir: rel_path.rsplit_once('/').map_or("", |(d, _)| d).to_string(),
            scope: Vec::new(),
            facts: &mut facts,
        };
        st.walk(tree.root_node());
        let root = tree.root_node();
        let mut c = root.walk();
        facts.shares_globals = !root
            .children(&mut c)
            .any(|n| matches!(n.kind(), "import_statement" | "export_statement"));
        facts
    }

    fn is_builtin(&self, name: &str) -> bool {
        BUILTINS.contains(&name)
    }
}

/// Globals every browser script can reach: the DOM, timers, and the standard library.
const BUILTINS: &[&str] = &[
    "window",
    "document",
    "console",
    "navigator",
    "location",
    "history",
    "localStorage",
    "sessionStorage",
    "fetch",
    "alert",
    "confirm",
    "prompt",
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "requestAnimationFrame",
    "structuredClone",
    "Node",
    "Element",
    "HTMLElement",
    "Event",
    "CustomEvent",
    "FormData",
    "URL",
    "URLSearchParams",
    "JSON",
    "Math",
    "Object",
    "Array",
    "String",
    "Number",
    "Boolean",
    "Symbol",
    "Date",
    "RegExp",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Promise",
    "Error",
    "TypeError",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "require",
];

fn parse(src: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .unwrap();
    parser.parse(src, None)
}

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Class,
    Func,
}

struct State<'a> {
    src: &'a [u8],
    /// Directory of the file, repo-relative, for resolving `./x.js` imports.
    dir: String,
    scope: Vec<(String, Scope)>,
    facts: &'a mut FileFacts,
}

impl State<'_> {
    fn text(&self, n: Node) -> String {
        n.utf8_text(self.src).unwrap_or_default().to_string()
    }

    fn walk(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }

    fn visit(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                self.define(node, node.child_by_field_name("name"), DefKind::Function)
            }
            "class_declaration" => {
                self.define(node, node.child_by_field_name("name"), DefKind::Class)
            }
            "method_definition" => {
                self.define(node, node.child_by_field_name("name"), DefKind::Method)
            }
            "variable_declarator"
                if node.child_by_field_name("value").is_some_and(|v| {
                    matches!(v.kind(), "arrow_function" | "function_expression")
                }) =>
            {
                let value = node.child_by_field_name("value").unwrap();
                self.define(value, node.child_by_field_name("name"), DefKind::Function)
            }
            "call_expression" => {
                self.call(node, node.child_by_field_name("function"));
                self.walk(node);
            }
            "new_expression" => {
                self.call(node, node.child_by_field_name("constructor"));
                self.walk(node);
            }
            "import_statement" => self.import(node),
            _ => self.walk(node),
        }
    }

    fn call(&mut self, node: Node, callee: Option<Node>) {
        let Some(callee) = callee else {
            return;
        };
        let text = self.spell(callee);
        // `this.m()` dispatches like Python's `self.m()`.
        let callee = match text.strip_prefix("this.") {
            Some(rest) => format!("self.{rest}"),
            None => text,
        };
        let enclosing = self
            .scope
            .iter()
            .rev()
            .find(|s| s.1 == Scope::Func)
            .map(|s| s.0.clone());
        self.facts.calls.push(Call {
            enclosing,
            callee,
            line: node.start_position().row + 1,
            local: false,
            recv_local: false,
        });
    }

    /// A callee as a member chain with arguments elided: `document.querySelectorAll('.x').forEach`
    /// reads `document.querySelectorAll().forEach`.
    fn spell(&self, node: Node) -> String {
        match node.kind() {
            "call_expression" => match node.child_by_field_name("function") {
                Some(f) => format!("{}()", self.spell(f)),
                None => self.text(node),
            },
            "member_expression" => match (
                node.child_by_field_name("object"),
                node.child_by_field_name("property"),
            ) {
                (Some(o), Some(p)) => format!("{}.{}", self.spell(o), self.text(p)),
                _ => self.text(node),
            },
            _ => self.text(node),
        }
    }

    /// `import d, { a as b } from './m.js'` / `import * as ns from 'pkg'`.
    fn import(&mut self, node: Node) {
        let Some(source) = node.child_by_field_name("source") else {
            return;
        };
        let module = self.module_of(self.text(source).trim_matches(|c| c == '\'' || c == '"'));
        let mut c = node.walk();
        let Some(clause) = node.children(&mut c).find(|n| n.kind() == "import_clause") else {
            return;
        };
        let mut c = clause.walk();
        for part in clause.named_children(&mut c) {
            match part.kind() {
                "identifier" => self.facts.imports.push(Import {
                    local: self.text(part),
                    module: module.clone(),
                    name: None,
                }),
                "namespace_import" => {
                    if let Some(id) = part.named_child(0) {
                        self.facts.imports.push(Import {
                            local: self.text(id),
                            module: module.clone(),
                            name: None,
                        });
                    }
                }
                "named_imports" => {
                    let mut c = part.walk();
                    for spec in part.named_children(&mut c) {
                        let Some(name) = spec.child_by_field_name("name") else {
                            continue;
                        };
                        let name = self.text(name);
                        let local = spec
                            .child_by_field_name("alias")
                            .map_or_else(|| name.clone(), |a| self.text(a));
                        self.facts.imports.push(Import {
                            local,
                            module: module.clone(),
                            name: Some(name),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// A relative specifier becomes the dotted module of the file it names; a package
    /// specifier (`lodash`, `@scope/pkg`) is kept as written and reads as external.
    fn module_of(&self, spec: &str) -> String {
        if !spec.starts_with('.') {
            return spec.to_string();
        }
        let mut parts: Vec<&str> = self.dir.split('/').filter(|p| !p.is_empty()).collect();
        for seg in spec.split('/') {
            match seg {
                "." | "" => {}
                ".." => {
                    parts.pop();
                }
                s => parts.push(s),
            }
        }
        Javascript.module_name(&parts.join("/"))
    }

    /// Records a definition named by `name` and walks `node`'s body inside its scope.
    fn define(&mut self, node: Node, name: Option<Node>, kind: DefKind) {
        let Some(name) = name else {
            return self.walk(node);
        };
        let name = self.text(name);
        let parent = self.scope.last().cloned();
        let qualname = match &parent {
            Some((q, _)) => format!("{q}.{name}"),
            None => name,
        };
        let mut bases = Vec::new();
        if kind == DefKind::Class {
            let mut c = node.walk();
            let heritage = node
                .children(&mut c)
                .find(|n| n.kind() == "class_heritage");
            if let Some(base) = heritage.and_then(|h| h.named_child(0)) {
                bases.push(self.text(base));
            }
        }
        self.facts.defs.push(Def {
            qualname: qualname.clone(),
            kind,
            line: node.start_position().row + 1,
            bases,
            ty: None,
            owner: None,
        });
        let scope = if kind == DefKind::Class {
            Scope::Class
        } else {
            Scope::Func
        };
        self.scope.push((qualname, scope));
        if let Some(body) = node.child_by_field_name("body") {
            self.visit(body);
        }
        self.scope.pop();
    }
}
