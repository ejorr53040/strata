use serde_json::json;
use std::fs;
use std::path::Path;
use tesselate_core::snapshot::Snapshot;
use tesselate_core::walk::WalkOptions;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "app/util.py", "def helper():\n    return 1\n");
    write(
        dir.path(),
        "app/main.py",
        "from app.util import helper\n\n\ndef main():\n    helper()\n",
    );
    dir
}

fn caller_lines(snap: &Snapshot, symbol: &str) -> Vec<String> {
    let out = snap.call("callers", &json!({ "symbol": symbol })).unwrap();
    out.as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{} {}:{}",
                e["caller"].as_str().unwrap(),
                e["path"].as_str().unwrap(),
                e["line"]
            )
        })
        .collect()
}

#[test]
fn one_snapshot_answers_repeated_tool_calls() {
    let dir = repo();
    let snap = Snapshot::build(dir.path(), &WalkOptions::default());
    for _ in 0..3 {
        assert_eq!(
            caller_lines(&snap, "app.util.helper"),
            vec!["app.main.main app/main.py:5"]
        );
    }
    let syms = snap.call("symbols", &json!({"query": "helper"})).unwrap();
    assert_eq!(
        (syms[0]["name"].as_str(), syms[0]["path"].as_str()),
        (Some("helper"), Some("app/util.py"))
    );
    assert_eq!(
        snap.call("overview", &json!({})).unwrap()["files"]["indexed"],
        2
    );
}

#[test]
fn unknown_tool_is_an_error_naming_the_tool() {
    let dir = repo();
    let snap = Snapshot::build(dir.path(), &WalkOptions::default());
    let err = snap.call("nope", &json!({})).unwrap_err();
    assert!(err.contains("unknown tool `nope`"), "{err}");
}

#[test]
fn snapshot_is_fresh_until_an_indexed_file_changes() {
    let dir = repo();
    let snap = Snapshot::build(dir.path(), &WalkOptions::default());
    assert!(!snap.is_stale());
    write(dir.path(), "app/util.py", "def helper():\n    return 22\n");
    assert!(snap.is_stale());
}

#[test]
fn snapshot_is_stale_after_an_indexed_file_is_deleted() {
    let dir = repo();
    let snap = Snapshot::build(dir.path(), &WalkOptions::default());
    fs::remove_file(dir.path().join("app/util.py")).unwrap();
    assert!(snap.is_stale());
}
