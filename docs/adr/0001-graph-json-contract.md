# graph.json is a versioned contract

The graph export is a single JSON document with a `schema_version`, node IDs that stay the same across rebuilds for unchanged symbols, a file and span on every node, an edge state on every edge, and the git commit the index was built from. Vault sync, diagram tools and MCP clients depend on it, and sync-vault deletes any note whose ID has vanished, so ID stability is the guarantee that matters most. Breaking changes bump `schema_version`; consumers are never asked to scrape the individual query commands instead.
