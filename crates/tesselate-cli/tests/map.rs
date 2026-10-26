mod common;

use std::path::PathBuf;

fn map(args: &[&str]) -> String {
    let out = common::tesselate()
        .arg("map")
        .args(args)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_repo"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn has_line(text: &str, want: &str) -> bool {
    text.lines().any(|l| l == want)
}

#[test]
fn the_map_lists_modules_symbols_signatures_and_docs() {
    let m = map(&[]);
    assert!(
        m.starts_with("# tesselate map | modules "),
        "{}",
        &m[..80.min(m.len())]
    );
    assert!(has_line(&m, "== shapes.util shapes/src/util.rs"), "{m}");
    assert!(
        has_line(
            &m,
            "fn double_it :35 pub fn double_it(x: f64) -> f64 — Doubles a value."
        ),
        "{m}"
    );
    assert!(has_line(&m, "struct Circle :3 pub struct Circle"), "{m}");
    assert!(has_line(&m, "  field Circle.r :4 r: f64"), "{m}");
    assert!(has_line(&m, "trait Shape :11 pub trait Shape"), "{m}");
}

#[test]
fn the_map_shows_what_each_symbol_calls_and_what_calls_it() {
    let m = map(&[]);
    let lines: Vec<&str> = m.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with("fn typed :"))
        .expect("typed in map");
    let calls = lines[at + 1];
    assert!(calls.starts_with("  > "), "{calls}");
    assert!(
        calls.contains("Gauge.level")
            && calls.contains("Circle.new")
            && calls.contains("Circle.area")
    );
    // Callers are the inverse: `typed` is among the callers of Gauge.level.
    let lvl = lines
        .iter()
        .position(|l| l.starts_with("  fn Gauge.level :"))
        .expect("Gauge.level in map");
    assert!(
        lines[lvl + 1..lvl + 4]
            .iter()
            .any(|l| l.trim_start().starts_with("< ") && l.contains("typed")),
        "{:?}",
        &lines[lvl..lvl + 4]
    );
}

#[test]
fn modules_list_the_modules_they_depend_on() {
    let m = map(&[]);
    let at = m
        .lines()
        .position(|l| l.starts_with("== app.main "))
        .unwrap();
    let deps = m.lines().nth(at + 1).unwrap();
    assert!(
        deps.starts_with("deps: ") && deps.contains("shapes"),
        "{deps}"
    );
}

#[test]
fn levels_and_budgets_trade_detail_for_size() {
    let full = map(&["--level", "3"]);
    let bare = map(&["--level", "0"]);
    assert!(bare.len() < full.len());
    assert!(!bare.lines().any(|l| l.trim_start().starts_with("> ")));
    // A budget picks the richest level that fits and says what it chose.
    let small = map(&["--budget", "1500"]);
    let head = small.lines().next().unwrap();
    let tokens: usize = head
        .split('~')
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.replace(',', "").parse().ok())
        .expect("token estimate in header");
    assert!(tokens <= 1500, "{head}");
    assert!(
        full.len() > small.len(),
        "the budget must have forced a smaller map"
    );
}

#[test]
fn map_is_also_an_agent_tool() {
    let out = common::tesselate()
        .args(["call", "map", r#"{"level":1}"#])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_repo"))
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["level"], 1);
    assert!(v["text"].as_str().unwrap().contains("== shapes "));
    assert!(v["tokens"].as_u64().unwrap() > 0);
}
