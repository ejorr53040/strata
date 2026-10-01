# strata

Clean-room, compiler-backed code intelligence in Rust: a tree-sitter index (Python, Rust, C/C++, plus HTML/CSS/DOM-JS) served by a per-repo daemon to a CLI, MCP server, LSP server + Neovim plugin, and a headless agent. Independently designed from public descriptions of code-intelligence tools; contains no code or assets from any other product.

## Agent skills

### Issue tracker

GitHub Issues on `ejorr53040/strata` (private until milestone 3), via the `gh` CLI; blocking edges use GitHub's native issue dependencies. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Analyzing code in this repo

strata indexes its own Rust source. For "who calls X", "what breaks if I change Y", or
"where is Z defined", use the CLI instead of grepping: build with `cargo build`, then run
`./target/debug/strata call overview '{}'` and follow `docs/agents/using-strata.md`
(`strata guide` prints the same text). The tool catalog lives in
`crates/strata-core/src/tools.rs`; add a tool there and it is available to the CLI, and
later to MCP and `strata ask`. If strata's answer looks wrong, that is a bug in strata:
write a failing fixture test under `crates/strata-cli/tests/` first.

