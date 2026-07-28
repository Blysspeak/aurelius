<p align="center">
  <img src="logo.png" width="200" alt="Aurelius" />
</p>

<h1 align="center">Aurelius</h1>

<p align="center">
  <strong>Self-hosted knowledge graph memory for developers and AI agents.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/v1.8.0-stable-a6e3a1?style=flat-square" alt="v1.8.0">
  <img src="https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white&style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white&style=flat-square" alt="SQLite">
  <img src="https://img.shields.io/badge/MCP-25_tools-a6e3a1?style=flat-square" alt="MCP">
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="#mcp-tools-25">MCP Tools</a> ·
  <a href="#task-management">Tasks</a> ·
  <a href="#web-ui">Graph UI</a> ·
  <a href="doc/README-ru.md">Русский</a>
</p>

---

## The Problem

Every AI session starts from zero. You re-explain your projects, your past decisions, your architecture. Tasks scatter across tools with no memory of what was done.

**With Aurelius:** `memory_status` → full project context. `task_view` → complete work history. `memory_session` → nothing lost between sessions.

---

## Quick Start

```bash
git clone https://github.com/Blysspeak/aurelius && cd aurelius
./install.sh
```

This builds binaries, installs to `~/.local/bin`, configures Claude Code MCP server and hooks, initializes the database, and indexes the project. Restart Claude Code and you're ready.

```
$ au --version
au 1.8.0
```

---

## MCP Tools (25)

Aurelius runs as an MCP server over stdio. `install.sh` configures it automatically, or add manually via `/mcp` in Claude Code (`command: au`, `args: ["mcp"]`).

### Knowledge Graph

| Tool | Description |
|------|-------------|
| `memory_status` | Session start — full project snapshot with active tasks. Optional `project` filter. |
| `memory_session` | Session end — save decisions, problems/solutions, next steps. Links to tasks. Returns active tasks hint. SHA-256 dedup. |
| `memory_recall` | Smart topic recall — FTS + BFS, grouped by type (incl. tasks), skips structural noise. |
| `memory_search` | Full-text search with `type`, `since`, and `limit` filters. `*` for recent. |
| `memory_context` | Raw BFS graph traversal from FTS seed nodes. |
| `memory_add` | Create node with label, type, note, data (JSON), memory_kind. |
| `memory_update` | Update existing node's note/data by UUID or label. |
| `memory_relate` | Create typed edge. INSERT OR IGNORE for dedup. |
| `memory_forget` | Delete node by UUID (cascades to edges). |
| `memory_gc` | Garbage collection — duplicate edges/nodes, orphans. |
| `memory_merge` | Merge two near-duplicate nodes — rewires edges, merges notes, deletes source. |
| `memory_dump` | Paginated graph export (offset/limit). |
| `memory_index` | Index project structure from Cargo.toml. |

### Task Management

| Tool | Description |
|------|-------------|
| `task_create` | Create structured task — title, description, acceptance criteria, priority, subtask/blocking relations. |
| `task_update` | Update status, priority, criteria. Auto-tracks `started_at`/`completed_at`. |
| `task_list` | Filter by project, status, priority. Sorted by priority, shows work log count. |
| `task_log` | Record work done — creates WorkLog + optional Decision/Problem/Solution nodes. Auto-activates backlog tasks. |
| `task_view` | Full task branch — timeline of work logs, decisions, problems, solutions, subtasks. |
| `task_stats` | Task analytics — counts by status/priority, completion rate, avg/median duration, blocked count, oldest active. |

### Skills

Reusable procedural "how-to" cards with progressive disclosure — the trigger is FTS-indexed (discoverable), the full markdown body is fetched on demand. The index auto-injects every session via a SessionStart hook, and skills surface in `memory_recall`/`memory_status`.

| Tool | Description |
|------|-------------|
| `skill_save` | Create/update a skill (upsert by name). Trigger → indexed note; body + tags → data. |
| `skill_list` | Cheap index (name + trigger + tags + uses), ranked by usage. Optional FTS `query`/`tag`. |
| `skill_get` | Full markdown body by name, with fuzzy FTS fallback + disambiguation. Bumps usage. |
| `skill_remove` | Delete a skill by name. |

### Web Search

| Tool | Description |
|------|-------------|
| `search_web` | Brave Search API with SQLite cache. Repeat queries served from cache. Optional `save_to_graph`. |
| `search_recall` | FTS search through cached web search results from past sessions. |

---

## Task Management

Tasks are **hub nodes** in the knowledge graph. Everything you do on a task — work logs, decisions, problems solved — automatically links to it, creating a complete branch of work history.

