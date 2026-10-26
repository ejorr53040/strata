mod common;

use std::path::PathBuf;
use std::process::Output;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/py_repo")
}

fn strata(args: &[&str]) -> Output {
    common::strata()
        .args(args)
        .current_dir(fixture())
        .output()
        .unwrap()
}

#[test]
fn symbols_finds_function_class_and_method_with_path_and_line() {
    let out = strata(&["symbols", "r"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.contains(&"function area pkg/shapes.py:1"), "{text}");
    assert!(
        lines.contains(&"method Circle.radius pkg/shapes.py:6"),
        "{text}"
    );
    assert!(lines.contains(&"class Circle pkg/shapes.py:5"), "{text}");
}

#[test]
fn json_flag_emits_same_facts_as_json() {
    let out = strata(&["symbols", "r", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let arr = v.as_array().unwrap();
    assert!(arr.contains(&serde_json::json!(
        {"kind": "method", "name": "Circle.radius", "path": "pkg/shapes.py", "line": 6}
    )));
    assert_eq!(arr.len(), 4);
}
