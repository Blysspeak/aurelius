<p align="center">
  <img src="logo.png" width="200" alt="Aurelius" />
</p>

<h1 align="center">Aurelius</h1>

<p align="center">
  <strong>Self-hosted knowledge graph memory for developers and AI agents.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white&style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white&style=flat-square" alt="SQLite">
  <img src="https://img.shields.io/badge/MCP-ready-a6e3a1?style=flat-square" alt="MCP">
</p>

<p align="center">
  <em>Every AI session starts with full project context.<br>
  Self-hosted alternative to Mem0 and Zep — free, private, extensible.</em>
</p>

---

## The Problem

Every AI session starts from zero. You re-explain your projects, your past decisions, your architecture.

**With Aurelius:** index once → graph lives and grows through hooks → `memory_status` gives the full picture.

---

## Quick Start

```bash
git clone https://github.com/Blysspeak/aurelius && cd aurelius
./install.sh
```

This builds everything, installs binaries to `~/.local/bin`, sets up hooks, and initializes the database.

```bash
au reindex                  # index your project
au view                     # open graph visualization
au mcp                      # start MCP server for Claude Code
```

---

## CLI

```bash
au init                          # initialize database
au note "chose X over Y"        # capture a decision
au context beacon                # graph around a topic
au search "redis"                # full-text search
au reindex                       # index current project
au sync                          # pull from TimeForged
au view                          # open web graph UI
au touch path/to/file            # track file access
au export                        # export full graph as JSON
au mcp                           # start MCP server
```

---

## MCP Tools (Claude Code Integration)

Aurelius runs as an MCP server over stdio. Add to Claude Code via `/mcp`:

```
Command: au
Args: mcp
```

| Tool | Description |
|------|-------------|
| `memory_status` | Full project snapshot for session start |
| `memory_context` | BFS context around a topic |
| `memory_search` | FTS5 full-text search |
| `memory_add` | Add a knowledge node |
| `memory_relate` | Create typed edge between nodes |
| `memory_index` | Index a project directory |
| `memory_forget` | Delete a node |
| `memory_dump` | Export full graph |

---

## Web UI

Interactive knowledge graph visualization with Obsidian-style physics.

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
  aurelius-core/     — domain models, SQLite, graph ops, FTS5, indexer, connectors
  aurelius/          — daemon + MCP server (JSON-RPC 2.0 stdio)
  au/                — CLI + web UI server
ui/                  — React + TypeScript + Tailwind (graph visualization)
contrib/
  claude-code/       — session hooks (reindex, track edits)
  git-hooks/         — post-commit (captures decisions)
```

### Key Design

- **SQLite + WAL** — concurrent reads, single writer, local-first
- **FTS5** — full-text search kept in sync via triggers
- **Versioned migrations** — `schema_version` table, v1 base + v2 extended fields
- **Connector trait** — `async fn pull() -> Vec<RawEvent>` for data sources
- **Graph traversal** — BFS from FTS seed nodes, depth-limited expansion
- **Content hashing** — SHA256 for incremental re-indexing

---

## Hooks (Auto-Capture)

| Hook | Event | What it does |
|------|-------|-------------|
| `aurelius-reindex.sh` | Stop | Re-indexes project on session end |
| `aurelius-track-edit.sh` | PostToolUse (Edit/Write) | Increments access_count on file nodes |
| `post-commit` | git commit | Captures commit as Decision node |

---

## Roadmap

- [x] v0.1 — Core graph, CLI, MCP server, project indexer, web UI, TimeForged connector
- [ ] v0.2 — Git connector, beads integration, Beacon connector
- [ ] v0.3 — Obsidian import, temporal decay scoring, 3D graph view
- [ ] v0.4 — Connector SDK, HTTP API, Tauri desktop app
- [ ] v1.0 — Multi-project sync, team features

---

## License

[MIT](LICENSE)
