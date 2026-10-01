use crate::walk::WalkOptions;
use std::path::Path;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Symbol {
    pub kind: &'static str,
    pub name: String,
    pub path: String,
    pub line: usize,
}

/// Index every non-skipped `.py` file under `root` and return symbols whose name contains
/// `query` (case-insensitive), sorted by path then line.
pub fn symbols(root: &Path, query: &str) -> Vec<Symbol> {
    symbols_with(root, query, &WalkOptions::default())
}

/// Like [`symbols`], with explicit walk options (e.g. `index.include`).
pub fn symbols_with(root: &Path, query: &str, opts: &WalkOptions) -> Vec<Symbol> {
    let q = query.to_lowercase();
    let mut out = Vec::new();
    for file in crate::walk::walk(root, opts).indexed {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let Some(tree) = parser.parse(&file.text, None) else {
            continue;
        };
        walk(
            tree.root_node(),
            file.text.as_bytes(),
            &file.rel,
            None,
            &mut out,
        );
    }
    out.retain(|s| s.name.to_lowercase().contains(&q));
    out
}

fn walk(node: Node, src: &[u8], path: &str, class: Option<&str>, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" | "class_definition" => {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                    .unwrap_or_default()
                    .to_string();
                let is_class = child.kind() == "class_definition";
                let (kind, shown) = match (is_class, class) {
                    (true, _) => ("class", name.clone()),
                    (false, Some(c)) => ("method", format!("{c}.{name}")),
                    (false, None) => ("function", name.clone()),
                };
                out.push(Symbol {
                    kind,
                    name: shown,
                    path: path.into(),
                    line: child.start_position().row + 1,
                });
                if let Some(body) = child.child_by_field_name("body") {
                    walk(
                        body,
                        src,
                        path,
                        if is_class { Some(&name) } else { None },
                        out,
                    );
                }
            }
            _ => walk(child, src, path, class, out),
        }
    }
}