```
[Project] <──belongs_to── [Task: Implement auth]
                              │
                              ├──contains──> [WorkLog: researched JWT libs]
                              ├──contains──> [WorkLog: implemented token refresh]
                              ├──contains──> [Decision: chose jsonwebtoken over jwt-simple]
                              ├──contains──> [Problem: token expiry race condition]
                              │                  └──solves── [Solution: added mutex lock]
                              ├──subtask_of──> [Task: Security epic]
                              └──blocks──> [Task: Deploy to prod]
```

### Status Lifecycle

```
backlog → active → done
                 → blocked (with reason)
                 → cancelled
```

First `task_log` entry auto-activates a backlog task. `task_update` tracks timestamps automatically.

### Acceptance Criteria

Every task can have a Definition of Done checklist:

```bash
au task new "Implement auth" --project myapp --priority high \
  -c "JWT tokens work" \
  -c "Refresh flow tested" \
  -c "Rate limiting active"
```

### Integration

- **`memory_status`** shows active/blocked tasks at session start
- **`memory_session`** accepts `tasks` parameter to link sessions to tasks, returns active tasks as hints
- **`memory_recall`** includes tasks in search results
- **`task_view`** aggregates the full work branch via BFS traversal

---

## CLI

```bash
au init                            # initialize database
au note "chose X over Y" -p app   # capture a decision → project
au context beacon                  # graph around a topic
au search "redis"                  # full-text search
au reindex                         # index current project
au view                            # open web graph UI
au touch path/to/file              # track file access
au export                          # export full graph as JSON
au mcp                             # start MCP server
au skills                          # print the skill index
au db check [PATH]                 # verify integrity (read-only); PATH verifies a snapshot
au db backup                       # safe snapshot via VACUUM INTO
```

### Task Commands

```bash
au task new "Title" -p myapp --priority high -c "Tests pass"
au task list --project myapp --status active,blocked
au task show <id>                  # full details with work log branch
au task log <id> "Did X, Y, Z"    # record work (auto-activates)
au task done <id>                  # mark complete
au task block <id> "waiting on API keys"
au task activate <id>              # resume blocked task
```

### Backups

```bash
au db check          # quick integrity report; exits non-zero when damaged
au db check --full   # exhaustive check, every table
au db check FILE     # verify a snapshot — a snapshot is an ordinary database
au db backup         # snapshot → aurelius-<UTC timestamp>.db next to the database
```

Snapshots are taken automatically: `install.sh` registers a SessionStart hook that keeps
a rolling set in `<data-dir>/aurelius/backups/`. Cadence follows activity rather than the
clock — the graph only changes when Claude Code, the CLI or the git hooks touch it, so an
idle machine does not accumulate identical copies. Several sessions in a day cost one
snapshot; ~50 ms for an 8 MB graph. Each snapshot is verified with `au db check` right
after it is written — one that fails is renamed to `.FAILED-CHECK` so it can never be
mistaken for a good backup, and kept, because a bad snapshot is evidence worth reading.

| Variable | Default | Meaning |
|---|---|---|
| `AURELIUS_BACKUP_KEEP` | `7` | snapshots retained |
| `AURELIUS_BACKUP_MIN_HOURS` | `24` | minimum age of the newest snapshot before a new one is taken |

> **Never copy, move or restore `aurelius.db` with `cp`/`mv`/`rsync`, a file manager or a
> backup agent while `au` or an MCP server is running.** In WAL mode, cross-process cache
> coherency runs through the `-shm` WAL-index rather than the database header, so replacing
> the file underneath open connections lets a live process keep flushing its cached pages
> into the new file — producing a database whose header describes 181 pages while its body
> holds 1781. `au db backup` uses SQLite's own `VACUUM INTO` and is the only safe way to
> copy a live database.

To restore a snapshot: stop everything that touches the database (every `au mcp`, `au view`,
any editor with hooks), move the current database and its `-wal`/`-shm` aside, copy the
snapshot into place, run `au db check`, then restart. The "stop everything" step is why this
is a documented procedure and not a command — `au` cannot stop processes it did not start.

---

## Web UI

Interactive knowledge graph visualization with Obsidian-style physics.

```bash
au view            # opens browser at localhost:7175
au view -P 8080    # custom port
au view --no-open  # don't open browser
```

Features:
- **Obsidian-style physics** — gentle forces, no pinning, drag follows neighbors naturally
- **Project hub nodes** — central nodes connecting sessions, decisions, problems, solutions, tasks
- **Clean labels** — only project names visible by default, details on hover/select
- **Project filter** — sidebar scoping by project (extracts from `[project-name]` label prefix)
- **Node type filter** — filter by decision, solution, problem, session, project, task
- Color-coded node types, node detail panel, keyboard shortcuts (/, Esc, Scroll)

