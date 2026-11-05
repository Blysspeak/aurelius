---
name: aurelius
description: Memory between sessions (aurelius). Load at the start of a session, after /clear, and before any mcp__aurelius__* call or `au` command. Short guide plus sections - write a fact, search, tasks, sessions, what never to write, architecture and traps, agent checkpoints, your own cards.
---

# aurelius

aurelius is a knowledge graph in SQLite: facts, decisions, problems, tasks, sessions, reminders. It survives /clear, so the next session picks up where the last one stopped instead of re-reading the repository.

Two ways in, one database: MCP tools `mcp__aurelius__*` for the agent (52 registered, 42 listed by default; the 10 admin tools are served by name, or listed when the server runs with `AURELIUS_MCP_ADMIN=1`) and the `au` CLI for a human or a script. The reference files show CLI commands and name the MCP tool with the same meaning. Flags change between versions: `au <command> --help` wins over any file here.

## The loop

1. Session start: `memory_status` (pass `project`). After /clear: `memory_pickup` (or the `/aurelius:pickup` command). Before touching past work: `memory_search`.
2. While working: write a fact the moment it happens (`memory_add`), one fact per record. `confidence` is required; `measured` needs the command in `evidence`.
3. "Do later": `task_create` with acceptance criteria, then `task_log` / `task_update` as you go.
4. End of session, or right before a compact: `memory_session` with decisions and next steps. A session record, not a plain fact.

## What to write

Only what cannot be recovered from code, git or command output: why an option was chosen, what was rejected and why, how a trap looked from the outside, a value tuned by experiment. Not: a retelling of a diff, "tests passed", progress reports, file contents, secret values. Notes are stored verbatim and nothing scrubs them.

## Models (Hugging Face)

| Role | Model | How it gets there |
|---|---|---|
| Embeddings (vector search) | [BAAI/bge-m3](https://huggingface.co/BAAI/bge-m3) | not downloaded for you: a hub-cache folder `models--BAAI--bge-m3` under `<data-dir>/models/` (`au home current` prints `<data-dir>`; `AURELIUS_MODELS_DIR` overrides), e.g. `hf download BAAI/bge-m3 --cache-dir <data-dir>/models`; also ONNX Runtime (`libonnxruntime.so` in `<data-dir>/onnxruntime/` or `ORT_DYLIB_PATH`) |
| Reranker (re-orders the top 30 hits) | [qilowoq/bge-reranker-v2-m3-en-ru](https://huggingface.co/qilowoq/bge-reranker-v2-m3-en-ru) | bge-reranker-v2-m3 with the vocabulary cut to English and Russian (375M parameters instead of 568M, same scores on the repo's control queries); `scripts/export-reranker-en-ru.py` in the aurelius repo exports FP16 ONNX, then `AURELIUS_RERANK_MODEL=<that folder>` and `AURELIUS_RERANK=cuda` |
| Reranker, built-in fallback | [BAAI/bge-reranker-v2-m3](https://huggingface.co/BAAI/bge-reranker-v2-m3), ONNX from [rozgo/bge-reranker-v2-m3](https://huggingface.co/rozgo/bge-reranker-v2-m3) | used when `AURELIUS_RERANK_MODEL` is unset; fetched into the models folder on first load |

Only the daemon (`au daemon`) loads models, so the variables go into the daemon's environment (its systemd unit), not your shell. The device is `AURELIUS_EMBED_DEVICE`: `auto` tries CUDA, proven by one real run, and falls back to CPU; `cpu` skips CUDA. A GPU is not required.

Both models are optional. Without bge-m3 the daemon logs `vector search off until restart` and `memory_search` answers from full-text alone, with a `vector_notice` saying so. Facts, tasks, sessions and the snapshot never touch the models. Put the weights in place and restart the daemon to get vectors back.

No GPU: set `AURELIUS_EMBED_DEVICE=cpu`. The reranker is then off unless `AURELIUS_RERANK=cpu`: one rerank of 30 documents takes seconds on CPU, and a search waits at most 1.5 s for scores before keeping the fused order.

## Sections

| Section | Read when |
|---|---|
| [architecture-and-traps](reference/architecture-and-traps.md) | first time: the three processes, which tool for which job, traps |
| [what-not-to-write](reference/what-not-to-write.md) | before any write: does this belong in memory at all |
| [write-fact](reference/write-fact.md) | recording a fact with full provenance (confidence, evidence, subject, volatility) |
| [search](reference/search.md) | finding something: search vs context vs recall vs journal |
| [tasks](reference/tasks.md) | a task through its life: activate, log, evidence, ripe, done, block |
| [sessions](reference/sessions.md) | end of a session, or a handoff for the next one |
| [agent-checkpoint](reference/agent-checkpoint.md) | a delegated agent finished an expensive step: checkpoint before returning |

## Your own cards

Reusable how-to cards (a release checklist, a deploy order, a repair procedure) live in the aurelius database, not in this plugin; a fresh database has none.

- Save: `skill_save(name, trigger, body, tags)`. `name` is kebab-case and is the lookup key; `trigger` is one or two sentences on when to apply the card (the full-text index matches on it); `body` is the markdown. Same name again updates the card. There is no CLI for saving.
- Find: `skill_list(query?, tag?)` returns names, triggers, tags and use counts without bodies, most used first. `skill_get(name)` returns the body (fuzzy fallback; `matched_by` says which) and bumps the use count. `skill_remove(name)` is an admin tool.
- Session start: the hook `au skills --hook` puts the card names (no triggers) into context; with no cards it prints nothing. `au skills` prints names with triggers and tags.
- Save a card when you have worked out a repeatable procedure that is longer than a fact: steps, commands, traps. A single lesson is a fact (`write-fact`), not a card.
