# Using strata to analyze code

strata is a compiler-backed code index (Python and Rust today). Use it instead of grepping
when you need to know what defines, calls or depends on something. Run it from the repo
root; every tool returns JSON.

```
strata tools                          # catalog: name, description, input_schema
strata call <tool> '<json-args>'      # run one tool
strata guide                          # print this document
```

## Workflow

1. `strata call overview '{}'` first. Read the edge-state counts: a graph that is mostly
   `unresolved` cannot answer impact questions; say so instead of guessing.
2. `strata call symbols '{"query":"walk"}'` to find the exact qualified id
   (`strata_core.walk.walk`). Rust modules are `<crate_dir>.<module path>`, methods are
   `<module>.<Type>.<method>`.
3. `callers` before changing a signature or deleting something; `callees` to see what a
   function depends on.
4. If something you expect is missing, `unindexed_files` says why (gitignored, vendored,
   unsupported language). Override with `index.include` in `.strata/config.toml`.

## Reading edge states

| state | meaning | what to do |
| --- | --- | --- |
| `resolved` | exactly one target, proven from imports/definitions | trust it |
| `candidate` | several possible targets; all are listed | treat every target as affected |
| `external` | outside the repo (std, third-party), with evidence | ignore for impact |
| `unresolved` | could not resolve; `reason` says why | read the code yourself |
| `unindexed` | target module exists but was skipped | check `unindexed_files` |
| `observed` | seen at runtime (reserved for the tracer) | not produced yet |

`candidate` is never a guess: it is the full bounded set. Method calls on a receiver of
unknown type (`x.run()`) list every indexed method named `run`. Standard-library methods
(`vec.push()`) show up as `unresolved`; that is expected, not a bug.

## Limits

- Rust macros are re-parsed as argument lists, so calls inside `format!(..)` are found but
  macro-generated items are not. `use a::*` imports are not followed.
- `external` is deliberate for Rust members no indexed definition has: std/dependency
  methods, and `Type::default()` / `Type::parse()` where the member is derived. If the
  member is defined by an `impl` in a different module than the type, it is indexed under
  the impl's module and a `Type::member` call may wrongly read `external`.
- Calls through local closures or fn-pointer parameters, and enum-variant / tuple-struct
  constructors, are not recorded as call edges.
- Trait impls in a different file from the type are not linked for `self.method()` dispatch.
- Add a `.strataignore` (gitignore syntax) to keep fixtures and generated code out of the
  graph.
