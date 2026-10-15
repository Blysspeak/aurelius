# MCP Tools (45)

[Back to README](../README.md)

Every tool the Aurelius MCP server answers, grouped by area.

## Contents

- [Server and tool count](#server-and-tool-count)
- [Knowledge Graph](#knowledge-graph)
- [Task Management](#task-management)
- [Reminders](#reminders)
- [Skills](#skills)
- [Web Search](#web-search)
- [Documents](#documents)
- [Secrets](#secrets)
- [Admin tools](#admin-tools)

## Server and tool count

Aurelius runs as an MCP server over stdio. `install.sh` registers it user-scope with
`claude mcp add -s user aurelius au mcp`, and the same command adds it by hand.

`crates/aurelius/src/mcp/tools.rs` registers 45 tools: the 35 below plus the 10 admin tools.
`tools/list` shows 35 tools. Ten more maintenance tools (see [Admin tools](#admin-tools)) are
left out of the list unless the server runs with `AURELIUS_MCP_ADMIN=1` — every session pays for
the descriptions it is shown, and these are rarely called. A call by name is served either way.

## Knowledge Graph

| Tool | Description |
|------|-------------|
| `memory_status` | Session start — project snapshot with active tasks and a `server` block (running MCP server version, `started_at`, and `restart_needed` when the binary on disk is newer than this running process — installing a new build over the old one doesn't kill an already-running MCP server). Optional `project` filter. Reads only: it never indexes the current directory. **Compact by default:** each item is one line — id, type, date, the claim (or the label when there is none) and `confidence`, plus `stale` only when there is a warning; no note excerpts. Five items per section, task evidence summarized to total/green/last_green and left out when a task has no runs, and a `truncation` block counts what was hidden past the cap — so the first call of a session cannot blow the context on a large graph. `full=true` returns the legacy complete shape. |
| `memory_session` | Session end — save decisions, problems/solutions, next steps. Links to tasks. Returns active tasks hint. SHA-256 dedup. Accepts the same provenance fields as `memory_add` (`confidence`, `evidence`, `subject`, `volatility`, `claim`, `measured_at`, `verify_with`); spawned decisions/problems/solutions inherit confidence/evidence but never subject/claim. |
| `memory_recall` | Smart topic recall — hybrid seeds (see [Search](memory.md#search)) + BFS, grouped by type (incl. tasks), skips structural noise. Ordered by the rank score, not by node degree: a hub node no longer wins a topic just for having many edges. |
| `memory_search` | Hybrid search (full-text + vectors, see [Search](memory.md#search)) with `type`, `since`, and `limit` (default 5) filters. `*` for recent. A query shaped like a subject key (`project:area:thing`) puts the node with exactly that subject first, then the newest member of each `query:…` family. Each hit is a summary — id, type, label, claim, date, confidence, subject, stale — and any field that would be null or empty is left out, on the hit and on the envelope. Pass `id` (a UUID, or the exact `subject` a fact was written with) to read one **whole** record instead — note, data, provenance; an unknown id is a clear miss, never a fuzzy neighbour. `unmatched_terms` names the words that matched nothing, telling "no such knowledge" apart from "the query didn't work"; `vector_notice` says why an answer came from full-text alone. |
| `memory_context` | Raw BFS graph traversal from FTS seed nodes. Capped at 200 nodes and depth 3, and the cut is reported in a `truncation` block — an explicit `depth=2` through a hub node used to expand the answer into megabytes. The same cap applies to `memory_recall`. |
| `memory_path` | Directed step ladder over `next_step`/`prerequisite` edges: shortest path between two nodes, or every node that transitively leads to one target. Same computation as `au path`. |
| `memory_snapshot` | The layered frozen slice as Markdown, under a hard budget. |
| `memory_pickup` | Bounded, ranked payload to rebuild working state after a context wipe — same computation as `au pickup`. |
| `memory_add` | Create node with label, type, note, data (JSON), memory_kind. Pass `project` to link it; without a link the response says so instead of reporting a silent success. |
| `memory_update` | Update existing node's note/data by UUID or label. |
| `memory_relate` | Create typed edge. INSERT OR IGNORE for dedup. |
| `memory_forget` | Delete node by UUID (cascades to edges, its vector and its embedding-queue row). |
| `memory_index` | Index a project directory on request — crates and files from `Cargo.toml`, key files for anything else. The only MCP path that indexes; nothing else does it implicitly. |

## Task Management

| Tool | Description |
|------|-------------|
| `task_create` | Create structured task — title, description, acceptance criteria, priority, subtask/blocking relations. Accepts the same provenance fields as `memory_add` (`confidence`, `evidence`, `subject`, `volatility`, `claim`, `measured_at`, `verify_with`). |
| `task_update` | Update status, priority, criteria. Auto-tracks `started_at`/`completed_at`; `branch`/`worktree` bind the task to a git branch. Accepts the same provenance fields as `memory_add` — a task's confidence can change after a measurement. |
| `task_list` | Filter by project, status, priority. Sorted by priority, shows work log count. **Compact by default:** id, label, status, priority, a short date, a tightly clipped note, work log count, `ripe`, and evidence only when there is any — null or empty fields are left out. `full=true` returns the historical shape (authors, RFC3339 timestamps, wider note budget, every field present). Evidence is an `EvidenceSummary` (total/green/last_green), not the full run array — that stays in `task_view`. `full_notes=true` returns each note whole instead of truncated. |
| `task_log` | Record work done — creates WorkLog + optional Decision/Problem/Solution nodes. Never changes the task's status; the response includes `task_status`, activation is explicit via `task_update status=active`. Accepts the same provenance fields as `memory_add`; spawned decisions/problems/solutions inherit confidence/evidence but never subject/claim. |
| `task_view` | Full task branch — timeline of work logs, decisions, problems, solutions, subtasks. |
| `task_stats` | Task analytics — counts by status/priority, completion rate, avg/median duration, blocked count, oldest active. |
| `task_ripe` | Tasks ready to close — active, with a passing evidence run newer than the last edit, plus the basis (which run, when, files touched). Same computation as `au task ripe`; closing itself is still `task_update`. |
| `task_criterion` | Mark one acceptance criterion met or unmet by its short handle; with neither flag, list the criteria and their handles. Same as `au task criterion`. A record of progress, not a closing condition. |

## Reminders

The reminder model — states, owner, journal, and why no timer lives in the MCP server — is described in [Reminders](reminders.md#model).

| Tool | Description |
|------|-------------|
| `reminder_add` | Set a reminder — exactly one of `due_in` (a delay) or `due_at` (a moment) is required. `owner: ai`/`both` is delivered into a session at the end of a turn or when the owner next types (the next touch, not the exact second); `owner: me`/`both` is the only kind an out-of-session channel can act on. Optional `task` attaches it (inherits the task's project), `repeat` re-arms it after each delivery. |
| `reminder_list` | List reminders, oldest due first. Non-terminal (`pending`/`delivered`) by default; `state` or `include_terminal=true` reaches `done`/`cancelled` too. |
| `reminder_show` | One reminder by id or unique prefix, together with its full journal — every creation, delivery, postponement and re-arm, oldest first. `snooze_count`/`original_due_at` say how many times and from when; the journal says exactly when each move happened. |
| `reminder_done` | Mark a reminder done — distinct from `cancelled` and from `delivered` (shown, not yet resolved). Refuses on one already done or cancelled rather than rewriting that history. |
| `reminder_snooze` | Postpone a reminder — exactly one of `in` (a delay) or `at` (a moment) is required. Recorded as a journal entry, not a silent overwrite; `original_due_at` never moves. |

There is no `reminder_cancel` on this surface — cancelling is CLI-only (`au remind cancel`), the same asymmetry `task_ripe --decline` has with MCP `task_update`.

## Skills

Reusable procedural "how-to" cards with progressive disclosure — the trigger is FTS-indexed (discoverable), the full markdown body is fetched on demand. The index auto-injects every session via a SessionStart hook, and skills surface in `memory_recall`/`memory_status`.

| Tool | Description |
|------|-------------|
| `skill_save` | Create/update a skill (upsert by name). Trigger → indexed note; body + tags → data. |
| `skill_list` | Cheap index (name + trigger + tags + uses), ranked by usage. Optional FTS `query`/`tag`. |
| `skill_get` | Full markdown body by name, with fuzzy FTS fallback + disambiguation. Bumps usage. |

## Web Search

| Tool | Description |
|------|-------------|
| `search_web` | Brave Search API or Perplexity Search API, picked via `provider` (default `brave`). SQLite cache is scoped per provider. Repeat queries served from cache. Optional `save_to_graph`. |
| `search_recall` | FTS search through cached web search results from past sessions. |

## Documents

Word, PowerPoint, Excel, OpenDocument, RTF, EPUB, CSV, PDF, HTML and plain text into GitHub-Flavored Markdown, converted locally — no network, no API key. Everything converted stays searchable afterwards, even once the original file is gone. Out of scope: audio/video transcription and OCR of scans.

| Tool | Description |
|------|-------------|
| `doc_convert` | Convert a file, or every file in a directory, to Markdown. Large output spills to a `.md` file and returns an outline + preview instead of filling the context. Optional `save_to_graph`. |
| `doc_read` | Paginated read of an already-converted document, by content hash or path. |
| `doc_recall` | FTS search across every document ever converted. |

## Secrets

Read-only on MCP, deliberately: Aurelius stores where a secret lives, never the value (see
[Secrets](cli.md#secrets) in the CLI docs), and recording a coordinate is a human act — `au secret add`/`rm`
stay CLI-only. Only the read side is exposed here, because it answers a question an assistant
gets asked directly ("where's the Stripe key?") and the coordinate is otherwise unreachable
through MCP: it never appears in `memory_snapshot` or any other automatic dump.

| Tool | Description |
|------|-------------|
| `secret_list` | Name, purpose, and location of every recorded secret coordinate — never the value. |

## Admin tools

Hidden from `tools/list` unless `AURELIUS_MCP_ADMIN=1`; still served when called by name.

| Tool | Description |
|------|-------------|
| `memory_consolidate` | Rebuild a project's distillate — open next steps plus unsolved problems. Idempotent. |
| `memory_gc` | Garbage collection — duplicate edges/nodes, orphans; leaves no vector behind. |
| `memory_merge` | Merge two near-duplicate nodes — rewires edges, merges notes, deletes source. |
| `memory_dump` | Paginated graph export (offset/limit). |
| `memory_journal` | What a given run wrote — same as `au journal`. |
| `memory_eval` | Score recall against a frozen fixture — same as `au eval`. |
| `skill_remove` | Delete a skill by name. |
| `db_check` | Integrity check, read-only — same as `au db check`. |
| `db_backup` | Safe `VACUUM INTO` snapshot — same as `au db backup`. |
| `db_reindex_embeddings` | Queue nodes missing a vector for the daemon to embed; returns at once, computes nothing itself. |
