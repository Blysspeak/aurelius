# Architecture and traps

A knowledge graph stored in SQLite: nodes and typed edges, full-text plus vector search, tasks, reminders, skill cards. Where the data lives: `au home current`.

## Three processes, and their roles do not overlap

**Daemon** (`au daemon`; the systemd user unit `contrib/systemd/aurelius-remind.service` in the aurelius repo runs it): the sole owner of the reminder clock and the only process that loads the embedding model and the reranker. It serves a query vector over the `embed.sock` socket in the data directory and drains the vectorization queue in batches of 16 per tick (every 60 s by default), after delivering that tick's reminders. Which device the models use is `AURELIUS_EMBED_DEVICE`: `auto` tries CUDA first, proven by one real run, and falls back to CPU; `cpu` skips CUDA. A GPU is not required.

**MCP server** (`au mcp`): the tools for a session, 52 registered and 42 listed by default (admin tools are served by name, or listed with `AURELIUS_MCP_ADMIN=1`). One server is started per session; it loads no model weights.

**CLI** (`au`): for a human and for scripts. Short-lived, loads no weights.

Models and their variables: see the "Models" section of the skill's SKILL.md.

## What to use

| Task | MCP tool | CLI |
|---|---|---|
| Session start | `memory_status` | `au snapshot -p myapp` |
| After a context reset | `memory_pickup` | `au pickup -p myapp` |
| Find by meaning | `memory_search` (hybrid: full-text + vectors fused by RRF) | `au search` |
| Fetch by exact key (UUID or subject, no fuzzy matching) | `memory_search` with `id` | `au recall` |
| Context around a topic | `memory_context` | `au context` |
| Record a fact | `memory_add` with `confidence` and `evidence` | `au note` |
| Link two nodes | `memory_relate` | `au relate` |
| Session summary | `memory_session` with `decisions` and `next_steps` | `au session` |
| Tasks | `task_create`, `task_update` (`status=done` closes), `task_log`, `task_criterion`, `task_view`, `task_ripe` | `au task ...` |
| Check the database | `db_check` (admin tool) | `au db check` |
| Safe copy of the database | `db_backup` (admin tool) | `au db backup` |
| Search quality | `memory_eval` (admin tool) | `au eval`, `au eval-search` |

A misspelled MCP parameter name stops the whole call before the handler runs, with an error naming the closest known parameter; nothing is written.

## What to write

Only what is not recoverable anywhere else; see `what-not-to-write.md`. Provenance is mandatory: `measured` requires a command in `evidence`; everything else is marked as unverified on output.

Nothing is indexed behind your back: no `au` command and no `memory_status` call turns the current directory into a project. A repository enters the graph only through an explicit `au reindex` (MCP `memory_index`).

## Traps

**Only the daemon loads the models.** A second process that loads bge-m3 competes with the daemon for video memory, and the loader falls back to CPU when CUDA fails (inference from the loader code). A symptom to look for: a process holding many cores near a hundred percent.

**`au db reindex-embeddings` computes nothing.** It marks the nodes that lack a vector and returns in milliseconds; the daemon does the computing on its ticks.

**Schema version and binaries.** A migration is applied when a newer binary opens the database. After that every older binary (the daemon, MCP servers of open sessions, hooks) refuses the database with `SchemaTooNew`. Keep every copy of `au` and `aurelius` on the machine at one version (`which -a au`); the shipped systemd unit starts `~/.local/bin/au`. After an upgrade restart the daemon and the Claude Code sessions: `memory_status` reports `server.restart_needed` when the binary on disk is newer than the running MCP server.

**Keys share one space.** `au note --key K` looks the key up without a type filter and rewrites the node it finds; a repeat call keeps the node's type unless `--type` is passed explicitly. Prefix keys with the project and area (`myapp:precompact:<session>`).

**Deleting a node** (`memory_forget`) marks it deleted, soft-deletes its edges and drops its vector. Vectors orphaned by older versions: `au db purge-orphans` counts them, `--apply` removes them. The number of vectors should equal the number of live nodes.

**Do not copy a live database with `cp`, `mv` or `rsync`.** Use `au db backup` (SQLite `VACUUM INTO`); a plain file copy over the WAL breaks the database. The SessionStart hook keeps a rolling set of snapshots (`au db backup --hook`, 7 kept, at most one per 24 h).

**Notes are stored verbatim.** There is no secret scanning on `au note` or `memory_add`; only action-trace payloads are masked. A token written into `--evidence` stays in the database and in every backup. Secrets are stored as coordinates, never values: `au secret add --name STRIPE_KEY --where "env:STRIPE_KEY" --purpose "charge webhooks" --project myapp`.
