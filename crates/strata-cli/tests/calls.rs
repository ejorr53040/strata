use std::path::PathBuf;
use std::process::Command;

fn run(args: &[&str]) -> Vec<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_strata"))
        .args(args)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/call_repo"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn callees_lists_every_call_site_with_its_edge_state() {
    assert_eq!(
        run(&["callees", "app.main.main"]),
        vec![
            "resolved app.main.main -> app.util.helper app/main.py:8",
            "resolved app.main.main -> app.util.other app/main.py:9",
            "external app.main.main -> requests.get (import requests) app/main.py:10",
            "resolved app.main.main -> app.models.Base app/main.py:11",
            "candidate app.main.main -> app.models.Base.run app/main.py:12",
        ]
    );
}

#[test]
fn callers_lists_call_sites_targeting_the_symbol_including_candidates() {
    assert_eq!(
        run(&["callers", "app.util.helper"]),
        vec![
            "resolved app.main.main -> app.util.helper app/main.py:8",
            "resolved app.util.other -> app.util.helper app/util.py:6",
        ]
    );
    assert_eq!(
        run(&["callers", "app.models.Sub.step"]),
        vec!["candidate app.models.Base.run -> app.models.Base.step,app.models.Sub.step app/models.py:3"]
    );
}

#[test]
fn symbol_may_be_given_by_unique_dotted_suffix() {
    assert_eq!(run(&["callers", "util.helper"]).len(), 2);
}

#[test]
fn json_output_carries_state_and_targets() {
    let out = Command::new(env!("CARGO_BIN_EXE_strata"))
        .args(["callees", "app.util.other", "--json"])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/call_repo"))
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v,
        serde_json::json!([{
            "caller": "app.util.other",
            "callee": "helper",
            "state": "resolved",
            "targets": ["app.util.helper"],
            "path": "app/util.py",
            "line": 6
        }])
    );
}
