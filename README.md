# strata

Clean-room, compiler-backed code intelligence in Rust: a tree-sitter index (Python and Rust
today; C/C++ and HTML/CSS/DOM-JS planned) that answers who-defines, who-calls and
what-breaks questions for humans and agents. Independently designed from public descriptions
of code-intelligence tools; it contains no code or assets from any other product.

```
cargo build
./target/debug/strata symbols walk          # find definitions
./target/debug/strata callers walk.walk     # who calls it, with edge states
./target/debug/strata call overview '{}'    # agent tool surface (JSON); see `strata guide`
```

Agent usage lives in [`docs/agents/using-strata.md`](docs/agents/using-strata.md).

## Self-analysis metrics

strata indexes its own source on every milestone and is scored against rust-analyzer's SCIP
index of the same commit (an independent ground truth). `scripts/metrics.py` produces every
number and the chart below; see [Metrics](CLAUDE.md#metrics) for when to run it.

- **Symbol recall**: share of rust-analyzer's fn/method/type definitions that strata lists.
- **Call-site recall**: share of calls to workspace functions where strata recorded an edge.
- **Target recall**: as above, but the edge's resolved/candidate targets include the right function.
- **False external**: calls to workspace functions that strata reported as `external`.
- **Review findings**: items raised by the two-axis code review (standards / spec) plus
  correctness or security notes found along the way. `–` means no review was run for that commit.

<!-- metrics:start -->

![strata self-analysis chart](docs/metrics/chart.svg)

| Run | Rev | What changed | Symbol recall | Call-site recall | Target recall | False external | Unresolved | Review findings (std / spec / correctness) |
| ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | `c46e806` | tools + resolver fixes | 100.0% | 99.5% | 96.2% | 7 | 0 | – |

<!-- metrics:end -->
