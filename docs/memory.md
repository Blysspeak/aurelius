# Memory

[Back to README](../README.md)

How Aurelius hands memory to a session (the snapshot), which nodes belong to a project, and how search finds them.

## Contents

- [Memory Snapshot](#memory-snapshot)
- [Active tasks](#active-tasks)
- [Commands](#commands)
- [Machine-readable form](#machine-readable-form)
- [What counts as "belonging to a project"](#what-counts-as-belonging-to-a-project)
- [Search](#search)
- [Retrieval path](#retrieval-path)
- [Embedding daemon](#embedding-daemon)
- [Model files and settings](#model-files-and-settings)
- [Measuring search quality](#measuring-search-quality)

## Memory Snapshot

The snapshot is a small curated slice of the graph, meant to be injected **once** at
session start (`au snapshot --hook`) — frozen for the session, so it does not break the
prefix cache — rather than a large JSON blob fetched on demand. The plugin does not
register that hook (see [Hooks](hooks.md)); a hand-written `settings.json` can.

```
Repository                  where the session stands: branch, changed paths, last commits
Owner                       what is known about you
In progress                 open tasks and unsolved problems
Pressure                    obligations taken on and not yet settled
Recent sessions             what the latest session concluded
Decisions and knowledge     decisions, then concepts
Distillate                  the latest session's next steps plus unsolved problems, until something newer is recorded
```

Every layer has a hard budget in **bytes**, not characters — the snapshot is paid for on every
request of the session, and Cyrillic costs twice as much as Latin — and the whole slice stays
under 3850 bytes. An empty layer is not printed at all; layers are numbered among the printed
ones. Past the total budget whole layers are dropped, least needed first: Distillate, Pressure,
Decisions and knowledge, Recent sessions. Repository, Owner and In progress are never dropped.
The Repository layer reads local git only (`rev-parse`, `status`, `log`, each under a 1.5 s
timeout, no fetch), and only for a repository named like the project. Without a project the
slice covers all of memory and says so under its heading. Skills are not a layer: the plugin's
SessionStart hook prints the skill index on its own.

### Active tasks

**Active tasks are the one exception.** A task actually in progress — status `active` —
is pulled out of "In progress" before the budget is applied and rendered in full, uncut by the
per-layer limit that trims everything else. There is still a hard ceiling (20
active tasks per project) purely against unbounded growth; past it the snapshot says how
many did not fit rather than dropping them silently. Whatever an active task's own
rendering costs beyond the layer's normal budget is taken from **Decisions and knowledge**,
not from the active tasks themselves — the layer below "In progress" in priority pays for it.
In a slice without a project, where every project contributes its own active task, they go
under the ordinary layer budget and the rest are counted.

### Commands

```bash
au snapshot --project myapp          # Markdown, for humans and for context
au snapshot                          # project = current folder name
au snapshot --project myapp --json   # fixed shape, for programs
au snapshot --hook                   # Claude Code SessionStart envelope — the flag stays,
                                     # but the plugin no longer registers it (see Hooks)
```

### Machine-readable form

Hooks are ordinary processes — they cannot reach the MCP server, so the CLI is their
only channel. Parsing the Markdown means depending on its layout, and a layout change
would break the consumer silently. `--json` fixes the shape instead:

```json
{"project":"myapp","facts":[{"kind":"decision","text":"chose SQLite over Postgres","at":"2026-08-15T21:30:40Z"}]}
```

`kind` names the source layer: `userfact`, `active_task`, `task`, `problem`, `obligation`,
`session`, `decision`, `concept`, `skill`, `digest`. `active_task` is the guaranteed,
uncut form of a task in progress described above; plain `task` covers everything else
still open (`backlog`, `blocked`) and is subject to the ordinary budget. Text is returned
whole — the budget belongs to the consumer, and a silently shortened fact reads exactly
like a short one.

The contract distinguishes the two states that a silent channel confuses:

| Result | Meaning |
|---|---|
| `{"project":…,"facts":[]}`, exit 0 | nothing to say |
| no output, or a non-zero exit | broken |

### What counts as "belonging to a project"

A node belongs to a project if **either** holds:

- its label carries the `[project-name]` prefix, or
- an edge connects it to that project's node, in either direction and under any relation.

Both are checked by a single predicate. Only the first used to be, which made every node
written through `memory_add` + `memory_relate` invisible to project-scoped queries —
including the snapshot itself. If you are upgrading from ≤ 1.10.0, that knowledge becomes
visible again with no re-import.

## Search

### Retrieval path

`memory_search`, `memory_recall`, `memory_context`, `au search` and `au context` share one
retrieval path with two halves:

- **Full-text** — SQLite FTS5 over label + note, plus a stemmed index (`nodes_stem_fts`,
  Snowball: Russian for Cyrillic words, English otherwise), so another case or number of the
  same word still matches.
- **Dense** — BGE-M3 vectors (1024-d, stored as int8 in `node_embeddings`).

The two ranked lists are fused with Reciprocal Rank Fusion (k = 12). A node whose `subject`
equals the query byte for byte comes first. When a reranker is available, the top 30 fused
candidates are re-ordered by the `bge-reranker-v2-m3` cross-encoder; a search waits at most
1.5 s for its scores and otherwise keeps the fused order.

### Embedding daemon

**The daemon owns the models.** `au daemon` (the same process that owns the reminder clock, see
[Reminders](reminders.md#the-daemon)) is the only process that loads BGE-M3 and the reranker.
It serves query vectors and rerank scores over a Unix socket, `embed.sock`, next to the database.
Writing a node only queues it in `embedding_queue`; the daemon embeds queued nodes in batches on
its ticks. `au db reindex-embeddings` queues every live node that still lacks a vector.

Without the daemon, search does not fail: it answers from full-text alone, and the JSON answer
carries a `vector_notice` saying why.

### Model files and settings

What the daemon needs on disk (nothing here is downloaded by `install.sh`):

- ONNX Runtime: `libonnxruntime.so` under `<data-dir>/onnxruntime/`, or the path in `ORT_DYLIB_PATH`.
- BGE-M3 weights: a `models--BAAI--bge-m3` directory under `<data-dir>/models/`, or under
  `AURELIUS_MODELS_DIR`. The reranker weights are fetched into the same directory on first load.

`<data-dir>` is the directory holding `aurelius.db` (`au home current`).

| Variable | Default | Meaning |
|---|---|---|
| `AURELIUS_EMBED_DEVICE` | `auto` | `auto`: CUDA first, proven by one real run, CPU when CUDA can't run a kernel. `cpu`: skip CUDA |
| `AURELIUS_EMBED_PRELOAD` | on | load BGE-M3 right after the daemon starts; `0` loads it on the first request |
| `AURELIUS_EMBED_IDLE_SECS` | `0` | unload BGE-M3 after this many idle seconds; `0` keeps it resident |
| `AURELIUS_RERANK` | `auto` | `auto`: rerank on CUDA only, and not at all when `AURELIUS_EMBED_DEVICE=cpu`. `cuda`: rerank on CUDA whatever the embed device — bge-m3 on the CPU, the reranker alone on the card. `cpu`: allow CPU (one rerank of 30 documents takes seconds there). `off`: never |
| `AURELIUS_RERANK_MODEL` | unset | directory with a reranker to load instead of the built-in bge-reranker-v2-m3 (`model.onnx`, `tokenizer.json`, `config.json`, `special_tokens_map.json`, `tokenizer_config.json`). `scripts/export-reranker-en-ru.py` builds the FP16 English+Russian variant: 1.3 GiB of the card instead of 3.5 |
| `AURELIUS_RERANK_IDLE_SECS` | `600` | unload the reranker after this many idle seconds; `0` keeps it resident |
| `AURELIUS_MODELS_DIR` | `<data-dir>/models` | where model weights live |

### Measuring search quality

`au eval-search` measures the path: recall@5, recall@10 and MRR@10 over a reference query set
(`fixtures/eval/search-baseline.jsonl`), split by query class (ru, en, cross-language, key), for
FTS, dense, RRF and RRF + rerank. Read-only; without the daemon only the FTS row is measured,
the others are skipped with the reason rather than failed.
