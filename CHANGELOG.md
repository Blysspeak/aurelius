# Changelog

## [v1.8.0] — 2026-07-29

### Added
- `au db check [PATH]` — the check now takes an optional path, so a snapshot can be verified. A snapshot is an ordinary database, so verifying one is the same command pointed at a different file (a22d373)
- The rolling backup hook verifies every snapshot it writes. An unverified backup is a guess. A snapshot that fails the check is renamed to `.FAILED-CHECK` — out of the `aurelius-*.db` pattern, so it can never be mistaken for a good backup nor counted by retention — and kept rather than deleted, because a bad snapshot is evidence worth reading (a22d373)

### Fixed
- `au db backup` reports a missing database instead of failing inside the snapshot call (a22d373)

---

## [v1.7.0] — 2026-07-29

### Fixed
- **A damaged database is refused instead of silently rewritten.** Every `db::open` now checks the file's own 100-byte header against its size and refuses an image whose header describes less than the file holds — the exact fingerprint of a file-level copy over a live WAL database. The error names the file, the finding, the two commands to run, and the rule that caused the damage, instead of the bare `database disk image is malformed` (d781fe2)
- **A failed read is no longer mistaken for an empty database.** `get_schema_version(...).unwrap_or(0)` turned `SQLITE_BUSY` and `SQLITE_CORRUPT` into "brand-new database" and re-ran the destructive migration chain over live data on every single invocation. Version reads now propagate their errors; zero means only that the `schema_version` table is absent (d781fe2)
- **Migrations are atomic.** The whole chain runs in one `BEGIN IMMEDIATE` transaction with the version re-read inside it, so a failure mid-`migrate_v4` can no longer leave the FTS index dropped, its triggers gone and the version advanced. Concurrent processes block instead of racing — 8 simultaneous opens of a fresh database used to fail with `UNIQUE constraint failed: schema_version.version` (d781fe2)
- **Concurrent access waits instead of failing.** Every connection sets `busy_timeout` before anything can take a lock — a hook spawns a writer on every file edit, and several MCP servers run at once, so contention is the norm rather than the exception (d781fe2)
- **WAL mode is verified, not assumed.** `PRAGMA journal_mode` reports a refused switch as a result row rather than an error, and `execute_batch` discarded that row — a connection could silently run in rollback-journal mode. The mode is now read back and checked, with bounded retries for the brief exclusive lock a fresh database needs (d781fe2)
- Databases written by a newer binary are refused instead of being written to under an older understanding of the schema (d781fe2)
- Failed edge writes propagate instead of being discarded at 20 call sites; `touch_node` and `ensure_indexed` stay best-effort — an access counter must never fail a read — but are logged rather than silently dropped (d781fe2)
- `migrate_v2` detects existing columns structurally via `pragma_table_info` instead of matching the English text of an error message (d781fe2)
- The database path had three divergent definitions that disagreed on their fallback; on a machine without a data directory the CLI and the MCP server would have used different files. Now resolved in one place (d781fe2)

### Added
- `au db backup [--out PATH]` — safe snapshot of a live database via SQLite's own `VACUUM INTO`, including data still sitting in an un-checkpointed `-wal`. **Copying `aurelius.db` with `cp`/`mv`/`rsync` while `au` or an MCP server is running is what corrupts it** — use this instead (d781fe2)
- `au db check [--full]` — read-only integrity report that never migrates and never writes a page. Exits non-zero when damaged, so it can gate a script or a hook (d781fe2)
- Skills subsystem — 4 MCP tools (`skill_save`, `skill_list`, `skill_get`, `skill_remove`), `au skills`, and session auto-injection via a SessionStart hook. Released as v1.6.0 but never merged to the default branch; folded in here (78dad01)
- First automated tests in the workspace: concurrent open, migration rollback, corruption refusal, newer-schema refusal, backup round-trip through an un-checkpointed WAL, fresh-open idempotence. Each was observed failing against the previous code (d781fe2)

### Documentation
- Spec-kit feature `002-db-durability-hardening` — specification, plan, research, data model, CLI contract, quickstart, task list — and the project constitution the plan is gated against (8d799ce)
- README: backup section with the reason file-level copying destroys a WAL database, and the manual restore procedure (d781fe2)

### Notes
- **v1.6.0 is contained in this release.** Its tag pointed at a commit that never reached the default branch, so the repository read as 1.5.0 while the installed binary reported 1.6.0. This release supersedes it.
- Verified against the preserved database from the 2026-07-27 incident: 22 consecutive operations, zero bytes changed, and both the refusal and the report name the file-level-copy signature in plain words.

---

## [v1.6.0] — 2026-06-21

### Added
- **Skills subsystem** — reusable procedural "how-to" cards with progressive disclosure. 4 MCP tools:
  - `skill_save` — create/update a skill (upsert by name). Trigger → FTS-indexed note (discoverable); body + tags → `data` (not keyword-indexed, so the body never pollutes search).
  - `skill_list` — cheap index (name + trigger + tags + uses), ranked by usage. Optional FTS `query` / `tag` filter.
  - `skill_get` — full markdown body by name, with fuzzy FTS fallback + `other_matches` for disambiguation. Bumps `access_count`.
  - `skill_remove` — delete a skill by name.
- New node type `Skill`.
- Skills surface automatically in `memory_recall` (new `skills` bucket) and `memory_status` (top skills by usage).
- **SessionStart hook** (`aurelius-skills.sh` / `au skills --hook`) injects the skill index into context every session via `hookSpecificOutput.additionalContext`.

### Changed
- CLI: new `au skills [--hook]` command.

---

## [v1.5.0] — 2026-04-19

### Added
- `memory_merge` — merge two duplicate/related nodes into one: rewires all edges from source to target, removes self-loops and duplicate edges, appends source's note to target, deletes source. CLI: `au merge <source> <target>` (909b06d)
- `task_stats` — analytics over tasks: counts by status/priority, completion rate, avg/median active→done duration, currently blocked count, oldest active age, done-in-window. CLI: `au task stats [--project] [--since-days]` (909b06d)

### Documentation
- Update tool count 19→21 in README and CLAUDE.md (238792c)

---

## [v1.4.1] — 2026-04-01

### Added
- Auto-index project on first use — no manual `au init`/`au reindex` needed (73afe86)

### Documentation
- Update README to v1.4.0 — task management, 19 tools, new sections (32da52e)

---

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
