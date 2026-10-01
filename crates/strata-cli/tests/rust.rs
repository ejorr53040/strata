use std::path::PathBuf;
use std::process::Command;

fn run(args: &[&str]) -> Vec<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_strata"))
        .args(args)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_repo"))
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
fn symbols_lists_rust_items_with_kinds_and_impl_methods() {
    let lines = run(&["symbols", ""]);
    for want in [
        "struct Circle shapes/src/lib.rs:3",
        "enum Kind shapes/src/lib.rs:7",
        "trait Shape shapes/src/lib.rs:11",
        "method Shape.area shapes/src/lib.rs:12",
        "method Circle.new shapes/src/lib.rs:19",
        "method Circle.area shapes/src/lib.rs:28",
        "function square shapes/src/util.rs:3",
        "function main app/src/main.rs:4",
    ] {
        assert!(
            lines.iter().any(|l| l == want),
            "missing `{want}` in {lines:#?}"
        );
    }
}

#[test]
fn callees_resolve_paths_imports_and_self_calls() {
    assert_eq!(
        run(&["callees", "shapes.Circle.area"]),
        vec![
            "resolved shapes.Circle.area -> shapes.util.square shapes/src/lib.rs:29",
            "resolved shapes.Circle.area -> shapes.Circle.unit shapes/src/lib.rs:29",
        ]
    );
    assert_eq!(
        run(&["callees", "shapes.util.make"]),
        vec!["resolved shapes.util.make -> shapes.Circle.new shapes/src/util.rs:8"]
    );
    assert_eq!(
        run(&["callees", "app.main.main"]),
        vec![
            "resolved app.main.main -> shapes.Circle.new app/src/main.rs:5",
            "candidate app.main.main -> shapes.Circle.area,shapes.Shape.area app/src/main.rs:6",
            "resolved app.main.main -> shapes.util.square app/src/main.rs:7",
            "external app.main.main -> std.process.exit (builtin) app/src/main.rs:8",
        ]
    );
}

#[test]
fn local_variables_shadow_imported_module_names() {
    assert_eq!(
        run(&["callees", "app.main.shadow"]),
        vec![
            "resolved app.main.shadow -> shapes.Circle.new app/src/main.rs:14",
            "candidate app.main.shadow -> shapes.Circle.area,shapes.Shape.area app/src/main.rs:15",
        ]
    );
}

#[test]
fn std_methods_nested_fns_and_constructors_do_not_pollute_the_graph() {
    assert_eq!(
        run(&["callees", "app.main.ext"]),
        vec![
            "external app.main.ext -> Vec.new (builtin) app/src/main.rs:19",
            "external app.main.ext -> _.push (no indexed method `push`; receiver type unknown) app/src/main.rs:21",
            "resolved app.main.ext -> app.main.ext.inner app/src/main.rs:23",
        ]
    );
}

#[test]
fn dogfooding_gaps_multiline_receivers_derives_reexports_and_macro_closures() {
    assert_eq!(
        run(&["callees", "shapes.Circle.twice"]),
        vec!["resolved shapes.Circle.twice -> shapes.Circle.area shapes/src/lib.rs:35"]
    );
    assert_eq!(
        run(&["callees", "shapes.config"]),
        vec!["external shapes.config -> Config.default (`shapes.Config` has no indexed member `default`; derived or trait method) shapes/src/lib.rs:47"]
    );
    assert_eq!(
        run(&["callees", "app.main.reexport"]),
        vec!["resolved app.main.reexport -> shapes.util.make app/src/main.rs:31"]
    );
    assert!(run(&["callees", "app.main.closure_in_macro"]).is_empty());
    assert_eq!(
        run(&["callees", "app.main.qualified"]),
        vec!["external app.main.qualified -> shapes.Config.default (`shapes.Config` has no indexed member `default`; derived or trait method) app/src/main.rs:40"]
    );
    assert_eq!(
        run(&["callees", "shapes.Circle.thrice"]),
        vec!["candidate shapes.Circle.thrice -> shapes.Circle.area,shapes.Shape.area shapes/src/lib.rs:54"]
    );
}
