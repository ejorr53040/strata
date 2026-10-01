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
            "resolved app.main.main -> shapes.Circle.area app/src/main.rs:6",
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
            "resolved app.main.shadow -> shapes.Circle.area app/src/main.rs:15",
        ]
    );
}

#[test]
fn std_methods_nested_fns_and_constructors_do_not_pollute_the_graph() {
    assert_eq!(
        run(&["callees", "app.main.ext"]),
        vec![
            "external app.main.ext -> Vec.new (builtin) app/src/main.rs:19",
            "external app.main.ext -> v.push (no indexed method `push`; receiver type unknown) app/src/main.rs:21",
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
            "resolved[import] app.main.<module> -> shapes.Circle app/src/main.rs:2",
            "resolved[type] app.main.main -> shapes.Circle app/src/main.rs:5",
            "resolved[type] app.main.shadow -> shapes.Circle app/src/main.rs:14",
            "resolved[type] app.main.field_on_unknown -> shapes.Circle app/src/main.rs:112",
            "resolved[type] app.main.typed -> shapes.Circle app/src/main.rs:193",
            "resolved[type] app.main.extra_user -> shapes.Circle app/src/main.rs:198",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:18",
            "resolved[type] shapes.Circle.new -> shapes.Circle shapes/src/lib.rs:19",
            "resolved[type] shapes.Circle.new -> shapes.Circle shapes/src/lib.rs:20",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:27",
            "resolved[type] shapes.Circle.area -> shapes.Circle shapes/src/lib.rs:29",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:33",
            "resolved[type] shapes.Circle -> shapes.Circle shapes/src/lib.rs:52",
            "resolved[import] shapes.util.<module> -> shapes.Circle shapes/src/util.rs:1",
            "resolved[type] shapes.util.make -> shapes.Circle shapes/src/util.rs:7",
            "resolved[type] shapes.util.make -> shapes.Circle shapes/src/util.rs:8",
            "resolved[type] shapes.util.shapes.Circle -> shapes.Circle shapes/src/util.rs:28",
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
            "resolved[type] app.main.tuple_pat -> app.main.Tup app/src/main.rs:135",
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
        vec![
            "resolved[value] app.main.consts_and_values -> app.main.LIMIT app/src/main.rs:107",
            "resolved[value] app.main.Args2 -> app.main.LIMIT app/src/main.rs:130",
        ]
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
            "resolved[value] app.main.chain -> app.main.double app/src/main.rs:184",
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
            "resolved[field] app.main.field_on_unknown -> shapes.Circle.r app/src/main.rs:113",
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

// Depth round 5: modules and imports.

#[test]
fn modules_are_symbols_and_path_segments_and_uses_are_edges_to_them() {
    let syms = run(&["symbols", ""]);
    for want in [
        "module shapes shapes/src/lib.rs:1",
        "module shapes.util shapes/src/util.rs:1",
        "module app.helper.tests app/src/helper/tests.rs:1",
    ] {
        assert!(syms.contains(&want.to_string()), "missing {want}");
    }
    assert_eq!(
        refs("shapes.util"),
        vec![
            "resolved[import] app.main.<module> -> shapes.util app/src/main.rs:1",
            "resolved[import] app.main.<module> -> shapes.util app/src/main.rs:11",
            "resolved[path] app.main.path_beats_local -> shapes.util app/src/main.rs:204",
            "resolved[path] shapes.<module> -> shapes.util shapes/src/lib.rs:1",
            "resolved[path] shapes.Circle.area -> shapes.util shapes/src/lib.rs:29",
            "resolved[import] shapes.<module> -> shapes.util shapes/src/lib.rs:50",
            "resolved[path] shapes.via_inline -> shapes.util shapes/src/lib.rs:115",
            "resolved[import] shapes.util.inline_tests -> shapes.util shapes/src/util.rs:17",
            "resolved[import] shapes.util.cyc -> shapes.util shapes/src/util.rs:40",
        ]
    );
}

#[test]
fn imports_follow_reexports_to_the_definition() {
    assert_eq!(
        refs("shapes.util.make"),
        vec![
            "resolved[import] app.main.<module> -> shapes.util.make app/src/main.rs:28",
            "resolved app.main.reexport -> shapes.util.make app/src/main.rs:31",
            "resolved[import] shapes.<module> -> shapes.util.make shapes/src/lib.rs:50",
        ]
    );
}

#[test]
fn super_inside_an_inline_module_is_the_enclosing_file_module() {
    let lines = refs("shapes.util");
    assert!(
        lines.contains(
            &"resolved[import] shapes.util.inline_tests -> shapes.util shapes/src/util.rs:17"
                .to_string()
        ),
        "{lines:#?}"
    );
}

#[test]
fn pub_crate_visibility_names_the_crate_root() {
    let lines = refs("shapes");
    assert!(
        lines.contains(
            &"resolved[path] shapes.util.hidden -> shapes shapes/src/util.rs:13".to_string()
        ),
        "{lines:#?}"
    );
}

// Depth round 5c: namespaces and scopes found by running on cirrocumulus.

#[test]
fn a_field_and_a_method_of_the_same_name_are_both_referenced() {
    let lines = refs("shapes.Gauge.level");
    assert!(
        lines.contains(
            &"resolved[field] shapes.Gauge.level -> shapes.Gauge.level shapes/src/lib.rs:106"
                .to_string()
        ),
        "{lines:#?}"
    );
    assert!(
        lines.contains(
            &"resolved shapes.read -> shapes.Gauge.level shapes/src/lib.rs:111".to_string()
        ),
        "{lines:#?}"
    );
}

#[test]
fn items_defined_inside_a_function_are_found_from_it() {
    assert_eq!(
        refs("app.main.local_items.COUNTER"),
        vec!["resolved[value] app.main.local_items -> app.main.local_items.COUNTER app/src/main.rs:126"]
    );
}

#[test]
fn inline_modules_are_path_references_not_values() {
    let lines = refs("shapes.util.inline_tests");
    assert!(
        lines.contains(
            &"resolved[path] shapes.via_inline -> shapes.util.inline_tests shapes/src/lib.rs:115"
                .to_string()
        ),
        "{lines:#?}"
    );
}

#[test]
fn attribute_arguments_may_hold_arbitrary_paths_and_tuple_struct_patterns_are_types() {
    assert!(refs("app.main.LIMIT").contains(
        &"resolved[value] app.main.Args2 -> app.main.LIMIT app/src/main.rs:130".to_string()
    ));
    assert!(refs("app.main.Tup").contains(
        &"resolved[type] app.main.tuple_pat -> app.main.Tup app/src/main.rs:135".to_string()
    ));
}

// Depth round 6: the long tail found on cirrocumulus.

#[test]
fn shorthand_pattern_bindings_are_locals_even_when_a_function_has_their_name() {
    assert!(refs("shapes.Gauge.level").contains(
        &"candidate[field] app.main.shorthand_local -> shapes.Gauge.level app/src/main.rs:146"
            .to_string()
    ));
}

#[test]
fn field_accesses_inside_unparseable_macros_are_found() {
    assert!(refs("app.main.Cfg2.n_vcpus").contains(
        &"candidate[field] app.main.json_field -> app.main.Cfg2.n_vcpus app/src/main.rs:155"
            .to_string()
    ));
}

#[test]
fn items_inside_inline_modules_resolve_from_that_module() {
    assert_eq!(
        refs("shapes.util.inline_tests.K"),
        vec!["resolved[value] shapes.util.inline_tests.t -> shapes.util.inline_tests.K shapes/src/util.rs:22"]
    );
}

#[test]
fn visibility_on_modules_and_types_in_item_macros_are_references() {
    assert!(refs("shapes").contains(
        &"resolved[path] shapes.util.<module> -> shapes shapes/src/util.rs:26".to_string()
    ));
    assert!(refs("shapes.Gauge").contains(
        &"resolved[type] shapes.<module> -> shapes.Gauge shapes/src/lib.rs:118".to_string()
    ));
}

#[test]
fn nothing_in_the_fixture_is_left_unresolved() {
    let v: Vec<serde_json::Value> =
        serde_json::from_str(&run(&["call", "edges", "{}"]).join("")).unwrap();
    let bad: Vec<String> = v
        .iter()
        // `cyc::f` calls a name that is deliberately defined nowhere (the loop-guard test).
        .filter(|e| {
            e["state"] == "unresolved"
                && e["callee"] != "not_defined_here"
                && e["callee"] != "util.inline_tests.nope"
                && e["callee"] != "inline_tests.nope"
        })
        .map(|e| format!("{} {} {} {}", e["kind"], e["path"], e["line"], e["callee"]))
        .collect();
    assert!(bad.is_empty(), "unresolved edges: {bad:#?}");
}

#[test]
fn receiver_chains_of_turbofish_calls_are_not_skipped() {
    assert!(refs("app.main.double").contains(
        &"resolved[value] app.main.chain -> app.main.double app/src/main.rs:184".to_string()
    ));
}

#[test]
fn path_attribute_points_a_mod_declaration_at_another_file() {
    assert_eq!(
        run(&["callees", "app.tests.it.it3"]),
        vec!["resolved app.tests.it.it3 -> app.helper.go app/tests/it.rs:17"]
    );
    assert!(run(&["symbols", "helper_alias"])
        .contains(&"module helper_alias app/tests/it.rs:14".to_string()));
}

// Depth round 7: trait implementations.

fn impl_edges() -> Vec<serde_json::Value> {
    let v: Vec<serde_json::Value> =
        serde_json::from_str(&run(&["call", "edges", "{}"]).join("")).unwrap();
    v.into_iter().filter(|e| e["kind"] == "impl").collect()
}

#[test]
fn trait_impls_are_edges_from_the_type_and_from_each_method_to_the_trait() {
    let lines = refs("shapes.Shape");
    assert!(
        lines.contains(
            &"resolved[impl] shapes.Circle -> shapes.Shape shapes/src/lib.rs:27".to_string()
        ),
        "{lines:#?}"
    );
    let lines = refs("shapes.Shape.area");
    assert!(
        lines.contains(
            &"resolved[impl] shapes.Circle.area -> shapes.Shape.area shapes/src/lib.rs:28"
                .to_string()
        ),
        "{lines:#?}"
    );
}

#[test]
fn impls_of_traits_outside_the_repo_are_kept_as_external_edges() {
    let e = impl_edges();
    let drop = e
        .iter()
        .find(|e| e["caller"] == "app.main.Guard2.drop")
        .expect("Guard2.drop implements Drop");
    assert_eq!(drop["state"], "external");
    assert_eq!(drop["callee"], "Drop.drop");
}

// Depth round 8: receiver types make method calls exact.

#[test]
fn receiver_types_from_params_fields_and_constructors_make_calls_exact() {
    assert_eq!(
        run(&["callees", "app.main.typed"]),
        vec![
            "resolved app.main.typed -> shapes.Gauge.level app/src/main.rs:192",
            "resolved app.main.typed -> shapes.Circle.new app/src/main.rs:193",
            "resolved app.main.typed -> shapes.Circle.area app/src/main.rs:194",
            "resolved app.main.typed -> shapes.Gauge.level app/src/main.rs:195",
        ]
    );
}

#[test]
fn methods_of_an_impl_in_another_module_belong_to_the_type() {
    assert_eq!(
        run(&["callees", "app.main.extra_user"]),
        vec!["resolved app.main.extra_user -> shapes.Circle.extra app/src/main.rs:199"]
    );
}

#[test]
fn a_path_call_names_the_module_even_when_a_local_has_the_same_name() {
    assert_eq!(
        run(&["callees", "app.main.path_beats_local"]),
        vec!["resolved app.main.path_beats_local -> shapes.util.square app/src/main.rs:204"]
    );
}

#[test]
fn path_segments_in_patterns_are_not_local_bindings() {
    assert!(refs("app.main.pick2").contains(
        &"resolved[value] app.main.pattern_path -> app.main.pick2 app/src/main.rs:214".to_string()
    ));
}

#[test]
fn an_import_that_points_back_at_its_own_module_does_not_loop() {
    // `use super::X` inside an inline module where X is not defined (e.g. made by a macro)
    // names the module itself; resolving it must terminate and leave the call unresolved.
    let lines = run(&["callees", "shapes.util.cyc.f"]);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("unresolved shapes.util.cyc.f -> not_defined_here"),
        "{lines:?}"
    );
}

#[test]
fn a_missing_name_inside_an_inline_module_is_unresolved_not_a_crash() {
    let lines = run(&["callees", "shapes.util.calls_inline"]);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("unresolved shapes.util.calls_inline"),
        "{lines:?}"
    );
}
