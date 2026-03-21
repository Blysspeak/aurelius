# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # build all crates
cargo build --release          # release build
cargo test                     # run all tests
cargo test -p aurelius-core    # test single crate
cargo clippy                   # lint
cargo fmt --check              # check formatting
cd ui && npm run build         # rebuild web UI
```

Two binaries are produced:
- `au` — the CLI (`crates/au`)
- `aurelius` — the daemon/MCP server (`crates/aurelius`)

After building, install with:
```bash
cp target/release/aurelius ~/.local/bin/aurelius
cp target/release/au ~/.local/bin/au
```

## Architecture

Aurelius is a self-hosted knowledge graph memory for developers and AI agents. It stores nodes (projects, decisions, concepts, problems, solutions, sessions) connected by typed edges in a local SQLite database with FTS5 full-text search.

### Workspace layout (3 crates)

- **`aurelius-core`** — shared library: domain models (`Node`, `Edge`, `NodeType`, `Relation`, `MemoryKind`), SQLite database with WAL mode and versioned migrations (V1-V4), graph operations (add/update/delete nodes, batch BFS traversal, FTS5 search with relevance ranking, typed search, unsolved problems query), and the `Connector` trait for data sources.
- **`aurelius`** — daemon binary with MCP server over stdio (JSON-RPC 2.0). Provides 12 tools for knowledge graph operations. Also serves as the library crate for MCP handlers.
- **`au`** — CLI binary using clap. Subcommands: `init`, `note`, `context`, `search`, `sync`, `reindex`, `view`, `export`, `mcp`, `touch`. The CLI delegates to `aurelius-core` for graph operations.

### MCP Tools (12)

| Tool | Purpose |
|---|---|
| `memory_status` | Session start snapshot — projects, decisions, problems, solutions, sessions. Supports project filter. |
| `memory_session` | Record session summary with decisions, problem/solution pairs, next steps. SHA-256 dedup. |
| `memory_recall` | Smart topic recall — FTS + BFS, results grouped by type, skips structural noise. |
| `memory_search` | FTS5 search with type filter and since filter. Wildcard `*` returns recent nodes. |
| `memory_context` | Raw BFS traversal from FTS seed nodes to specified depth. |
| `memory_add` | Create node with label, type, note, data (JSON), memory_kind (semantic/episodic). |
| `memory_update` | Update existing node's note/data by UUID or label. |
| `memory_relate` | Create typed edge between nodes. INSERT OR IGNORE for dedup. |
| `memory_forget` | Delete node by UUID (cascades to edges). |
| `memory_index` | Index project structure from Cargo.toml (crates, files, deps). |
| `memory_dump` | Paginated graph export (offset/limit). |
| `memory_gc` | Garbage collection — removes duplicate edges, orphans, duplicate nodes. |

### Key design details

- Single global database at `~/.local/share/aurelius/aurelius.db`
- Schema versioning: V1 (core tables), V2 (memory_kind, access tracking), V3 (indexes, edge dedup), V4 (FTS5 without data column)
- FTS5 indexes `label` and `note` only (not raw JSON `data`) — kept in sync via triggers
- Batch BFS context traversal — `WHERE id IN (...)` per level, not N+1 per node
- Search ranked by FTS relevance + access_count boost
- Session dedup via SHA-256 content_hash on (project, summary)
- Edge dedup via UNIQUE index on (from_id, to_id, relation) + INSERT OR IGNORE
- Problem lifecycle: unsolved = no Solution node with `solves` edge pointing to it
- Node types: project, decision, concept, problem, solution, session, crate, file, dependency, etc.
- Relations: uses, depends_on, solves, caused_by, belongs_to, contains, related_to, etc.

### contrib/

- `claude-code/` — hooks for Claude Code: session capture (Stop), file access tracking (PostToolUse), project reindex (Stop)
- `git-hooks/` — post-commit hook that captures commit messages as decision nodes (skips merge/chore/bump/release commits)

### Web UI

React + TypeScript + Tailwind CSS + react-force-graph-2d. Always-live graph visualization with Obsidian-style physics. Served by `au view` on port 7175.
