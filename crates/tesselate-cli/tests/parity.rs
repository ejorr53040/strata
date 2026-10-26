//! Every argument-free tool, through the daemon, answers exactly what tesselate answered before
//! the daemon existed. `tests/golden/<fixture>/<tool>.json` was recorded with the
//! rebuild-per-call binary at 03b5f27; re-record only for an intended output change.

mod common;

use std::path::PathBuf;

const TOOLS: &[(&str, &str)] = &[
    ("overview", "{}"),
    ("symbols", r#"{"query":""}"#),
    ("edges", "{}"),
    ("unindexed_files", "{}"),
    ("map", "{}"),
    ("pages", "{}"),
    ("stylesheets", "{}"),
    ("unused_selectors", "{}"),
    ("selector_refs", "{}"),
];

#[test]
fn every_tool_answers_as_before_the_daemon() {
    let tests = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    for fixture in ["call_repo", "py_repo", "rust_repo", "web_repo"] {
        for (tool, args) in TOOLS {
            let out = common::tesselate()
                .args(["call", tool, args])
                .current_dir(tests.join("fixtures").join(fixture))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let got: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            let golden = tests.join(format!("golden/{fixture}/{tool}.json"));
            let want: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&golden).unwrap()).unwrap();
            assert!(
                got == want,
                "{fixture} {tool} differs from {}",
                golden.display()
            );
        }
    }
}
