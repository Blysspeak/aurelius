# Architecture

[Back to README](../README.md)

How a session uses Aurelius, how the code is laid out, and the design choices behind the graph.

## Contents

- [Session Lifecycle](#session-lifecycle)
- [Crate layout](#crate-layout)
- [Key Design](#key-design)
- [Node Types](#node-types)
- [Relations](#relations)
- [Web UI](#web-ui)

## Session Lifecycle

```
Session start  →  memory_status(project: "myapp")     # full context + active tasks
During work    →  task_log, memory_add, memory_relate  # track progress
Session end    →  memory_session(summary, decisions, problems_solved, tasks)
```

## Crate layout

```
crates/
  aurelius-core/
    src/graph/       — crud.rs, search.rs (FTS + hybrid), traverse.rs, snapshot.rs (layers + distillate),
                       rank.rs (the score recall orders by, RRF), fusion.rs, rerank.rs (cross-encoder client),
                       render.rs (one node → one prose line), lease.rs, path.rs, pickup.rs, import.rs, export.rs
    src/db.rs        — SQLite setup, migrations V1-V19
    src/fts.rs, stem.rs — full-text query building, Snowball stemming for nodes_stem_fts
    src/embed.rs     — BGE-M3 and bge-reranker-v2-m3 loading (ONNX Runtime, CUDA or CPU)
    src/embed_socket.rs — embed.sock protocol: query vectors and rerank scores from the daemon
    src/git.rs       — repository state for the snapshot's Repository layer
    src/models.rs    — Node, Edge, NodeType, Relation, MemoryKind
    src/provenance.rs — confidence/evidence/subject/volatility, parsed once for both doors
    src/secret.rs    — secret detection: masks secrets in trace payloads, classifies coordinates
    src/eval.rs      — recall scored against a frozen fixture, read-only
    src/search_eval.rs — search baseline for au eval-search
    src/tasks.rs     — task lifecycle, leasing, ripeness
    src/reminders.rs — reminders/reminder_events: state, owner, journal of postponements
    src/indexer.rs   — project indexer behind au reindex / memory_index: crates and files, no dependency nodes
    src/session_registry.rs — Claude Code session id per process, for traces written outside MCP
    src/identity.rs  — local identity config (~/.config/aurelius/identity.toml)
    src/sync/        — push/pull types, upsert + last-writer-wins merge logic
    src/trace.rs     — append-only journal of what the agent actually did
    src/probes.rs    — claims checked against ground truth (paths, git SHAs)
    src/codec.rs     — surprise gate: normalised compression distance per scope
    src/window.rs    — recall as a transaction: query signature, labile windows
    src/differ.rs    — outcome judge: reinforce / erode / fork / null, no model called
    src/ledger.rs    — clearing, node value in bits, bankruptcy-driven GC
    src/obligations.rs — promises taken in from speech, settled by later events
  aurelius-sync-server/ — self-hosted sync server (POST/GET /sync/push,pull,grants)
  aurelius/
    src/mcp/
      handlers/      — status.rs, session.rs, crud.rs, search.rs, task.rs, reminder.rs
      tools.rs       — MCP tool definitions (JSON schemas)
      mod.rs         — JSON-RPC 2.0 server
    src/search/
      brave.rs       — Brave Search API client
      perplexity.rs  — Perplexity Search API client
      cache.rs       — SQLite search cache with FTS5, scoped per provider
  au/                — CLI + web UI server (+ `identity`/`share` sync commands, plugin hook modes, `au daemon`)
ui/                  — React + TypeScript + Tailwind (graph visualization)
contrib/
  claude-code/       — deprecated bash hook wrappers (superseded by plugin/)
  git-hooks/         — post-commit (mirrors commit messages as decisions; opt-in)
  systemd/           — user unit for au daemon
deploy/
  aurelius-sync-server/ — Dockerfile + docker-compose for self-hosting the sync server
```

## Key Design

- **SQLite + WAL** — concurrent reads, single writer, local-first. Every connection sets a busy timeout, verifies that WAL mode actually took effect, and checks the file header against the file size before use
- **FTS5** — indexes label + note (not raw JSON), kept in sync via triggers, plus a stemmed twin (`nodes_stem_fts`)
- **Vectors** — BGE-M3 embeddings in a `sqlite-vec` table, computed by the daemon from a queue; see [Search](memory.md#search)
- **19 schema migrations** — V1 core, V2 access tracking, V3 indexes + edge dedup, V4 clean FTS, V5 search cache, V6 sync attribution/tombstones, V7-V8 documents and skills, V9 the action journal (`act_trace`, `probes`, `pathways`, `labile_window`, `corrections`), V10 `codec`/`delta`/`node_version`, V11 obligations, V12 readable obligation objects, V13 the run that wrote a record, V14 the subject a fact asserts about, V15 `reminders`/`reminder_events`, V16 `node_embeddings`, V17 run nodes retyped, V18 `embedding_queue`, V19 `nodes_stem_fts`. Applied atomically in a single `BEGIN IMMEDIATE` transaction. A database newer than the binary is refused with instructions instead of being opened
- **Sync attribution** — `created_by`/`updated_by` stamped from the local identity config; deletes are soft (`deleted_at`) so they propagate as tombstones instead of resurrecting on the next sync
- **Batch BFS** — `WHERE id IN (...)` per level, not N+1 per node
- **Session dedup** — SHA-256 content hash on (project, summary)
- **Edge dedup** — UNIQUE constraint on (from_id, to_id, relation)
- **Task hub nodes** — tasks collect work logs, decisions, problems, solutions via `contains` edges
- **Problem lifecycle** — unsolved = no Solution node with `solves` edge
- **Relevance ranking** — recall orders by one score in `graph/rank.rs`: normalised bm25, provenance, freshness measured from `measured_at`, access count and node type. Full-text and dense lists are fused by RRF (k = 12), then optionally reranked by a cross-encoder
- **Project hub nodes** — auto-created by `memory_session`, `task_create` and `memory_add(project=…)`, all children linked via `belongs_to`
- **Project scoping** — membership is the label prefix `[project-name]` **or** an edge to the project node, in either direction and under any relation. Checking only the label made everything written via `memory_add` + `memory_relate` invisible to project queries
- **Label convention** — child nodes prefixed `[project-name] description`, project nodes use plain names

## Node Types

`project` · `task` · `work_log` · `decision` · `concept` · `problem` · `solution` · `session` · `skill` · `user_fact` · `digest` · `crate` · `file` · `dependency` · `module` · `config` · `person` · `server` · `language`

## Relations

`belongs_to` · `contains` · `solves` · `subtask_of` · `blocks` · `depends_on` · `uses` · `caused_by` · `related_to` · `implements` · `configures` · `tracked_by` · `inspired_by` · `conflicts_with` · `supersedes` · `learned_from` · `imports` · `exports`

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
