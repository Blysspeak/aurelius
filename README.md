<p align="center">
  <img src="aurelius-logo.png" width="200" alt="Aurelius" />
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
  <em>Captures decisions, projects, and context automatically.<br>
  Self-hosted alternative to Mem0 and Zep — free, private, extensible.</em>
</p>

---

## The Problem

Every AI session starts from zero. You re-explain your projects, your past decisions, your architecture. Context is lost between conversations, between tools, between days.

**With Aurelius:** pull one node — get the entire web of related knowledge.

```
git commit → decision captured
bd close   → solution linked to problem
tf session → project context updated
au note    → instant knowledge node
           ↓
     Claude Code asks:
   "What's the context for Beacon?"
           ↓
  Aurelius returns: project + decisions +
  related concepts + recent deploys +
  open tasks — all in one shot
```

---

## Quick Start

```bash
git clone https://github.com/Blysspeak/aurelius && cd aurelius
cargo build --release
./target/release/au init
```

Then tell Claude Code to use it:

```bash
bash contrib/claude-code/install.sh
```

---

## How It Works

Aurelius maintains a **local knowledge graph** in SQLite — nodes (projects, decisions, concepts, problems) connected by typed edges (uses, solves, inspired_by, conflicts_with).

Data flows in automatically from your existing tools:

| Source | What's captured |
|--------|----------------|
| `git log` | Decisions, file changes, project activity |
| `beads` | Tasks, epics, dependencies |
| `TimeForged` | Active projects, time, languages |
| `Beacon` | Failed deploys → problem nodes |
| Claude Code | Terms and concepts from dialogs |
| `au note` | Manual decisions and observations |

---

## CLI

```bash
au init                          # initialize graph
au note "why I chose X over Y"   # capture a decision
au context beacon                # graph around a topic
au search "redis"                # full-text search
au graph                         # open web visualization
au sync                          # pull from all connectors
au profile work                  # switch memory profile
au export > backup.json          # export full graph
au import --from obsidian ~/vault
```

---

## MCP Integration (Claude Code)

Aurelius runs as an MCP server. Claude can query your memory directly:

```json
{
  "mcpServers": {
    "aurelius": {
      "command": "au",
      "args": ["mcp"]
    }
  }
}
```

Available tools:

| Tool | Description |
|------|-------------|
| `memory_context(topic, depth)` | Graph around a topic |
| `memory_search(query)` | Full-text search |
| `memory_add(label, type, note)` | Add a node |
| `memory_relate(a, b, relation)` | Link two nodes |
| `memory_dump()` | Full snapshot for new chat |

---

## Architecture

```
crates/
  aurelius-core/     # types, SQLite, graph ops, FTS5
  aurelius/          # daemon + MCP server (stdio)
  au/                # CLI
contrib/
  claude-code/       # hooks for automatic capture
  git-hooks/         # post-commit parser
  waybar/            # status bar widget
  beads/             # beads connector
  timeforged/        # TimeForged connector
```

---

## Connectors

Aurelius uses a simple connector trait — anyone can build one:

```rust
trait Connector {
    fn name(&self) -> &str;
    fn pull(&self) -> anyhow::Result<Vec<RawEvent>>;
}
```

Official connectors: `git`, `beads`, `timeforged`, `beacon`
Community connectors: anything you want.

---

## Roadmap

- [ ] v0.1 — Core: SQLite graph, CLI, MCP server, git hook
- [ ] v0.2 — Dev ecosystem: beads, TimeForged, Beacon connectors, Waybar
- [ ] v0.3 — Universal: Obsidian import, Telegram connector, web UI
- [ ] v0.4 — Platform: Connector SDK, HTTP API
- [ ] v1.0 — Cloud: sync, team plans

---

## License

[MIT](LICENSE)
