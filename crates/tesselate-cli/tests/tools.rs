mod common;

use std::path::PathBuf;
use std::process::Output;

fn tesselate(args: &[&str]) -> Output {
    common::tesselate()
        .args(args)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_repo"))
        .output()
        .unwrap()
}

fn json(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn tools_lists_a_catalog_with_input_schemas() {
    let v = json(&tesselate(&["tools"]));
    let tools = v.as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in [
        "overview",
        "symbols",
        "callers",
        "callees",
        "unindexed_files",
    ] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    let callees = tools.iter().find(|t| t["name"] == "callees").unwrap();
    assert_eq!(callees["input_schema"]["required"][0], "symbol");
    assert!(callees["description"].as_str().unwrap().len() > 20);
}

#[test]
fn call_dispatches_a_tool_by_name_with_json_arguments() {
    let v = json(&tesselate(&[
        "call",
        "callees",
        r#"{"symbol":"shapes.Circle.area"}"#,
    ]));
    let edges = v.as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["state"], "resolved");
    assert_eq!(edges[0]["targets"][0], "shapes.util.square");
}

#[test]
fn call_rejects_unknown_tools_and_missing_arguments() {
    let out = tesselate(&["call", "nope", "{}"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown tool `nope`"));

    let out = tesselate(&["call", "callees", "{}"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("symbol"));
}

#[test]
fn overview_summarises_symbols_and_edge_states() {
    let v = json(&tesselate(&["call", "overview", "{}"]));
    assert!(v["files"]["indexed"].as_u64().unwrap() >= 7);
    assert!(v["symbols"]["struct"].as_u64().unwrap() >= 3);
    assert_eq!(v["symbols"]["trait"], 1);
    assert!(v["edges"]["resolved"].as_u64().unwrap() >= 4);
    assert!(v["edges"]["candidate"].as_u64().unwrap() >= 1);
}

#[test]
fn guide_tells_agents_how_to_use_the_tools() {
    let out = tesselate(&["guide"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("tesselate call") && text.contains("candidate"));
}

#[test]
fn edges_dumps_every_edge_with_a_kind() {
    let v = json(&tesselate(&["call", "edges", "{}"]));
    let edges = v.as_array().unwrap();
    assert!(edges.len() > 10);
    assert!(edges.iter().all(|e| e["kind"].is_string()));
    assert!(edges.iter().any(|e| e["kind"] == "call"));
}

#[test]
fn references_lists_every_kind_of_use_of_a_symbol() {
    let v = json(&tesselate(&[
        "call",
        "references",
        r#"{"symbol":"shapes.util.square"}"#,
    ]));
    let edges = v.as_array().unwrap();
    assert!(edges.len() >= 2);
    assert!(edges.iter().all(|e| e["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "shapes.util.square")));
}
