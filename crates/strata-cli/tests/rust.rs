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

#[test]
fn dogfood_round_two_crate_root_mods_and_match_guards() {
    // `mod helper;` in main.rs is a sibling file src/helper.rs, not src/main/helper.rs.
    assert_eq!(
        run(&["callees", "app.main.uses_helper"]),
        vec!["resolved app.main.uses_helper -> app.helper.go app/src/main.rs:46"]
    );
    // Same for an integration-test crate root with a shared `tests/common/mod.rs`.
    assert_eq!(
        run(&["callees", "app.tests.it.it"]),
        vec!["resolved app.tests.it.it -> app.tests.common.setup app/tests/it.rs:4"]
    );
    assert_eq!(
        run(&["callees", "app.tests.it.it2"]),
        vec!["resolved app.tests.it.it2 -> app.tests.common.setup app/tests/it.rs:10"]
    );
    // A call inside a match guard is a call, not a pattern binding.
    assert_eq!(
        run(&["callees", "app.main.guard"]),
        vec!["resolved app.main.guard -> app.main.helper_fn app/src/main.rs:51"]
    );
}

// Gaps found by running strata on cirrocumulus.

#[test]
fn let_binding_does_not_hide_the_function_it_is_initialised_from() {
    assert_eq!(
        run(&["callees", "app.main.shadow_call"]),
        vec!["resolved app.main.shadow_call -> app.main.pick app/src/main.rs:65"]
    );
}

#[test]
fn glob_imports_of_the_parent_module_resolve_in_test_files() {
    assert_eq!(
        run(&["callees", "app.helper.tests.t"]),
        vec!["resolved app.helper.tests.t -> app.helper.go app/src/helper/tests.rs:4"]
    );
}

#[test]
fn calls_inside_json_style_macros_are_found() {
    assert_eq!(
        run(&["callees", "app.main.json_like"]),
        vec!["resolved app.main.json_like -> app.main.helper_fn app/src/main.rs:71"]
    );
}

#[test]
fn derived_methods_on_self_are_external_not_unresolved() {
    assert_eq!(
        run(&["callees", "shapes.Marker.dup"]),
        vec!["external shapes.Marker.dup -> self.clone (no indexed method `clone` on `shapes.Marker`; derived or trait method) shapes/src/lib.rs:67"]
    );
}

#[test]
fn turbofish_paths_are_ordinary_paths() {
    assert_eq!(
        run(&["callees", "app.main.turbofish"]),
        vec!["external app.main.turbofish -> Vec.new (builtin) app/src/main.rs:77"]
    );
}

#[test]
fn type_aliases_are_symbols() {
    assert!(run(&["symbols", "Radius"]).contains(&"type Radius shapes/src/lib.rs:60".to_string()));
}

#[test]
fn functions_generated_by_ioctl_macros_are_defined() {
    assert!(run(&["symbols", "tun_none"])
        .contains(&"function tun_none shapes/src/lib.rs:71".to_string()));
    assert_eq!(
        run(&["callees", "shapes.use_ioctl"]),
        vec!["resolved shapes.use_ioctl -> shapes.tun_none shapes/src/lib.rs:74"]
    );
}

// Depth round 1: type references.

fn refs(sym: &str) -> Vec<String> {
    run(&["refs", sym])
}

#[test]
fn type_references_are_edges_with_their_enclosing_item_as_the_source() {
    assert_eq!(
        refs("shapes.Circle"),
        vec![
            "resolved[type] app.main.main -> shapes.Circle app/src/main.rs:5",
            "resolved[type] app.main.shadow -> shapes.Circle app/src/main.rs:14",
            "resolved[type] app.main.field_on_unknown -> shapes.Circle app/src/main.rs:112",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:18",
            "resolved[type] shapes.Circle.new -> shapes.Circle shapes/src/lib.rs:19",
            "resolved[type] shapes.Circle.new -> shapes.Circle shapes/src/lib.rs:20",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:27",
            "resolved[type] shapes.Circle.area -> shapes.Circle shapes/src/lib.rs:29",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:33",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:52",
            "resolved[type] shapes.util.make -> shapes.Circle shapes/src/util.rs:7",
            "resolved[type] shapes.util.make -> shapes.Circle shapes/src/util.rs:8",
        ]
    );
}

