# tesselate

Clean-room, compiler-backed code intelligence in Rust: a tree-sitter index (Rust, Python, JavaScript, plus HTML/CSS and the DOM lookups JS makes) served by a per-repo daemon to a CLI. Planned, not built: C/C++, an MCP server, an LSP server + Neovim plugin, and a headless agent (`tesselate ask`). Independently designed from public descriptions of code-intelligence tools; contains no code or assets from any other product.

## Agent skills

### Issue tracker

GitHub Issues on `ejorr53040/tesselate` (private until milestone 3), via the `gh` CLI; blocking edges use GitHub's native issue dependencies. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Analyzing code in this repo

tesselate indexes its own Rust source. For "who calls X", "what breaks if I change Y", or
"where is Z defined", use the CLI instead of grepping: build with `cargo build`, then run
`./target/debug/tesselate call overview '{}'` and follow `docs/agents/using-tesselate.md`
(`tesselate guide` prints the same text). The tool catalog lives in
`crates/tesselate-core/src/tools.rs`; add a tool there and it is available to the CLI, and
later to MCP and `tesselate ask`. If tesselate's answer looks wrong, that is a bug in tesselate:
write a failing fixture test under `crates/tesselate-cli/tests/` first.


## Metrics

tesselate scores itself against rust-analyzer's SCIP index and tracks the numbers over time:
`scripts/metrics.py`, history in `docs/metrics/history.jsonl`, chart `docs/metrics/chart.svg`,
table and chart embedded in `README.md`. **Keep them current as part of the work, not as an
afterthought:**

1. After any change to the resolver, a language plugin, the walker or the tool catalog, and
   after every `code-review`, commit the change first (runs measure a clean checkout of a
   commit, never uncommitted edits), then run
   `python3 scripts/metrics.py run --label "<what changed>"`. For a review, add
   `--review standards=N,spec=N,correctness=N` with that review's finding counts.
2. Commit `docs/metrics/` and `README.md` in a follow-up commit ("metrics: <rev>"). Never
   hand-edit them; `python3 scripts/metrics.py render` redraws from history.
3. Read the new row against the previous one. Recall must not drop; if it does, that is the
   next bug. `-v` lists every missed call and definition. A miss tesselate gets wrong is a
   tesselate bug: add a failing fixture test under `crates/tesselate-cli/tests/` first, then fix.
   A miss that is the scorer's fault gets fixed in `metrics.py` and noted in the commit.
4. Check `map.qa` and `map.tokens` too: the map must stay answerable (QA where/fields/callers) and
   within budget. Dogfood on a big real crate (e.g. tokio in ~/.cargo/registry) now and then:
   crashes and blow-ups show up there first.
5. Report each run's deltas (recall, false external, unresolved, review findings) to the user.

A second series scores tesselate on an external repo (cirrocumulus, `~/cs/cirrocumulus`; override
with `CIRRO_PATH`): `python3 scripts/metrics.py run --corpus cirrocumulus --label "..."`
(history `docs/metrics/history-cirrocumulus.jsonl`, chart `chart-cirrocumulus.svg`). Run it
whenever the self series is run, and whenever cirrocumulus gains commits worth re-scoring. It
exports the corpus with `git archive` and never runs rust-analyzer inside that repo (its
`build.rs` nests a cargo build into its own `target/`). New misses there are tesselate bugs: add a
fixture case to `crates/tesselate-cli/tests/fixtures/rust_repo`, see it fail, then fix.

The numbers are Rust-only; `scripts/` and test fixtures are in `.tesselateignore` so they do not
distort the self-index. The runs table is a series: do not delete rows or rewrite old ones.
