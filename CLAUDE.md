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
```

Two binaries are produced:
- `au` — the CLI (`crates/au`)
- `aurelius` — the daemon/MCP server (`crates/aurelius`)

## Architecture

Aurelius is a self-hosted knowledge graph memory for developers and AI agents. It stores nodes (projects, decisions, concepts, problems) connected by typed edges in a local SQLite database with FTS5 full-text search.

### Workspace layout (3 crates)

- **`aurelius-core`** — shared library: domain models (`Node`, `Edge`, `NodeType`, `Relation`, `RawEvent`), SQLite database setup with WAL mode and auto-migration, graph operations (add node/edge, BFS context traversal, FTS5 search), and the `Connector` trait for data sources.
- **`aurelius`** — daemon binary that starts the MCP server over stdio (JSON-RPC 2.0). The MCP server is how Claude Code queries the knowledge graph.
- **`au`** — CLI binary using clap. Subcommands: `init`, `note`, `context`, `search`, `sync`, `export`, `mcp`. The CLI delegates to `aurelius-core` for graph operations and calls into `aurelius::mcp::serve()` for the `mcp` subcommand.

### Key design details

- Database lives at `$XDG_DATA_HOME/aurelius/aurelius.db` (typically `~/.local/share/aurelius/aurelius.db`)
- Graph traversal (`graph::context`) does BFS from FTS seed nodes, expanding edges to `depth` levels
- FTS5 is kept in sync via SQLite triggers (insert/update/delete on `nodes` table)
- The `Connector` trait (`connector.rs`) is the extension point for data sources — each connector produces `Vec<RawEvent>`. Connectors for git, beads, timeforged, and beacon are planned but not yet implemented.
- MCP server is stub — needs JSON-RPC 2.0 implementation with tools: `memory_context`, `memory_search`, `memory_add`, `memory_relate`, `memory_dump`

### contrib/

- `claude-code/` — session hook that captures project heartbeats into Aurelius on Claude Code session stop
- `git-hooks/` — post-commit hook that captures commit messages as decision nodes (skips merge/chore/bump/release commits)
