use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

fn call(tool: &str, args: Value) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_strata"))
        .args(["call", tool, &args.to_string()])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web_repo"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn el(line: usize, tag: &str, id: Value, classes: &[&str]) -> Value {
    json!({"path": "site/index.html", "line": line, "tag": tag, "id": id, "classes": classes})
}

#[test]
fn matched_elements_follows_descendant_and_child_combinators() {
    assert_eq!(
        call("matched_elements", json!({"selector": "#main .card"})),
        json!([el(12, "div", Value::Null, &["card", "wide"])])
    );
    assert_eq!(
        call("matched_elements", json!({"selector": "div > .title"})),
        json!([el(13, "p", Value::Null, &["title"])])
    );
    assert_eq!(
        call("matched_elements", json!({"selector": ".title"})),
        json!([
            el(13, "p", Value::Null, &["title"]),
            el(16, "p", Value::Null, &["title"])
        ])
    );
}

fn rule(path: &str, line: usize, selector: &str, spec: [u32; 3], props: &[&str]) -> Value {
    json!({"selector": selector, "specificity": spec, "path": path, "line": line, "properties": props})
}

fn win(path: &str, line: usize, selector: &str) -> Value {
    json!({"selector": selector, "path": path, "line": line})
}

#[test]
fn matching_rules_orders_by_cascade_and_names_each_propertys_winner() {
    assert_eq!(
        call("matching_rules", json!({"element": "site/index.html:12"})),
        json!([{
            "element": el(12, "div", Value::Null, &["card", "wide"]),
            "rules": [
                rule("site/base.css", 2, ".card", [0, 1, 0], &["padding"]),
                rule("site/theme.css", 1, ".card", [0, 1, 0], &["color"]),
                rule("site/theme.css", 3, ".card", [0, 1, 0], &["padding"]),
                rule("site/base.css", 4, ".card.wide", [0, 2, 0], &["margin"]),
                rule("site/theme.css", 4, ".card:hover", [0, 2, 0], &["padding"]),
                rule("site/base.css", 3, "#main .card", [1, 1, 0], &["color"]),
            ],
            "winners": {
                "color": win("site/base.css", 3, "#main .card"),
                "margin": win("site/base.css", 4, ".card.wide"),
                // Equal specificity: theme.css is linked after base.css, so it wins. The more
                // specific `.card:hover` only applies while hovered, so it does not.
                "padding": win("site/theme.css", 3, ".card"),
            }
        }])
    );
}

#[test]
fn later_sheets_win_ties_and_important_beats_specificity() {
    assert_eq!(
        call("matching_rules", json!({"element": "site/index.html:13"})),
        json!([{
            "element": el(13, "p", Value::Null, &["title"]),
            "rules": [
                rule("site/base.css", 1, "p", [0, 0, 1], &["color"]),
                rule("site/theme.css", 2, "p", [0, 0, 1], &["font-weight !important"]),
                rule("site/index.html", 7, ".title", [0, 1, 0], &["color"]),
                rule("site/base.css", 5, "div > .title", [0, 1, 1], &["font-weight"]),
            ],
            "winners": {
                "color": win("site/index.html", 7, ".title"),
                "font-weight": win("site/theme.css", 2, "p"),
            }
        }])
    );
}

#[test]
fn unused_selectors_match_nothing_on_any_page_that_loads_them() {
    assert_eq!(
        call("unused_selectors", json!({})),
        json!([
            {"selector": ".unused", "path": "site/base.css", "line": 4},
            // `.card` exists, but no page links orphan.css.
            {"selector": ".card", "path": "site/orphan.css", "line": 1},
        ])
    );
}

fn sref(line: usize, call: &str, selector: &str, elements: &[&str], rules: &[&str]) -> Value {
    json!({
        "caller": "site.app.wire", "call": call, "selector": selector, "state": "resolved",
        "path": "site/app.js", "line": line, "elements": elements, "rules": rules
    })
}

#[test]
fn selector_refs_link_literal_dom_selectors_to_elements_and_rules() {
    let card_rules = [
        "site/base.css:2 .card",
        "site/base.css:3 #main .card",
        "site/base.css:4 .card.wide",
        "site/theme.css:1 .card",
        "site/theme.css:3 .card",
        "site/theme.css:4 .card:hover",
        "site/theme.css:5 .card.open",
    ];
    let runtime = |line: usize, call: &str, expr: &str| {
        json!({
            "caller": "site.app.wire", "call": call, "selector": null, "state": "unresolved",
            "reason": format!("selector built at runtime: `{expr}`"),
            "path": "site/app.js", "line": line, "elements": [], "rules": []
        })
    };
    assert_eq!(
        call("selector_refs", json!({"path": "site/app.js"})),
        json!([
            sref(
                2,
                "querySelector",
                "#main .card",
                &["site/index.html:12"],
                &card_rules
            ),
            sref(
                2,
                "classList.add",
                ".open",
                &[],
                &["site/theme.css:5 .card.open"]
            ),
            sref(
                3,
                "getElementById",
                "#main",
                &["site/index.html:11"],
                &["site/base.css:3 #main .card"]
            ),
            sref(
                4,
                "querySelectorAll",
                ".title",
                &["site/index.html:13", "site/index.html:16"],
                &["site/base.css:5 div > .title", "site/index.html:7 .title"]
            ),
            runtime(5, "querySelector", "'.' + name"),
            runtime(6, "getElementsByClassName", "name"),
        ])
    );
}
