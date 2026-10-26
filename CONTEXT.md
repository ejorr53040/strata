# strata

Code intelligence for humans and agents: a tree-sitter index of a repo that answers
who-defines, who-calls and what-breaks questions, and says how sure it is.

## Language

**Snapshot**:
One in-memory index of a repo, built once and queried many times. It is stale when the size
or mtime of any indexed file differs from what it recorded.
_Avoid_: cache, database

**Daemon**:
The per-repo process that holds a snapshot and answers queries from the thin client over a
unix socket. It starts on the first query and exits after an idle timeout (default 600s).
_Avoid_: server, service

**Edge**:
A recorded connection between two symbols, such as a call site. Every edge has an edge state.

**Edge state**:
How confidently an edge was tied to its target. One of the six below. It is the answer to
"can I trust this edge?", and strata never hides it.
_Avoid_: confidence, label

**Resolved**:
Exactly one known target.

**Candidate**:
Several possible targets. Every one is listed; none is guessed.

**External**:
The target lives outside the indexed repo. The import is given as evidence.

**Unresolved**:
Could not be tied to anything. A reason says why.

**Unindexed**:
The target lives in code that indexing skipped.

**Observed**:
Seen at runtime by a tracer. Reserved: static analysis never produces it.

**Inferred**:
Produced by an LLM reading non-code input (docs, images, PDFs). Reserved: nothing produces
it yet. It is never merged with resolved.

**Tool catalog**:
The set of named, JSON-in JSON-out queries (`strata call <tool>`). The CLI and, later, MCP
are thin views of it, so a capability is added once, in the catalog.

**Symbol**:
A definition the index knows by name and location (function, type, element, rule).

## Relationships

- A **daemon** holds one **snapshot** per repo.
- A **snapshot** contains symbols and edges; each **edge** has exactly one **edge state**.
- Every consumer (CLI, vault sync, diagram tools, MCP) reads the **tool catalog**.

## Flagged ambiguities

- "Confidence" is not an edge state: states are categories, not scores. Do not rank them.
