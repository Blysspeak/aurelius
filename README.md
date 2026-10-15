<p align="center">
  <img src="logo.png" width="200" alt="Aurelius" />
</p>

<h1 align="center">Aurelius</h1>

<p align="center">
  <strong>Self-hosted knowledge graph memory for developers and AI agents.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/v3.8.0-stable-a6e3a1?style=flat-square" alt="v3.8.0">
  <img src="https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white&style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white&style=flat-square" alt="SQLite">
  <img src="https://img.shields.io/badge/MCP-45_tools-a6e3a1?style=flat-square" alt="MCP">
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="docs/mcp-tools.md">MCP Tools</a> ·
  <a href="docs/memory.md#search">Search</a> ·
  <a href="docs/memory.md#memory-snapshot">Snapshot</a> ·
  <a href="docs/tasks.md">Tasks</a> ·
  <a href="docs/sync.md">Sync</a> ·
  <a href="docs/architecture.md#web-ui">Graph UI</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

Aurelius keeps projects, decisions, problems, solutions, tasks and reminders in one local SQLite
knowledge graph, and serves it to Claude Code as an MCP server, to you as the `au` CLI, and to the
browser as a graph UI.

## The Problem

Every AI session starts from zero. You re-explain your projects, your past decisions, your architecture. Tasks scatter across tools with no memory of what was done.

**With Aurelius:** `memory_status` → full project context. `task_view` → complete work history. `memory_session` → nothing lost between sessions.

---

## Quick Start

Aurelius ships as a [Claude Code plugin](plugin/hooks.json): seven `au` hook commands, skill cards and
the `/pickup` command, installed with two `claude plugin` commands instead of hand-edited
`settings.json`. The MCP server is registered separately, user-scope, by `install.sh` (`claude mcp
add -s user aurelius au mcp`) — a plugin-bundled server would rename every tool to
`mcp__plugin_aurelius_aurelius__*`.

**Clean machine (Linux, macOS)**

```bash
git clone https://github.com/Blysspeak/aurelius && cd aurelius
cargo build --release
install -m 755 target/release/au target/release/aurelius ~/.local/bin/   # must be in PATH
au init
claude mcp add -s user aurelius au mcp   # MCP server, user scope: tools stay mcp__aurelius__*
claude plugin marketplace add Blysspeak/aurelius      # or a local clone path: claude plugin marketplace add .
claude plugin install aurelius@blysspeak -s user
```

Restart Claude Code. `claude plugin list` shows `aurelius@blysspeak`, and a fresh session gets the
skill index at start.

**Existing machine (stood up by hand or by an older `install.sh`)**

```bash
cd aurelius && git pull && ./install.sh
```

`install.sh` builds the binaries, installs the plugin, registers the MCP server user-scope with
`claude mcp add -s user aurelius au mcp`, and removes any legacy hook and `mcpServers.aurelius`
entries it previously wrote into `~/.claude/settings.json` and `~/.claude.json` — printing each
removed entry with its reason and leaving a `.bak-<UTC date>` copy next to each file it touches.
Migration keeps a canonical `au mcp` entry in `~/.claude.json` and removes only the old
wrapper-path entry; re-running the whole script, including the MCP registration, is a no-op once
migrated. Use `./install.sh --migrate-only` to run just the cleanup, without building anything or
registering the server.

**Windows**

```powershell
git clone https://github.com/Blysspeak/aurelius; cd aurelius
cargo build --release
New-Item -ItemType Directory -Force "$env:USERPROFILE\.local\bin" | Out-Null
Copy-Item target\release\au.exe "$env:USERPROFILE\.local\bin\"    # must be in PATH
au init
claude mcp add -s user aurelius au mcp
claude plugin marketplace add Blysspeak/aurelius
claude plugin install aurelius@blysspeak -s user
```

Git Bash and python3 are not required — every plugin hook runs `au` directly. If hooks were
previously added by hand, remove them from `$env:USERPROFILE\.claude\settings.json` (hooks whose
command matches `aurelius-*.sh` or `au … --hook`) — copy the file first. A `mcpServers.aurelius`
entry in `$env:USERPROFILE\.claude.json` is now the right place, when its command is `au mcp`;
only an entry pointing at the old wrapper script is stale — replace it with
`claude mcp add -s user aurelius au mcp`.

```
$ au --version
au 3.8.0 (0123456789ab)
$ aurelius --version        # the MCP binary answers too, instead of holding stdin open
aurelius 3.8.0 (0123456789ab)
```

The parenthesis is the 12-character commit the binary was built from, with `-dirty` appended
when the tree had uncommitted changes and `unknown` for a build outside git — so an installed
binary can be traced back to its source.

Nothing is indexed behind your back: no `au` command and no `memory_status` call turns the
current directory into a project. A repository enters the graph only through an explicit
`au reindex` (or MCP `memory_index`); `install.sh` runs it once, for the aurelius checkout itself.


---

## Features

- **Knowledge graph over MCP** — 45 tools for memory, tasks, reminders, skills, web search,
  documents and secret coordinates; 35 are listed by default. [MCP Tools](docs/mcp-tools.md)
- **Memory snapshot** — a small layered slice of the graph under a hard byte budget, with a fixed
  JSON shape for programs. [Memory](docs/memory.md#memory-snapshot)
- **Hybrid search** — full-text plus BGE-M3 vectors, fused by RRF and optionally reranked by a
  cross-encoder. [Search](docs/memory.md#search)
- **Task management** — tasks are hub nodes with work logs, evidence runs, ripening, resolutions,
  leasing and a fitness gate. [Task Management](docs/tasks.md)
- **Reminders** — explicit states and a journal of postponements, delivered by one daemon or at
  the next turn. [Reminders](docs/reminders.md)
- **CLI** — the `au` binary: exit codes, provenance fields, secrets, backups and maintenance.
  [CLI](docs/cli.md)
- **Project sync** — share one project between instances over a self-hosted server.
  [Project Sync](docs/sync.md)
- **Hooks** — a Claude Code plugin whose hooks call `au` directly. [Hooks](docs/hooks.md)
- **Architecture and Web UI** — crate layout, design choices, node types, relations and the graph
  viewer. [Architecture](docs/architecture.md)

## Documentation

| Document | What is inside |
|----------|----------------|
| [docs/mcp-tools.md](docs/mcp-tools.md) | Every MCP tool by group, including the admin tools hidden from `tools/list` |
| [docs/memory.md](docs/memory.md) | Memory snapshot, its JSON form, project membership, the search path and its settings |
| [docs/tasks.md](docs/tasks.md) | Task lifecycle, timestamps, branches, evidence, closing, criteria, leasing, fitness gate, task commands |
| [docs/reminders.md](docs/reminders.md) | Reminder model, the `au daemon` clock, reminder commands |
| [docs/cli.md](docs/cli.md) | Command overview, exit codes, provenance, secrets, backups, maintenance |
| [docs/sync.md](docs/sync.md) | Project sync between instances and the sync commands |
| [docs/hooks.md](docs/hooks.md) | Claude Code plugin hooks and what each one does |
| [docs/architecture.md](docs/architecture.md) | Session lifecycle, crate layout, key design, node types, relations, Web UI |
| [docs/roadmap.md](docs/roadmap.md) | Released versions |
| [CHANGELOG.md](CHANGELOG.md) | Changes per release |

---

## License

[MIT](LICENSE)
