# 0002: strata keeps its own edge-state vocabulary

Status: accepted (2026-10-25, spec #59)

Graphify labels edges EXTRACTED, INFERRED or AMBIGUOUS. Strata already records six finer
states: resolved, candidate, external, unresolved, unindexed, and the reserved observed.

Decision: keep strata's vocabulary and add one reserved state, `inferred`, for edges an LLM
produces from non-code input. Do not adopt graphify's three labels. If a consumer needs a
mapping, publish a table; do not change the states.

Why: the states carry information the three labels lose. Candidate (several targets, none
guessed) and unindexed (target skipped) are different answers that graphify would merge.
`inferred` must stay separate from resolved so LLM output can never be read as compiler-
backed fact.

Consequences: static analysis produces resolved, candidate, external, unresolved and
unindexed only. `observed` and `inferred` appear in the vocabulary now so consumers handle
them, but nothing produces them yet.
