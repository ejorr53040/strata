# strata

Clean-room, compiler-backed code intelligence in Rust: a tree-sitter index (Python, Rust, C/C++, plus HTML/CSS/DOM-JS) served by a per-repo daemon to a CLI, MCP server, LSP server + Neovim plugin, and a headless agent. Independently designed from public descriptions of code-intelligence tools; contains no code or assets from any other product.

## Agent skills

### Issue tracker

GitHub Issues on `ejorr53040/strata` (private until milestone 3), via the `gh` CLI; blocking edges use GitHub's native issue dependencies. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
