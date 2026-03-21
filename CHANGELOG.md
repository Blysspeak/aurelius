# Changelog

## [0.5.0] — 2026-03-21

### Optimized
- **`memory_status`** — uses SQL LIMIT instead of fetching all nodes and truncating in Rust; 6x fewer rows deserialized
- **`get_unsolved_problems`** — parameterized node types (no hardcoded JSON strings), added LIMIT
- **`memory_session`** — deduplication via SHA-256 content_hash; duplicate calls return existing session instead of creating duplicates
- **`memory_session`** — removed double storage: decisions/problems no longer stored in Session node's `data` JSON (they're already separate graph nodes)

### Added
- `find_node_by_content_hash()` — lookup nodes by content hash for dedup

---

## [0.4.0] — 2026-03-21

### Added
- **`memory_recall`** — smart topic recall: combines FTS search with BFS traversal, returns results grouped by type (decisions, problems, solutions, sessions, other). One call instead of separate search+context
- **`memory_search` type filter** — optional `type` parameter to filter results by node type (e.g. `type: "decision"`)
- **`get_unsolved_problems()`** — SQL query that finds problems without a linked solution (via `solves` edge)
- **`search_typed()`** — FTS search with node type filter in core

### Improved
- **`memory_status`** — `open_problems` now shows only unsolved problems (those without a `solves` edge from a Solution node), not all problems
- **Web UI** — graph physics now always active (`cooldownTicks=Infinity`, `d3AlphaMin=0`), Obsidian-like behavior

### Fixed
- Graph visualization froze after 5-10 seconds due to d3-force simulation cooling down

---

## [0.3.0] — 2026-03-21

### Added
- **`memory_session`** — record session summaries with decisions, problems solved, and next steps; creates episodic Session node linked to project, plus Decision and Problem/Solution nodes with proper graph relations
- **`memory_update`** — update existing node's note and/or data by UUID or label; enables enriching nodes with additional context after creation
- **`memory_add` enhanced** — now accepts `data` (arbitrary JSON metadata) and `memory_kind` (semantic/episodic) parameters

### Improved
- **`memory_status`** — now returns recent solutions alongside problems, session details with full node info (not just brief), and uses lightweight count queries for stats
- **`memory_add`** — uses `add_node_full` internally, supporting all node fields

---

## [0.2.0] — 2026-03-21

### Improved
- **`memory_search`** — empty query (`""`) or wildcard (`"*"`) now returns most recent nodes instead of FTS5 error
- **`memory_dump`** — added pagination with `offset` and `limit` parameters (default: 50 items) to prevent exceeding MCP token limits; response includes `total_nodes`/`total_edges` counts for navigation

### Added
- `get_recent_nodes()` — fetch N most recent nodes by creation date
- `get_nodes_paginated()` / `get_edges_paginated()` — paginated graph queries
- `count_nodes()` / `count_edges()` — lightweight count queries

---

## [0.1.0] — 2026-03-21

### Added
- **Knowledge Graph Core** — SQLite-backed graph with FTS5 full-text search, WAL mode, versioned migrations
- **Domain Model** — 14 node types (Project, Crate, File, Decision, Concept, Problem, Solution, etc.), 16 relation types, MemoryKind (Semantic/Episodic)
- **Graph Operations** — add/delete/update nodes, BFS traversal, FTS search, touch (access tracking), find by label/data field
- **Project Indexer** — parses Cargo.toml workspaces, discovers crates, files, dependencies; SHA256 content hashing for incremental re-index
- **TimeForged Connector** — async integration with TimeForged time tracking daemon; pulls sessions, projects, languages into the graph
- **MCP Server** — JSON-RPC 2.0 over stdio, 8 tools: `memory_status`, `memory_context`, `memory_search`, `memory_add`, `memory_relate`, `memory_index`, `memory_forget`, `memory_dump`
- **CLI (`au`)** — 9 subcommands: `init`, `note`, `context`, `search`, `sync`, `reindex`, `view`, `export`, `mcp`, `touch`
- **Web UI** — React + TypeScript + Tailwind CSS + react-force-graph-2d; interactive graph visualization with Obsidian-style physics, sidebar filters, node detail panel, search
- **Claude Code Integration** — MCP server config, PostToolUse hook (tracks file access), Stop hook (auto re-index on session end), git post-commit hook (captures decisions)
- **Install script** — `install.sh` for one-command setup: build, install, configure hooks
