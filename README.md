<p align="center">
  <img src="logo.png" width="200" alt="Aurelius" />
</p>

<h1 align="center">Aurelius</h1>

<p align="center">
  <strong>Self-hosted knowledge graph memory for developers and AI agents.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/v1.0.0-stable-a6e3a1?style=flat-square" alt="v1.0.0">
  <img src="https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white&style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white&style=flat-square" alt="SQLite">
  <img src="https://img.shields.io/badge/MCP-12_tools-a6e3a1?style=flat-square" alt="MCP">
</p>

<p align="center">
  <em>Every AI session starts with full project context.<br>
  Decisions, problems, solutions — preserved across sessions.</em>
</p>

---

## The Problem

Every AI session starts from zero. You re-explain your projects, your past decisions, your architecture.

**With Aurelius:** `memory_status` → full picture. `memory_session` → nothing lost.

---

## Quick Start

```bash
git clone https://github.com/Blysspeak/aurelius && cd aurelius
./install.sh
```

This builds binaries, installs to `~/.local/bin`, configures Claude Code MCP server and hooks, initializes the database, and indexes the project. Restart Claude Code and you're ready.

---

## MCP Tools (12)

Aurelius runs as an MCP server over stdio. `install.sh` configures it automatically, or add manually via `/mcp` in Claude Code (`command: au`, `args: ["mcp"]`).

| Tool | Description |
|------|-------------|
| `memory_status` | Session start — full project snapshot. Optional `project` filter. |
| `memory_session` | Session end — save decisions, problems/solutions, next steps. SHA-256 dedup. |
| `memory_recall` | Smart topic recall — FTS + BFS, grouped by type, skips structural noise. |
| `memory_search` | Full-text search with `type`, `since`, and `limit` filters. `*` for recent. |
| `memory_context` | Raw BFS graph traversal from FTS seed nodes. |
| `memory_add` | Create node with label, type, note, data (JSON), memory_kind. |
| `memory_update` | Update existing node's note/data by UUID or label. |
| `memory_relate` | Create typed edge. INSERT OR IGNORE for dedup. |
| `memory_forget` | Delete node by UUID (cascades to edges). |
| `memory_gc` | Garbage collection — duplicate edges/nodes, orphans. |
| `memory_dump` | Paginated graph export (offset/limit). |
| `memory_index` | Index project structure from Cargo.toml. |

### Session Lifecycle

```
Session start  →  memory_status(project: "myapp")
During work    →  memory_add, memory_relate (as needed)
Session end    →  memory_session(summary, decisions, problems_solved, next_steps)
```

---

## CLI

```bash
au init                          # initialize database
au note "chose X over Y"        # capture a decision
au context beacon                # graph around a topic
au search "redis"                # full-text search
au reindex                       # index current project
au view                          # open web graph UI
au touch path/to/file            # track file access
au export                        # export full graph as JSON
au mcp                           # start MCP server
```

---

## Web UI

Interactive knowledge graph visualization with always-live Obsidian-style physics.

```bash
au view            # opens browser at localhost:7175
au view -P 8080    # custom port
au view --no-open  # don't open browser
```

Features: force-directed graph, color-coded node types, sidebar filters, node detail panel, search, drag interaction.

---

## Architecture

```
crates/
  aurelius-core/
    src/graph/       — crud.rs, search.rs, traverse.rs
    src/db.rs        — SQLite setup, migrations V1-V4
    src/models.rs    — Node, Edge, NodeType, Relation, MemoryKind
    src/indexer.rs   — Cargo.toml project indexer
  aurelius/
    src/mcp/
      handlers/      — status.rs, session.rs, crud.rs
      tools.rs       — MCP tool definitions (JSON schemas)
      mod.rs         — JSON-RPC 2.0 server
  au/                — CLI + web UI server
ui/                  — React + TypeScript + Tailwind (graph visualization)
contrib/
  claude-code/       — session hooks (reindex, track edits)
  git-hooks/         — post-commit (captures decisions)
```

### Key Design

- **SQLite + WAL** — concurrent reads, single writer, local-first
- **FTS5** — indexes label + note (not raw JSON), kept in sync via triggers
- **4 schema migrations** — V1 core, V2 access tracking, V3 indexes + edge dedup, V4 clean FTS
- **Batch BFS** — `WHERE id IN (...)` per level, not N+1 per node
- **Session dedup** — SHA-256 content hash on (project, summary)
- **Edge dedup** — UNIQUE constraint on (from_id, to_id, relation)
- **Problem lifecycle** — unsolved = no Solution node with `solves` edge
- **Relevance ranking** — FTS results boosted by access_count
- **Content hashing** — SHA-256 for incremental re-indexing

---

## Hooks (Auto-Capture)

Installed automatically by `install.sh` into `~/.claude/settings.json`.

| Hook | Event | What it does |
|------|-------|-------------|
| `aurelius-reindex.sh` | Stop | Re-indexes project on session end |
| `aurelius-track-edit.sh` | PostToolUse (Edit/Write) | Increments access_count on file nodes |
| `post-commit` | git commit | Captures commit as Decision node |

---

## Roadmap

- [x] v0.1 — Core graph, CLI, MCP server (8 tools), project indexer, web UI
- [x] v0.2 — Wildcard search, dump pagination
- [x] v0.3 — Session memory, memory_update, enhanced memory_add
- [x] v0.4 — Smart recall, type-filtered search, problem lifecycle, always-live graph
- [x] v0.5 — Query optimization, session dedup, no double storage
- [x] v1.0 — Project scoping, batch BFS, GC, edge dedup, FTS cleanup, modular codebase, install.sh auto-config
- [ ] Next — Semantic/embedding search, auto-session via Stop hook, git log connector

---

## License

[MIT](LICENSE)
