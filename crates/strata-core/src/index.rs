use std::path::Path;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Symbol {
    pub kind: &'static str,
    pub name: String,
    pub path: String,
    pub line: usize,
}

/// Index every `.py` file under `root` and return symbols whose name contains `query`
/// (case-insensitive), sorted by path then line.
pub fn symbols(root: &Path, query: &str) -> Vec<Symbol> {
    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    let q = query.to_lowercase();
    let mut out = Vec::new();
    for file in files {
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let Some(tree) = parser.parse(&src, None) else {
            continue;
        };
        walk(tree.root_node(), src.as_bytes(), &rel, None, &mut out);
    }
    out.retain(|s| s.name.to_lowercase().contains(&q));
    out
}

fn collect(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, files);
        } else if p.extension().is_some_and(|x| x == "py") {
            files.push(p);
        }
    }
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