---

## Session Lifecycle

```
Session start  →  memory_status(project: "myapp")     # full context + active tasks
During work    →  task_log, memory_add, memory_relate  # track progress
Session end    →  memory_session(summary, decisions, problems_solved, tasks)
```

---

## Architecture

```
crates/
  aurelius-core/
    src/graph/       — crud.rs, search.rs, traverse.rs
    src/db.rs        — SQLite setup, migrations V1-V5
    src/models.rs    — Node, Edge, NodeType, Relation, MemoryKind
    src/indexer.rs   — Cargo.toml project indexer
  aurelius/
    src/mcp/
      handlers/      — status.rs, session.rs, crud.rs, search.rs, task.rs
      tools.rs       — MCP tool definitions (JSON schemas)
      mod.rs         — JSON-RPC 2.0 server
    src/search/
      brave.rs       — Brave Search API client
      cache.rs       — SQLite search cache with FTS5
  au/                — CLI + web UI server
ui/                  — React + TypeScript + Tailwind (graph visualization)
contrib/
  claude-code/       — session hooks (reindex, track edits)
  git-hooks/         — post-commit (captures decisions)
```

### Key Design

- **SQLite + WAL** — concurrent reads, single writer, local-first. Every connection sets a busy timeout, verifies that WAL mode actually took effect, and checks the file header against the file size before use
- **FTS5** — indexes label + note (not raw JSON), kept in sync via triggers
- **5 schema migrations** — V1 core, V2 access tracking, V3 indexes + edge dedup, V4 clean FTS, V5 search cache. Applied atomically in a single `BEGIN IMMEDIATE` transaction
- **Batch BFS** — `WHERE id IN (...)` per level, not N+1 per node
- **Session dedup** — SHA-256 content hash on (project, summary)
- **Edge dedup** — UNIQUE constraint on (from_id, to_id, relation)
- **Task hub nodes** — tasks collect work logs, decisions, problems, solutions via `contains` edges
- **Problem lifecycle** — unsolved = no Solution node with `solves` edge
- **Relevance ranking** — FTS results boosted by access_count
- **Project hub nodes** — auto-created by `memory_session` and `task_create`, all children linked via `belongs_to`
- **Label convention** — child nodes prefixed `[project-name] description`, project nodes use plain names

### Node Types

`project` · `task` · `work_log` · `decision` · `concept` · `problem` · `solution` · `session` · `crate` · `file` · `dependency` · `module` · `config` · `person` · `server` · `language`

### Relations

`belongs_to` · `contains` · `solves` · `subtask_of` · `blocks` · `depends_on` · `uses` · `caused_by` · `related_to` · `implements` · `configures` · `tracked_by` · `inspired_by` · `conflicts_with` · `supersedes` · `learned_from` · `imports` · `exports`

---

## Hooks (Auto-Capture)

Installed automatically by `install.sh` into `~/.claude/settings.json`.

| Hook | Event | What it does |
|------|-------|-------------|
| `aurelius-reindex.sh` | Stop | Re-indexes project on session end |
| `aurelius-track-edit.sh` | PostToolUse (Edit/Write) | Increments access_count on file nodes |
| `post-commit` | git commit | Captures commit as Decision node, linked to project via `belongs_to` |

---

## Roadmap

- [x] v0.1 — Core graph, CLI, MCP server (8 tools), project indexer, web UI
- [x] v0.2 — Wildcard search, dump pagination
- [x] v0.3 — Session memory, memory_update, enhanced memory_add
- [x] v0.4 — Smart recall, type-filtered search, problem lifecycle, always-live graph
- [x] v0.5 — Query optimization, session dedup, no double storage
- [x] v1.0 — Project scoping, batch BFS, GC, edge dedup, FTS cleanup, modular codebase, install.sh
- [x] v1.1 — Web search (Brave API + SQLite cache + graph integration)
- [x] v1.2 — UI overhaul, project-scoped linking, indexer fix
- [x] v1.3 — Obsidian-style graph physics, project hub nodes, session auto-linking
- [x] v1.4 — Task management (5 MCP tools + CLI), work branches, acceptance criteria
- [x] v1.5 — `memory_merge`, `task_stats`, semantic cluster graph layout
- [x] v1.6 — Skills subsystem (4 MCP tools + `au skills`), session auto-injection
- [x] v1.7 — DB hardening: busy timeout, atomic migrations, integrity gate, `au db check` / `au db backup`
- [x] v1.8 — `au db check [PATH]` — self-verifying rolling backups
- [ ] Next — npm distribution, `au repair`, context-ranked search, git log connector

---

## License

[MIT](LICENSE)
