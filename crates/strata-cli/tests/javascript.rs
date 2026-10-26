mod common;

use std::path::PathBuf;

fn run(args: &[&str]) -> Vec<String> {
    let out = common::strata()
        .args(args)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web_repo"))
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
fn symbols_lists_js_functions_classes_methods_and_arrow_consts() {
    let lines = run(&["symbols", ""]);
    for want in [
        "function helper web/util.js:1",
        "class Counter web/util.js:5",
        "method Counter.constructor web/util.js:6",
        "method Counter.bump web/util.js:10",
        "function double web/util.js:15",
    ] {
        assert!(
            lines.contains(&want.to_string()),
            "missing {want}: {lines:#?}"
        );
    }
}

#[test]
fn callees_resolve_es_imports_and_mark_browser_and_packages_external() {
    assert_eq!(
        run(&["callees", "web.main.main"]),
        vec![
            "resolved web.main.main -> web.util.Counter web/main.js:6",
            "resolved web.main.main -> web.util.helper web/main.js:7",
            "resolved web.main.main -> web.util.double web/main.js:8",
            "external web.main.main -> confetti (import canvas-confetti) web/main.js:9",
            "external web.main.main -> console.log (builtin) web/main.js:10",
            "unresolved web.main.main -> missing (`missing` is not defined in module scope or imports) web/main.js:11",
        ]
    );
}

#[test]
fn this_calls_dispatch_through_extends() {
    assert_eq!(
        run(&["callees", "web.main.Panel.open"]),
        vec!["resolved web.main.Panel.open -> web.util.Counter.bump web/main.js:16"]
    );
}

#[test]
fn classic_scripts_share_top_level_functions_as_globals() {
    assert_eq!(
        run(&["callees", "static.page.init"]),
        vec![
            "resolved static.page.init -> static.widgets.render static/page.js:2",
            "candidate static.page.init -> static.legacy.setup,static.widgets.setup static/page.js:3",
            "external static.page.init -> localStorage.getItem (builtin) static/page.js:4",
            "unresolved static.page.init -> helper (`helper` is not defined in module scope or imports) static/page.js:5",
        ]
    );
}

#[test]
fn es_modules_do_not_leak_top_level_names_as_globals() {
    // `web/util.js` exports `helper`; a classic script calling `helper()` cannot see it.
    let lines = run(&["callers", "web.util.helper"]);
    assert!(lines.iter().all(|l| !l.contains("static.")), "{lines:#?}");
}

#[test]
fn chains_rooted_in_a_browser_global_are_external_with_arguments_elided() {
    let lines = run(&["callees", "static.page.wire"]);
    for want in [
        "external static.page.wire -> document.querySelectorAll().forEach (builtin) static/page.js:9",
        "external static.page.wire -> document.querySelectorAll (builtin) static/page.js:9",
    ] {
        assert!(lines.contains(&want.to_string()), "missing {want}: {lines:#?}");
    }
}

#[test]
fn extensionless_imports_resolve_inside_dotted_directories() {
    assert_eq!(
        run(&["callees", "web.v1.2.main.run"]),
        vec!["resolved web.v1.2.main.run -> web.v1.2.util.f web/v1.2/main.js:2"]
    );
}
