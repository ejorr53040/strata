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
            // `extra.css` reaches imp/index.html through main.css's `@import`, so `.x` is used.
            {"selector": ".y", "path": "imp/main.css", "line": 2},
            {"selector": ".unused", "path": "site/base.css", "line": 4},
            // `.card` exists, but no page links orphan.css. `admin/app.js` adds `unused`, but
            // only on the admin page, which does not load base.css.
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
    let runtime = |line: usize, call: &str, reason: &str| {
        json!({
            "caller": "site.app.wire", "call": call, "selector": null, "state": "unresolved",
            "reason": reason,
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
            runtime(
                5,
                "querySelector",
                "selector built at runtime: `'.' + name`"
            ),
            runtime(
                6,
                "getElementsByClassName",
                "selector is the variable `name`, whose value is not tracked"
            ),
        ])
    );
}

#[test]
fn pages_list_what_each_page_loads_and_names() {
    assert_eq!(
        call("pages", json!({})),
        json!([{
            "path": "admin/index.html",
            "stylesheets": ["admin/admin.css"],
            "inline_styles": 0,
            "scripts": ["admin/app.js"],
            "ids": ["a.b"],
            "classes": ["m", "panel", "sm:p-2"],
        }, {
            // An imported sheet comes before the sheet importing it.
            "path": "imp/index.html",
            "stylesheets": ["imp/extra.css", "imp/main.css"],
            "inline_styles": 0,
            "scripts": [],
            "ids": [],
            "classes": ["x"],
        }, {
            "path": "site/index.html",
            "stylesheets": ["site/base.css", "site/theme.css"],
            "inline_styles": 1,
            "scripts": ["site/app.js"],
            "ids": ["main"],
            "classes": ["card", "title", "wide"],
        }])
    );
}

#[test]
fn stylesheets_list_rules_and_mark_the_unused() {
    let r = |selector: &str, line: usize, unused: bool| json!({"selector": selector, "line": line, "unused": unused});
    assert_eq!(
        call("stylesheets", json!({})),
        json!([
            {"path": "admin/admin.css", "rules": [r(".panel", 1, false), r(".sm\\:p-2", 2, false), r(".is\\:on", 3, false),
                r(".m", 4, false), r(".m", 5, false), r(".m", 6, false)]},
            {"path": "imp/extra.css", "rules": [r(".x", 1, false)]},
            {"path": "imp/main.css", "rules": [r(".y", 2, true)]},
            {"path": "site/base.css", "rules": [
                r("p", 1, false), r(".card", 2, false), r("#main .card", 3, false),
                r(".card.wide", 4, false), r(".unused", 4, true), r("div > .title", 5, false),
            ]},
            {"path": "site/orphan.css", "rules": [r(".card", 1, true)]},
            {"path": "site/theme.css", "rules": [
                r(".card", 1, false), r("p", 2, false), r(".card", 3, false),
                r(".card:hover", 4, false), r(".card.open", 5, false),
            ]},
            // A `<style>` block is listed under its page.
            {"path": "site/index.html", "rules": [r(".title", 7, false)]},
        ])
    );
}

#[test]
fn escaped_characters_in_selectors_and_ids_match_literally() {
    let p = json!({"path": "admin/index.html", "line": 4, "tag": "p", "id": "a.b", "classes": ["sm:p-2"]});
    assert_eq!(
        call("matched_elements", json!({"selector": ".sm\\:p-2"})),
        json!([p])
    );
    let refs = call("selector_refs", json!({"path": "admin/app.js"}));
    let by_id = refs
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["call"] == "getElementById")
        .unwrap();
    assert_eq!(by_id["selector"], "#a\\.b");
    assert_eq!(by_id["elements"], json!(["admin/index.html:4"]));
}

#[test]
fn inline_style_beats_normal_rules_and_media_gated_rules_never_win() {
    assert_eq!(
        call("matching_rules", json!({"element": "admin/index.html:5"})),
        json!([{
            "element": {"path": "admin/index.html", "line": 5, "tag": "p", "id": null, "classes": ["m"]},
            "rules": [
                rule("admin/admin.css", 4, ".m", [0, 1, 0], &["color"]),
                {"selector": ".m", "specificity": [0, 1, 0], "path": "admin/admin.css", "line": 5,
                 "properties": ["padding"], "media": "print"},
                rule("admin/admin.css", 6, ".m", [0, 1, 0], &["margin !important"]),
            ],
            "winners": {
                "color": win("admin/index.html", 5, "style attribute"),
                "margin": win("admin/admin.css", 6, ".m"),
            }
        }])
    );
}