#[test]
fn generic_parameters_are_not_type_references_but_their_bounds_are() {
    let lines = refs("shapes.Shape");
    assert!(
        lines.contains(
            &"resolved[type] shapes.ident -> shapes.Shape shapes/src/lib.rs:77".to_string()
        ),
        "{lines:#?}"
    );
    let all = run(&["call", "edges", "{}"]).join("\n");
    assert!(
        !all.contains(r#""callee":"T""#),
        "generic param T leaked as a type reference"
    );
}

#[test]
fn type_references_in_constructors_turbofish_and_macros() {
    assert_eq!(
        refs("app.main.Tup"),
        vec![
            "resolved[type] app.main.types_in_odd_places -> app.main.Tup app/src/main.rs:85",
            "resolved[type] app.main.types_in_odd_places -> app.main.Tup app/src/main.rs:86",
            "resolved[type] app.main.types_in_odd_places -> app.main.Tup app/src/main.rs:87",
            "resolved[type] app.main.types_in_odd_places -> app.main.Tup app/src/main.rs:89",
        ]
    );
}

// Depth round 2: functions and constants as values.

#[test]
fn consts_and_statics_are_symbols_and_their_uses_are_value_references() {
    let syms = run(&["symbols", ""]);
    assert!(
        syms.contains(&"const LIMIT app/src/main.rs:94".to_string()),
        "{syms:#?}"
    );
    assert!(syms.contains(&"static NAME app/src/main.rs:95".to_string()));
    assert_eq!(
        refs("app.main.LIMIT"),
        vec!["resolved[value] app.main.consts_and_values -> app.main.LIMIT app/src/main.rs:107"]
    );
    // A method call on a static is a call on a value of unknown type, not an unresolved name.
    assert!(run(&["callees", "app.main.consts_and_values"])
        .iter()
        .all(|l| !l.starts_with("unresolved")));
}

#[test]
fn functions_used_as_values_and_in_attributes_are_references() {
    assert_eq!(
        refs("app.main.double"),
        vec![
            "resolved[value] app.main.Args -> app.main.double app/src/main.rs:102",
            "resolved[value] app.main.consts_and_values -> app.main.double app/src/main.rs:109",
        ]
    );
}

// Depth round 3: fields (and the variants that own some).

#[test]
fn fields_and_variants_are_symbols() {
    let syms = run(&["symbols", ""]);
    for want in [
        "field Circle.r shapes/src/lib.rs:4",
        "variant Kind.Round shapes/src/lib.rs:8",
        "variant Shape2.Sq shapes/src/lib.rs:82",
        "field Shape2.Sq.side shapes/src/lib.rs:82",
    ] {
        assert!(syms.contains(&want.to_string()), "missing {want}");
    }
}

#[test]
fn field_accesses_initialisers_and_unknown_receivers_are_field_references() {
    assert_eq!(
        refs("shapes.Circle.r"),
        vec![
            "candidate[field] app.main.field_on_unknown -> shapes.Circle.r app/src/main.rs:113",
            "resolved[field] shapes.Circle.new -> shapes.Circle.r shapes/src/lib.rs:20",
            "resolved[field] shapes.Circle.area -> shapes.Circle.r shapes/src/lib.rs:29",
        ]
    );
    assert_eq!(
        refs("shapes.Shape2.Sq.side"),
        vec!["resolved[field] shapes.make_sq -> shapes.Shape2.Sq.side shapes/src/lib.rs:87"]
    );
}

// Depth round 4: enum variants named in expressions and patterns.

#[test]
fn variant_paths_and_imported_bare_variants_are_variant_references() {
    assert_eq!(
        refs("shapes.Kind.Round"),
        vec![
            "resolved[variant] app.main.bare_variant -> shapes.Kind.Round app/src/main.rs:120",
            "resolved[variant] shapes.kind_ref -> shapes.Kind.Round shapes/src/lib.rs:91",
            "resolved[variant] shapes.kind_match -> shapes.Kind.Round shapes/src/lib.rs:96",
        ]
    );
}
