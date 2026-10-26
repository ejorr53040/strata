# strata is renamed tesselate

The project was called strata, a name several large products in programming already use. It is renamed `tesselate`, with `tssl` as a short alias binary and `tesselate-daemon` as the daemon, before the `graph.json` contract, the MCP tool names and the public release bake the name in. The old name is gone everywhere with no compatibility shims: `.strataignore`, `STRATA_*` variables, the `strata` config directory and the `strata` runtime socket directory have no effect, so a missed migration is noticed and not silently honoured.

`trail` and `trailmark` were rejected because Trail-code and Trailmark are code-intelligence tools for coding agents. The name is spelled with one l on purpose: the dictionary spelling is taken on crates.io and PyPI, and the one-l spelling is unique in searches. `tess` is not used as an alias because another CLI already uses it.

A guard test fails if the old name appears anywhere in the tree outside this file and the append-only metrics series, whose rows keep the name they were recorded under.
