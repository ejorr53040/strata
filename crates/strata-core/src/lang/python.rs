use super::{Call, Def, DefKind, FileFacts, Import, Language};
use tree_sitter::{Node, Parser};

pub struct Python;

impl Language for Python {
    fn name(&self) -> &'static str {
        "python"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["py"]
    }

    fn module_name(&self, rel_path: &str) -> String {
        let stem = rel_path.strip_suffix(".py").unwrap_or(rel_path);
        let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
        let stem = if stem == "__init__" { "" } else { stem };
        stem.replace('/', ".")
    }

    fn extract(&self, rel_path: &str, src: &str) -> FileFacts {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let mut facts = FileFacts::default();
        let Some(tree) = parser.parse(src, None) else {
            return facts;
        };
        let mut st = State {
            src: src.as_bytes(),
            module: self.module_name(rel_path),
            is_pkg: rel_path.ends_with("__init__.py"),
            scope: Vec::new(),
            facts: &mut facts,
        };
        st.walk(tree.root_node());
        facts
    }

    fn is_builtin(&self, name: &str) -> bool {
        BUILTINS.contains(&name)
    }
}

const BUILTINS: &[&str] = &[
    "print",
    "len",
    "range",
    "str",
    "int",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "open",
    "isinstance",
    "issubclass",
    "sorted",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sum",
    "min",
    "max",
    "abs",
    "any",
    "all",
    "repr",
    "type",
    "super",
    "getattr",
    "setattr",
    "hasattr",
    "iter",
    "next",
    "reversed",
    "round",
    "bytes",
    "id",
    "hash",
    "input",
    "format",
    "vars",
    "dir",
    "callable",
    "Exception",
    "ValueError",
    "TypeError",
    "KeyError",
    "RuntimeError",
    "NotImplementedError",
];

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Class,
    Func,
}

struct State<'a> {
    src: &'a [u8],
    module: String,
    is_pkg: bool,
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
            match child.kind() {
                "function_definition" | "class_definition" => self.definition(child),
                "call" => {
                    self.call(child);
                    self.walk(child);
                }
                "import_statement" => self.import(child),
                "import_from_statement" => self.import_from(child),
                _ => self.walk(child),
            }
        }
    }

    fn definition(&mut self, node: Node) {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n))
            .unwrap_or_default();
        let is_class = node.kind() == "class_definition";
        let parent = self.scope.last().cloned();
        let qualname = match &parent {
            Some((q, _)) => format!("{q}.{name}"),
            None => name,
        };
        let kind = match (is_class, parent.map(|p| p.1)) {
            (true, _) => DefKind::Class,
            (false, Some(Scope::Class)) => DefKind::Method,
            (false, _) => DefKind::Function,
        };
        let mut bases = Vec::new();
        if let Some(sup) = node.child_by_field_name("superclasses") {
            let mut c = sup.walk();
            for b in sup.named_children(&mut c) {
                if matches!(b.kind(), "identifier" | "attribute") {
                    bases.push(self.text(b));
                }
            }
        }
        self.facts.defs.push(Def {
            qualname: qualname.clone(),
            kind,
            line: node.start_position().row + 1,
            bases,
        });
        self.scope
            .push((qualname, if is_class { Scope::Class } else { Scope::Func }));
        if let Some(body) = node.child_by_field_name("body") {
            self.walk(body);
        }
        self.scope.pop();
    }

    fn call(&mut self, node: Node) {
        let Some(f) = node.child_by_field_name("function") else {
            return;
        };
        let callee = self.text(f);
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
        });
    }

    fn import(&mut self, node: Node) {
        let mut c = node.walk();
        for n in node.named_children(&mut c) {
            match n.kind() {
                "dotted_name" => {
                    let t = self.text(n);
                    self.facts.imports.push(Import {
                        local: t.clone(),
                        module: t,
                        name: None,
                    });
                }
                "aliased_import" => {
                    let module = n
                        .child_by_field_name("name")
                        .map(|x| self.text(x))
                        .unwrap_or_default();
                    let local = n
                        .child_by_field_name("alias")
                        .map(|x| self.text(x))
                        .unwrap_or_default();
                    self.facts.imports.push(Import {
                        local,
                        module,
                        name: None,
                    });
                }
                _ => {}
            }
        }
    }

    fn import_from(&mut self, node: Node) {
        let Some(m) = node.child_by_field_name("module_name") else {
            return;
        };
        let raw = self.text(m);
        let module = if m.kind() == "relative_import" {
            let dots = raw.chars().take_while(|c| *c == '.').count();
            let rest = &raw[dots..];
            let mut parts: Vec<&str> = if self.module.is_empty() {
                vec![]
            } else {
                self.module.split('.').collect()
            };
            if !self.is_pkg {
                parts.pop();
            }
            for _ in 1..dots {
                parts.pop();
            }
            if !rest.is_empty() {
                parts.push(rest);
            }
            parts.join(".")
        } else {
            raw
        };
        let mut c = node.walk();
        for n in node.children_by_field_name("name", &mut c) {
            let (name, local) = if n.kind() == "aliased_import" {
                (
                    n.child_by_field_name("name")
                        .map(|x| self.text(x))
                        .unwrap_or_default(),
                    n.child_by_field_name("alias")
                        .map(|x| self.text(x))
                        .unwrap_or_default(),
                )
            } else {
                (self.text(n), self.text(n))
            };
            self.facts.imports.push(Import {
                local,
                module: module.clone(),
                name: Some(name),
            });
        }
    }
}
