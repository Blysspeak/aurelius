# Roadmap

[Back to README](../README.md)

Released versions and what each one added. Per-release details are in [CHANGELOG.md](../CHANGELOG.md).

## Contents

- [Releases](#releases)
- [Version bump rule](#version-bump-rule)

## Releases

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
- [x] v1.9 — Documents to Markdown, converted locally (3 MCP tools + `au doc`)
- [x] v1.10 — Seven-layer snapshot; Bit-i-Delo stages 1-4: action journal, ground-truth probes, surprise gate, outcome judge, clearing and obligations
- [x] v1.11 — Project scoping by edge, not just by label prefix; `au snapshot --json`; `memory_add` warns on unattached nodes
- [x] v1.12 — `au session`/`au relate` writing moved into core; records stamped with the run that wrote them; `au journal`
- [x] v2.0 — Provenance: `confidence` + `evidence` required, volatility ageing, `subject` contradiction refusal
- [x] v2.1 — Task leasing (`claim`/`renew`/`release`/`give-up`) and the autonomy fitness gate over the queue
- [x] v3.0 — Closing a task became automatic: evidence attaches itself, `task_ripe` presents what is ready; doc graph import; secret coordinates
- [x] v3.1 — Perplexity Search as a second `search_web` provider next to Brave, cache scoped per provider
- [x] v3.2 — Provenance fields on every write handle; `memory_status` reports a stale server; task list stopped retelling the run journal
- [x] v3.3 — Tasks addressable by an id prefix; the fitness gate reads volume stated as a number
- [x] v3.4 — Claude Code plugin (`plugin/`), `--hook` mode inside `au` itself — no bash, no python3 — and an installer that updates instead of skipping
- [x] v3.5 — Dense retrieval: BGE-M3 vectors, RRF fusion with a stemmed full-text index, cross-encoder rerank, one daemon owning the models (spec [011](../specs/011-dense-retrieval/)); reminders with a daemon-owned clock; `au hint --hook` brings memory in on the first edit of a file; `memory_search` reads a whole record by id and answers compactly; `au db prune` and the other dry-run-first maintenance commands; nothing is indexed or written implicitly any more — no auto-indexing, no run or dependency nodes, no default post-commit hook

## Version bump rule

A release bumps `plugin/.claude-plugin/plugin.json` together with `Cargo.toml` — `crates/au/tests/plugin_manifest.rs`
asserts the two versions are equal, so `cargo test` stays red until both are updated.
