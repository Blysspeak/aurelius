# Changelog

## [v1.4.0] — 2026-04-01

### Added
- **Task management system** — 5 MCP tools (`task_create`, `task_update`, `task_list`, `task_log`, `task_view`) + full CLI (`au task`) (6857f8f)
- New node types: `Task`, `WorkLog`; new relations: `SubtaskOf`, `Blocks`
- Tasks as hub nodes: collect work logs, decisions, problems, solutions via `contains` edges
- Acceptance criteria, priority-based sorting, auto-activation on first log
- `memory_status` shows active tasks; `memory_session` accepts `tasks` param and returns active hints
- `memory_recall` includes tasks in grouped results

### Other
- Verify post-commit hook links to project (31b55e5)
- Clean test commits (6ea8c12)

---

## [1.3.0] — 2026-03-28

### Fixed
- **`memory_session` auto-creates project nodes** — sessions now create their project hub node if it doesn't exist, and link all child nodes (decisions, problems, solutions) to it via `belongs_to` edges. Previously, sessions silently skipped project linking when the project node was missing, leaving the graph fragmented.
- **Project filter includes hub node** — sidebar project filter now includes the project node itself, keeping the graph connected when filtering by project.

### Improved
- **Obsidian-style graph physics** — reworked force simulation: no node pinning after drag (nodes release back into simulation), gentle center force, stronger link forces for cluster cohesion. Drag a node and its neighbors follow naturally.
- **Cleaner graph labels** — only project nodes show labels by default; other nodes reveal labels on hover/select with neighbor highlighting.
- **Softer link styling** — links are subtle by default, brighten on highlight (Obsidian-inspired).
- **Smaller node sizes** — reduced node radii for cleaner visualization at scale.
- **Project navigation in sidebar** — new "Projects" section extracts project names from `[project-name]` label prefix, allows one-click project scoping.

### Removed
- Position persistence (localStorage pinning) — graph recalculates layout each session, matching Obsidian behavior.

---

## [1.0.0] — 2026-03-21

### Added
- **`memory_gc`** — garbage collection: removes duplicate edges, orphaned edges, and duplicate nodes (by content_hash)
- **`memory_status` project filter** — optional `project` parameter to scope decisions, problems, sessions to a specific project
- **`memory_search` since filter** — optional `since` parameter for time-based queries (`today`, `yesterday`, `7d`, `24h`, ISO 8601)
- **Batch BFS** — context traversal uses batch queries (`WHERE id IN (...)`) instead of N+1 per-node queries
- **Relevance-ranked search** — FTS results boosted by `access_count` for frequently accessed nodes
- **V3 migration** — composite indexes: `edges(to_id, relation)`, unique `edges(from_id, to_id, relation)`, `nodes(content_hash)`, `nodes(node_type, created_at)`
- **V4 migration** — rebuilt FTS5 index without `data` column to eliminate JSON key noise in search results
- **Edge deduplication** — `INSERT OR IGNORE` prevents duplicate edges on same `(from_id, to_id, relation)` triple

### Refactored
- **`graph.rs`** (531 lines) → `graph/{crud, search, traverse}.rs` — modular graph operations
- **`handlers.rs`** (594 lines) → `handlers/{crud, session, status}.rs` — modular MCP handlers

### Fixed
- Project-scoped `memory_status` now uses `search_typed` for proper SQL-level type+FTS filtering
- Project-scoped `open_problems` uses `get_unsolved_problems` with label prefix filter
- FTS5 bracket escaping for `[project]` prefix queries
- V3 migration cleans duplicate edges before creating UNIQUE index

### Removed
- Dead code: unused `get_edges` single-node query (replaced by batch version)
- TimeForged sync — evaluated and rejected (time data not useful for AI memory)

---

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
