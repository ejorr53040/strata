# 0001: `graph.json` is a versioned contract

Status: accepted (2026-10-25, spec #59)

Strata will export its whole graph as one JSON document. Other tools depend on it: the
`sync-vault` Obsidian sync, diagram tools such as archify, and MCP clients.

Decision: the export guarantees
- a `schema_version`, bumped on any breaking change;
- node IDs that stay the same across rebuilds when the symbol is unchanged;
- `file` and `span` on every node;
- an edge state on every edge;
- the git commit the index was built from.

Why: consumers diff exports and delete outputs whose ID disappears (`sync-vault` removes the
note for a gone ID). Unstable IDs would make every rebuild look like a total rewrite, and an
unversioned shape would break consumers silently.

Consequences: changing node-ID derivation or the document shape is a breaking change and
needs a new `schema_version`. The graph is the single source for exports; later exports
(GraphML, HTML view) build on it.
