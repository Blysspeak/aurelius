# MCP Tools (52)

[Back to README](../README.md)

Every tool the Aurelius MCP server answers, grouped by area: what it is for, its parameters, what
it returns, and one example call.

## Contents

- [Server and tool count](#server-and-tool-count)
- [Calling conventions](#calling-conventions)
- [Knowledge Graph](#knowledge-graph)
- [Task Management](#task-management)
- [Reminders](#reminders)
- [Finance](#finance)
- [Skills](#skills)
- [Web Search](#web-search)
- [Documents](#documents)
- [Secrets](#secrets)
- [Admin tools](#admin-tools)
- [Regenerating the tables](#regenerating-the-tables)

## Server and tool count

Aurelius runs as an MCP server over stdio (`au mcp`). `install.sh` registers it user-scope with
`claude mcp add -s user aurelius au mcp`, and the same command adds it by hand; the tools then
appear to Claude Code as `mcp__aurelius__<name>`.

`crates/aurelius/src/mcp/tools.rs` registers **52 tools**. `tools/list` shows **42** of them. The
other **10** are [admin tools](#admin-tools), named in `ADMIN_TOOLS` in
`crates/aurelius/src/mcp/mod.rs`: they are left out of the list unless the server runs with
`AURELIUS_MCP_ADMIN=1`, because every session pays for the descriptions it is shown and these are
rarely called. A call by name is served either way.

| Group | Listed | Admin |
|-------|-------:|------:|
| [Knowledge Graph](#knowledge-graph) | 13 | 6 |
| [Task Management](#task-management) | 8 | — |
| [Reminders](#reminders) | 5 | — |
| [Finance](#finance) | 7 | — |
| [Skills](#skills) | 3 | 1 |
| [Web Search](#web-search) | 2 | — |
| [Documents](#documents) | 3 | — |
| [Secrets](#secrets) | 1 | — |
| Database | — | 3 |
| **Total** | **42** | **10** |

## Calling conventions

- **Examples** below show the `arguments` object of a `tools/call` request. Every example was run
  against a scratch database (`AURELIUS_HOME=/tmp/...`) with `au` 3.8.0; ids in them are
  placeholders.
- **Results** are one text content item holding a JSON object, except where noted. Field lists
  under "Returns" name the top-level keys.
- **Unknown parameters are refused.** A misspelled parameter name stops the whole call before the
  handler runs, with a tool error (`isError: true`) that names the closest known parameter —
  nothing is written. Example: `memory_add` with `confidnce` answers
  `memory_add: неизвестные параметры: 'confidnce' (похоже на 'confidence')…`.
- **Tool errors** (bad values, missing nodes) come back as `isError: true` with a text starting
  `Error:`, not as JSON-RPC protocol errors, so the model sees them and can call again.
- **Provenance fields** (`confidence`, `evidence`, `measured_at`, `claim`, `volatility`,
  `verify_with`, `subject`, `resolution`) mean the same on every tool that takes them; they are
  defined once in [Provenance](cli.md#provenance).
- **Node selectors** (`id`, `task`, `from`, `to`) accept a UUID, a unique id prefix of 8+
  characters for tasks, or an exact label, unless the table says otherwise.

## Knowledge Graph

### memory_status

Session start: a project snapshot with active tasks, recent decisions, problems, solutions and
sessions, skills, and a `server` block. Reads only — it never indexes the current directory.
Compact by default: five items per section, notes as excerpts, task evidence summarized, and a
`truncation` block counting what the cap hid.

<!-- params:memory_status -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | no | Filter by project name (e.g. 'aurelius'). Shows only decisions, problems, sessions for this project. |
| `full` | boolean | no | Return the full legacy response: complete notes and every field on every item. Default false — compact excerpts. |
<!-- /params -->

**Returns:** `server` (`version`, `started_at`, `restart_needed` — true when the binary on disk is
newer than the running process), `summary` (node and edge totals), `projects`, `crates`,
`active_tasks`, `recent_decisions`, `recent_sessions`, `recent_solutions`, `open_problems`,
`skills`, `project_filter`, `truncation`.

```json
{"project": "aurelius"}
```

### memory_context

Raw breadth-first walk of the graph from full-text seed nodes. Capped at 200 nodes and depth 3;
the cut is reported in `truncation`.

<!-- params:memory_context -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `topic` | string | yes | Topic to search for and expand context around |
| `depth` | integer | no | BFS traversal depth (default: 1). Depth 2+ walks through project hub nodes and back out onto every unrelated task/decision that shares the project — expect a much larger, noisier result. Default: `1`. |
| `limit` | integer | no | Max nodes to return (default: 50). Seeds first, then by BFS depth. Response's `truncation` field says how many were hidden. Default: `50`. |
<!-- /params -->

**Returns:** `topic`, `depth`, `nodes`, `edges` (`from`, `to`, `relation`, `weight`), `returned`,
`total`, `truncation`, and `vector_notice` when the vector half of the search was unavailable.

```json
{"topic": "sync server", "depth": 2, "limit": 20}
```

### memory_path

Directed step ladder over `next_step`/`prerequisite` edges: the shortest path between two nodes,
or every node that transitively leads to one target (`before`), earliest first. Same computation
as `au path`.

<!-- params:memory_path -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `from` | string | no | Start node selector. Required together with 'to'; mutually exclusive with 'before'. |
| `to` | string | no | End node selector. Required together with 'from'; mutually exclusive with 'before'. |
| `before` | string | no | Target node selector. Returns every ancestor instead of a from/to path; mutually exclusive with 'from'/'to'. |
| `max_depth` | integer | no | Walk depth cap (default: 50) Default: `50`. |
<!-- /params -->

**Returns:** `steps` — `index`, `id`, `label`, `subject`, `relation` (the edge into this step),
`weight`. A missing path is `{"error": …}` in a normal result, not a tool error.

```json
{"from": "<uuid of the first node>", "to": "<uuid of the last node>"}
```

### memory_search

Hybrid search (full-text plus vectors, see [Search](memory.md#search)); `*` or an empty query
lists recent nodes. A query shaped like a subject key (`project:area:thing`) puts the node with
exactly that subject first. Pass `id` (a UUID, or the exact `subject`) to read one whole record
instead; an unknown id is a clear miss, never a fuzzy neighbour.

<!-- params:memory_search -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `query` | string | no | FTS5 search query. Use empty string or '*' to list most recent nodes. Ignored when 'id' is given. |
| `id` | string | no | Node UUID, or the exact subject a fact was written with. When set, returns that one whole record (note, data, provenance, created_at, confidence, subject) and ignores 'query'; an unknown id or subject is a clear miss, never a fuzzy match. |
| `type` | string | no | Filter by node type: decision, problem, solution, session, concept, project, crate, file, dependency |
| `since` | string | no | Time filter: 'today', 'yesterday', '7d', '24h', or ISO 8601 datetime |
| `limit` | integer | no | Maximum results (default: 5, same as `au search`). Pass a larger value for more; it is honoured as asked Default: `5`. |
| `session_id` | string | no | Calling session's identity, used for query-correction telemetry (which pathways this session's recalls take). Defaults to a placeholder when omitted. |
<!-- /params -->

**Returns:** for a query — `query`, `count`, `route`, `results` (each: `id`, `type`, `label`,
`claim`, `created_at` date, `confidence`, `subject`, `stale`, `window`; empty fields left out),
`unmatched_terms` and `query_hint` when words matched nothing, `vector_notice` when the answer came
from full-text alone. For `id` — the whole record: `note`, `data`, `provenance`, `created_at`,
`created_by`, `access_count`.

```json
{"query": "sync server port", "limit": 5}
```

```json
{"id": "aurelius:sync-server:port"}
```

### memory_add

Create one knowledge node. Pass `project` to link it: without a link the node is invisible to
`memory_status(project=…)` and the snapshot, and the response says so in `attachment_warning`.

<!-- params:memory_add -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `label` | string | yes | Short label |
| `project` | string | no | Owning project (creates belongs_to edge). Omit only for global knowledge. |
| `type` | string: `project`, `decision`, `concept`, `problem`, `solution`, `person`, `dependency`, `server`, `file`, `module`, `crate`, `config`, `session`, `language`, `task`, `work_log`, `skill`, `user_fact`, `digest`, `doc`, `run` | no | Node type. Default: `"concept"`. |
| `note` | string | no | Detailed note |
| `source` | string | no | Source (default: mcp) Default: `"mcp"`. |
| `data` | object | no | Arbitrary JSON metadata |
| `memory_kind` | string: `semantic`, `episodic` | no | semantic (facts) or episodic (events) Default: `"semantic"`. |
| `session_id` | string | no | Writing run, for `au journal --session <id>`. |
| `confidence` | string: `measured`, `inferred`, `reported`, `unverified` | yes | REQUIRED origin. measured: from a command/query quoted verbatim in 'evidence'. inferred: derived from a measurement. reported: told by a human/docs, unchecked. unverified: origin unnamed. Non-measured is flagged on output. |
| `evidence` | string | no | The producing command/query VERBATIM. Required when confidence is 'measured'. |
| `measured_at` | string | no | RFC 3339 time of measurement; defaults to now for 'measured'. |
| `claim` | string | no | The assertion, returned whole. Max 240 chars; reasoning goes in 'note'. |
| `volatility` | string: `immutable`, `slow`, `volatile` | no | immutable (commit sha), slow (DB schema), volatile (.env value). Stale facts return with a re-check note. Omit if unknown. |
| `verify_with` | string | no | Command that re-checks the claim. |
| `subject` | string | no | Key of the asserted thing, e.g. 'xhub:.env:FLAG'. A second fact on the same subject is refused unless 'resolution' is given. |
| `resolution` | string: `supersede`, `refine`, `coexist` | no | Vs the fact on this subject. supersede: old is false (supersedes edge). refine: more precise. coexist: both hold. |
<!-- /params -->

**Returns:** `id`, `label`, `type`, `created`, `project`, `memory_kind`, `confidence`,
`confidence_downgraded`, `stored_fields`, `dropped_fields`, `attachment_warning`,
`probe_warnings`, `subject`, `subject_peer`, `resolution`, `resolved_against`, `surprise`.

```json
{
  "label": "Sync server listens on 8787",
  "project": "aurelius",
  "type": "config",
  "confidence": "measured",
  "evidence": "ss -ltnp | grep 8787",
  "claim": "aurelius-sync-server listens on port 8787",
  "subject": "aurelius:sync-server:port",
  "volatility": "slow"
}
```

### memory_relate

Create a typed edge between two nodes. A repeated edge is ignored, not duplicated.

<!-- params:memory_relate -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `from` | string | yes | Label or ID of the source node |
| `to` | string | yes | Label or ID of the target node |
| `relation` | string | yes | Relation type: uses, depends_on, solves, caused_by, inspired_by, conflicts_with, supersedes, refines, belongs_to, related_to, learned_from, contains, imports, exports, implements, configures, tracked_by, subtask_of, blocks, verified_by, prerequisite, next_step, defines, references |
| `weight` | number | no | Edge weight (default: 1.0) Default: `1.0`. |
<!-- /params -->

**Returns:** `id`, `created`, `from`, `to` (labels), `relation`.

```json
{"from": "<uuid>", "to": "<uuid>", "relation": "next_step"}
```

### memory_index

Index a project directory on request — crates and files from `Cargo.toml`, key files for anything
else. The only MCP path that indexes; nothing else does it implicitly.

<!-- params:memory_index -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `path` | string | yes | Absolute path to the project root |
<!-- /params -->

**Returns:** `project`, `crates_found`, `files_indexed`, `nodes_created`, `nodes_updated`,
`nodes_removed`.

```json
{"path": "/home/me/src/aurelius"}
```

### memory_update

Replace the note and/or data of an existing node, by UUID or label.

<!-- params:memory_update -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | UUID or label of the node to update |
| `note` | string | no | New note text (replaces existing) |
| `data` | object | no | New JSON metadata (replaces existing) |
<!-- /params -->

**Returns:** `id`, `label`, `updated`.

```json
{"id": "<uuid>", "note": "Checked after the 3.8.0 upgrade."}
```

### memory_session

Session end: an episodic Session node linked to the project, plus Decision and Problem/Solution
nodes, optional task links, and the project's active tasks back. Deduplicated by SHA-256 of the
content. Spawned nodes inherit `confidence`/`evidence` but never `subject`/`claim`; `resolution`
is not accepted here.

<!-- params:memory_session -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `summary` | string | yes | Brief summary of what was accomplished this session |
| `project` | string | yes | Project name (used for linking and labeling) |
| `decisions` | array of string | no | List of decisions made and their reasoning |
| `problems_solved` | array of object | no | List of problem/solution pairs encountered |
| `problems_solved[].problem` | string | no |  |
| `problems_solved[].solution` | string | no |  |
| `next_steps` | array of string | no | What should be done next (carried forward to future sessions) |
| `key_files` | array of string | no | Key files that were modified or are relevant |
| `session_id` | string | no | The run that wrote this record. Stamps the session node and everything it spawns, so `au journal --session <id>` can list them back. |
| `tasks` | array of string | no | UUIDs or labels of tasks worked on during this session (creates related_to edges) |
| `confidence` | string: `measured`, `inferred`, `reported`, `unverified` | no | Required. Provenance field, defined in memory_add. |
| `evidence` | string | no | As in memory_add. |
| `measured_at` | string | no | As in memory_add. |
| `claim` | string | no | As in memory_add. |
| `volatility` | string: `immutable`, `slow`, `volatile` | no | As in memory_add. |
| `verify_with` | string | no | As in memory_add. |
| `subject` | string | no | As in memory_add. |
<!-- /params -->

**Returns:** `id`, `label`, `type`, `created`, `complete`, `decisions_written`, `problems_written`,
`linked_tasks`, `unresolved_tasks`, `active_tasks_hint`, `rejected` (child nodes not accepted,
each named), `stored_fields`, `dropped_fields`, `provenance`.

```json
{
  "summary": "Documented the finance module",
  "project": "aurelius",
  "confidence": "reported",
  "decisions": ["Rates convert at query time"],
  "next_steps": ["Open the docs PR"],
  "key_files": ["docs/finance.md"]
}
```

### memory_recall

Smart topic recall: full-text seeds, a breadth-first walk, then records ranked by edges inside
the found subgraph and by recency — not by node degree. Node bodies are not returned; skill cards
are excluded.

<!-- params:memory_recall -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `topic` | string | yes | Topic to recall knowledge about |
| `depth` | integer | no | BFS traversal depth (default: 2 — depth 1 stops before the project hub node and answers with leaves). The walk is capped at 200 nodes regardless, and a cut answer says so in `truncation`. Default: `2`. |
<!-- /params -->

**Returns:** `topic`, `knowledge` (up to 12 lasting records: `id`, `type`, `label`, `claim` or a
`window` around the match, `subject`, `confidence`, `created_at`), `recent` (at most 2 episodic
records), `matched_knowledge`, `matched_recent`, `shown`, `total_graph_nodes`, `truncation`,
`vector_notice`.

```json
{"topic": "sync server"}
```

### memory_forget

Delete a node by UUID; its edges, vector and embedding-queue row go with it.

<!-- params:memory_forget -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | UUID of the node to delete |
<!-- /params -->

**Returns:** `id`, `deleted`.

```json
{"id": "<uuid>"}
```

### memory_snapshot

The seven-layer frozen slice of memory as compact Markdown under a hard budget (see
[Memory snapshot](memory.md#memory-snapshot)). Read-only and instant; the SessionStart hook
injects the same thing through `au snapshot --hook`.

<!-- params:memory_snapshot -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | no | Project scope (label prefix). Omit for global snapshot. |
| `consolidate` | boolean | no | Rebuild the project digest (layer 7) before building the snapshot |
<!-- /params -->

**Returns:** `project`, `markdown`.

```json
{"project": "aurelius"}
```

### memory_pickup

Bounded, ranked payload to rebuild working state after a context wipe — same computation as
`au pickup`.

<!-- params:memory_pickup -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | yes | Project to rebuild. Required: an anchor facet makes no sense without a scope to compute it in. |
<!-- /params -->

**Returns:** `anchor` (facet), `tail` (freshest session with non-empty `next_steps`), `facets`,
`records` (up to 8 open records under the anchor), `critical` (open critical-priority tasks);
each list carries `truncated`.

```json
{"project": "aurelius"}
```

## Task Management

How tasks move through their lifecycle is described in [Task Management](tasks.md).

### task_create

Create a task (title, description, acceptance criteria, priority), linked to its project;
supports subtasks (`parent`) and blocking (`blocks`). Created in `backlog`.

<!-- params:task_create -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | Short, actionable task title |
| `description` | string | no | Detailed description — what needs to be done and why |
| `project` | string | no | Project name (auto-creates if missing). Default: 'unknown' |
| `priority` | string: `critical`, `high`, `medium`, `low` | no | Task priority (default: medium) Default: `"medium"`. |
| `acceptance_criteria` | array of string | no | Definition of Done checklist — what must be true for this task to be complete |
| `parent` | string | no | UUID or label of parent task (creates subtask_of edge) |
| `blocks` | array of string | no | UUIDs or labels of tasks that this task blocks |
| `confidence` | string: `measured`, `inferred`, `reported`, `unverified` | no | Required. Provenance field, defined in memory_add. |
| `evidence` | string | no | As in memory_add. |
| `measured_at` | string | no | As in memory_add. |
| `claim` | string | no | As in memory_add. |
| `volatility` | string: `immutable`, `slow`, `volatile` | no | As in memory_add. |
| `verify_with` | string | no | As in memory_add. |
| `subject` | string | no | As in memory_add. |
<!-- /params -->

**Returns:** `id`, `label` (`[project] title`), `status`, `priority`, `project`, `created`,
`created_by`, `provenance`.

```json
{
  "title": "Write docs/finance.md",
  "project": "aurelius",
  "priority": "high",
  "acceptance_criteria": ["Entry model described", "au fin examples"]
}
```

### task_update

Change status, priority, acceptance criteria or note. `active` stamps `activated_at` and binds the
task to the current git branch and worktree; other active tasks of the project stay active.
`done` stamps `closed_at` and builds the resolution like `au task done` — commit from git HEAD
unless given, files from edits traced since activation.

<!-- params:task_update -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | UUID or label of the task to update |
| `status` | string: `backlog`, `active`, `blocked`, `done`, `cancelled` | no | New task status |
| `priority` | string: `critical`, `high`, `medium`, `low` | no | New priority |
| `blocked_by` | string | no | Reason for blocking (auto-sets status to 'blocked') |
| `note` | string | no | Update task description/notes |
| `acceptance_criteria` | array of string | no | Replace acceptance criteria checklist |
| `commit` | string | no | Only with status='done'. Commit that resolves the task — refines the resolution; if omitted, the commit is read from the repo's current HEAD automatically |
| `pull_request` | string | no | Only with status='done'. Pull request that resolves the task — refines the auto-collected resolution |
| `unconfirmed` | boolean | no | Only with status='done'. Force-mark the resolution as unconfirmed even if a commit/PR/edited files were found (default: unconfirmed only when nothing was found) |
| `branch` | string | no | Git branch the task is worked on. Read from the repo's HEAD on activation and then follows file edits; name it here when the work happens elsewhere (another worktree) — a named branch is never moved automatically |
| `worktree` | string | no | Only with 'branch'. Absolute path of the git worktree that branch is checked out in |
| `confidence` | string: `measured`, `inferred`, `reported`, `unverified` | no | Required. Provenance field, defined in memory_add. |
| `evidence` | string | no | As in memory_add. |
| `measured_at` | string | no | As in memory_add. |
| `claim` | string | no | As in memory_add. |
| `volatility` | string: `immutable`, `slow`, `volatile` | no | As in memory_add. |
| `verify_with` | string | no | As in memory_add. |
| `subject` | string | no | As in memory_add. |
<!-- /params -->

**Returns:** `id`, `label`, `status`, `priority`, `updated`, `activated_at`, `closed_at`, `git`
(`branch`, `worktree`), `resolution`, `provenance`, `updated_by`.

```json
{"id": "<task uuid or 8+ char prefix>", "status": "active"}
```

### task_list

List tasks by project, status and priority, sorted by priority then creation date. Compact by
default; `full=true` returns the historical per-task shape, `full_notes=true` each note whole.

<!-- params:task_list -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | no | Filter by project name |
| `status` | string | no | Filter by status (comma-separated: 'active,blocked') |
| `priority` | string | no | Filter by priority level |
| `limit` | integer | no | Max results (default: 20) Default: `20`. |
| `full` | boolean | no | Return the historical per-task shape (authors, full RFC3339 timestamps, a wider note budget, every field present even when null) instead of the compact default. Default: `false`. |
| `full_notes` | boolean | no | Return each task's note whole instead of truncated to the note_char_budget. Evidence is always a summary here (total/green/last_green) regardless of this flag — the full array is only in task_view. Default: `false`. |
<!-- /params -->

**Returns:** `tasks` (each: `id`, `label`, `status`, `priority`, `date`, clipped `note` with
`note_truncated`, `work_logs`, `ripe`, `branch`, `branch_state`, and `evidence` as
total/green/last_green only when there is any), `total`, `filters`, `note_char_budget`,
`how_to_see_full_note`.

```json
{"project": "aurelius", "status": "active"}
```

### task_log

Record work on a task as a WorkLog node, optionally with decisions and problem/solution pairs.
Never changes the task's status.

<!-- params:task_log -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `task` | string | yes | UUID or label of the task |
| `text` | string | yes | Description of work done |
| `decisions` | array of string | no | Decisions made during this work |
| `problems_solved` | array of object | no | Problem/solution pairs encountered |
| `problems_solved[].problem` | string | no |  |
| `problems_solved[].solution` | string | no |  |
| `confidence` | string: `measured`, `inferred`, `reported`, `unverified` | no | Required. Provenance field, defined in memory_add. |
| `evidence` | string | no | As in memory_add. |
| `measured_at` | string | no | As in memory_add. |
| `claim` | string | no | As in memory_add. |
| `volatility` | string: `immutable`, `slow`, `volatile` | no | As in memory_add. |
| `verify_with` | string | no | As in memory_add. |
| `subject` | string | no | As in memory_add. |
<!-- /params -->

**Returns:** `task_id`, `task_label`, `task_status`, `created_nodes` (the work log and each spawned
decision/problem/solution), `total_created`, `provenance`.

```json
{
  "task": "<task id>",
  "text": "Drafted the rates section",
  "confidence": "reported",
  "decisions": ["Document priority dictated > own exchange > bank > market"]
}
```

### task_stats

Task analytics for a project and time window.

<!-- params:task_stats -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | no | Filter by project name |
| `since_days` | integer | no | Window size in days for 'done_in_window' metric (default: all time) |
<!-- /params -->

**Returns:** `total`, `by_status`, `by_priority`, `completion_rate`, `avg_active_to_done_hours`,
`median_active_to_done_hours`, `currently_blocked`, `oldest_active_age_days`, `done_in_window`,
`closed`, `window_days`, `project`.

```json
{"project": "aurelius", "since_days": 30}
```

### task_view

The whole task — never truncated — plus its own branch: work logs as a timeline, decisions,
problems, solutions, direct subtasks. The branch is capped at 5 items per category with notes
clipped to 300 characters unless `full=true` or `limit=N`.

<!-- params:task_view -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | UUID or label of the task |
| `full` | boolean | no | Skip truncation entirely: every branch node, notes untrimmed (default: false) |
| `limit` | integer | no | Max nodes shown per category (timeline/decisions/problems/solutions/subtasks) when full is not set (default: 5) |
<!-- /params -->

**Returns:** `task` (the node), `status`, `priority`, `acceptance_criteria`, `activated_at`,
`closed_at`, `resolution`, `evidence` (every run), `ripe`, `git` (`branch`, `worktree`, `state`,
`pull_request`), `timeline`, `decisions`, `problems`, `solutions`, `subtasks`, `total_edges`,
`truncation`.

```json
{"id": "<task id>"}
```

### task_ripe

Tasks ready to close: active, with a passing evidence run newer than their last edit, each with
its basis — which run, when, which files were touched. Same computation as `au task ripe`;
declining an offer is CLI-only (`au task ripe --decline`).

<!-- params:task_ripe -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | no | Filter by project name. Omit to check every project. |
<!-- /params -->

**Returns:** `project`, `ripe` (tasks with their basis), `total`.

```json
{"project": "aurelius"}
```

### task_criterion

Mark one acceptance criterion met or unmet by its short handle; with neither flag, list the
criteria and their handles. A record of progress, not a closing condition. Same as
`au task criterion`.

<!-- params:task_criterion -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | UUID or label of the task |
| `met` | string | no | Short handle of the criterion to mark met (from the listing) |
| `unmet` | string | no | Short handle of the criterion to unmark |
<!-- /params -->

**Returns:** listing — `id`, `label`, `criteria` (`handle`, `text`, `met`, `met_at`),
`orphaned_marks`; marking — `id`, `label`, `handle`, `text`, `met`, `changed`.

```json
{"id": "<task id>", "met": "7c599784"}
```

## Reminders

The reminder model — states, owner, journal, and why no timer lives in the MCP server — is
described in [Reminders](reminders.md#model). There is no `reminder_cancel` here: cancelling is
CLI-only (`au remind cancel`).

All five tools return the reminder record: `id`, `text`, `owner`, `state`, `due_at`,
`original_due_at`, `snooze_count`, `repeat_spec`, `project`, `task_id`, `created_at`,
`delivered_at`, `delivered_count`, `delivered_via`, `done_at`, `cancelled_at`.

### reminder_add

Set a reminder — exactly one of `due_in` (a delay) or `due_at` (a moment). `owner: ai`/`both` is
delivered into a session at the end of a turn or when the owner next types (the next touch, not
the exact second); `owner: me`/`both` is the only kind the out-of-session channel acts on.

<!-- params:reminder_add -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `text` | string | yes | What to remind about |
| `due_in` | string | no | Delay from now: a bare number of minutes, or a number with suffix m/h/d/w (e.g. '2h', '30', '1w'). Exactly one of due_in/due_at is required. |
| `due_at` | string | no | An absolute moment: RFC 3339, 'YYYY-MM-DD HH:MM', 'YYYY-MM-DD' (09:00 local), or 'HH:MM' (today if still ahead, otherwise tomorrow). Exactly one of due_in/due_at is required. |
| `owner` | string: `me`, `ai`, `both` | no | Who this reminder is for, and therefore which channel may deliver it. 'ai': session-only, arrives on the next turn boundary. 'me': out-of-session channel only (the daemon). 'both': either. Default: both. Default: `"both"`. |
| `task` | string | no | UUID or label of a task to attach this reminder to, resolved the same way task tools resolve an id-or-label. The reminder inherits this task's project unless 'project' is also given. |
| `project` | string | no | Project name. Inherited from 'task' when one is given and this is omitted. |
| `repeat` | string | no | Delay to re-arm the reminder after each delivery, same grammar as due_in (e.g. '1d' for a daily reminder). Omit for a one-shot reminder. |
<!-- /params -->

**Returns:** the reminder record plus `created`.

```json
{"text": "Check the docs PR review", "due_in": "2h", "owner": "ai", "project": "aurelius"}
```

### reminder_list

List reminders, oldest due first: `pending` and `delivered` by default.

<!-- params:reminder_list -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | no | Filter by project name |
| `state` | string: `pending`, `delivered`, `done`, `cancelled` | no | Filter to exactly this state. Overrides include_terminal. |
| `include_terminal` | boolean | no | Include done/cancelled reminders (default: false — only pending/delivered) Default: `false`. |
| `limit` | integer | no | Max results (default: 50) Default: `50`. |
<!-- /params -->

**Returns:** `reminders` (records), `count`.

```json
{"project": "aurelius"}
```

### reminder_show

One reminder by id or unique prefix (4+ hex characters), with its full journal.

<!-- params:reminder_show -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | Full reminder id, or a unique prefix of at least 4 characters |
<!-- /params -->

**Returns:** `reminder` (the record), `journal` (`at`, `kind` — created, delivered, snoozed,
re-armed —, `detail`), `postponements` (`count`, `original_due_at`).

```json
{"id": "8815a36e"}
```

### reminder_done

Mark a reminder done — acted on. Refuses on one already done or cancelled.

<!-- params:reminder_done -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | Full reminder id, or a unique prefix of at least 4 characters |
<!-- /params -->

**Returns:** the record plus `done: true`.

```json
{"id": "8815a36e"}
```

### reminder_snooze

Postpone a reminder — exactly one of `in` or `at`. Recorded as a journal entry; `original_due_at`
never moves.

<!-- params:reminder_snooze -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | Full reminder id, or a unique prefix of at least 4 characters |
| `in` | string | no | Delay from now, same grammar as due_in (e.g. '2h'). Exactly one of in/at is required. |
| `at` | string | no | An absolute moment, same grammar as due_at. Exactly one of in/at is required. |
<!-- /params -->

**Returns:** the record plus `snoozed: true`.

```json
{"id": "8815a36e", "in": "1d"}
```

## Finance

Personal spending, income and currency exchanges. The entry model, currencies and the rate
priority are described in [Personal finance](finance.md). Entries are addressed by integer id
(`#N` in their lines); every entry object carries `id`, `kind`, `amount`, `amount_minor`,
`currency`, `counter_amount`, `counter_amount_minor`, `counter_currency`, `category`, `note`,
`occurred_at`, `source`, `provenance`, `rate` (the effective rate of an exchange), `usd` (USD
equivalent with `usd`, `usd_minor`, `units_per_usd`, `usd_per_unit`, `rate_date`, `source`,
`source_label`, `stale`, `text`, or `null`), `voided_at`, `void_reason`, and `line` — the
human-readable line, in Russian, to read back to the owner.

### fin_log

Record a batch of entries — one call per spoken phrase. All or nothing: one invalid entry writes
nothing and the error names `entries[i]`. The tool description carries the rules the agent
follows (reuse categories, ask about unknown currencies and missing exchange sides).

<!-- params:fin_log -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `entries` | array of object | yes | Entries to record, in the order spoken. |
| `entries[].kind` | string: `expense`, `income`, `exchange` | yes |  |
| `entries[].amount` | string | yes | Decimal STRING in currency units, e.g. "25" or "0.0015" — never a fractional JSON number. For exchange: the amount given. |
| `entries[].currency` | string | yes | Currency code, e.g. BYN, RUB, USD, USDT, TON. For exchange: the currency given. |
| `entries[].counter_amount` | string | no | Exchange only, required there: amount received, as a decimal STRING. |
| `entries[].counter_currency` | string | no | Exchange only, required there: currency received. |
| `entries[].category` | string | no | Lowercase free text; reuse one from fin_categories. Required for expense/income. |
| `entries[].note` | string | no | Anything else worth keeping: card, place, counterparty. |
| `entries[].occurred_at` | string | no | When it happened: YYYY-MM-DD, 'YYYY-MM-DD HH:MM' (local) or RFC 3339. Default: now. |
| `entries[].source` | string: `voice`, `text`, `manual`, `import` | no | Overrides the batch source for this entry. |
| `entries[].provenance` | string | no | Where the entry came from, e.g. a bot message id. |
| `source` | string: `voice`, `text`, `manual`, `import` | no | How the owner said it: voice message, typed text, manual entry. Default: text. |
<!-- /params -->

**Returns:** `entries`, `lines`, `logged`, `rate_errors`.

```json
{
  "source": "voice",
  "entries": [
    {"kind": "expense", "amount": "25", "currency": "BYN", "category": "groceries", "note": "Green, card"},
    {"kind": "exchange", "amount": "100", "currency": "USD", "counter_amount": "325", "counter_currency": "BYN"}
  ]
}
```

### fin_list

Entries for a period, newest first, each converted to USD at the rate of its date. Voided entries
are hidden unless `include_voided=true`. A network failure leaves entries unconverted, never fails
the call.

<!-- params:fin_list -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `period` | string | no | today \| yesterday \| week \| month \| year \| all \| <N>d (last N days). Default: month. |
| `since` | string | no | Start, inclusive: YYYY-MM-DD (local midnight) or RFC 3339. Overrides the period's start. |
| `until` | string | no | End, exclusive: YYYY-MM-DD (local midnight) or RFC 3339. |
| `kind` | string: `expense`, `income`, `exchange` | no |  |
| `category` | string | no |  |
| `currency` | string | no | Matches either side of an exchange. |
| `include_voided` | boolean | no |  |
| `limit` | integer | no | Default 50. |
<!-- /params -->

**Returns:** `entries`, `lines`, `count`, `period` (`label`, `since`, `until`), `rate_errors`.

```json
{"period": "month", "currency": "BYN"}
```

### fin_stats

Totals for a period. With `base=USD` (default): one USD total, a USD breakdown by `group_by`, how
many entries used each rate source, stale entries, and the unconverted bucket. `totals` stays per
original currency; exchanges are never counted as expense or income.

<!-- params:fin_stats -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `period` | string | no | today \| yesterday \| week \| month \| year \| all \| <N>d. Default: month. |
| `since` | string | no | Start, inclusive: YYYY-MM-DD or RFC 3339. |
| `until` | string | no | End, exclusive: YYYY-MM-DD or RFC 3339. |
| `group_by` | string: `category`, `month`, `currency` | no | Default: category. |
| `base` | string: `USD`, `none` | no | Summary currency. Default USD; none = per-currency only, no rates. |
<!-- /params -->

**Returns:** `text` (a ready summary), `period`, `group_by`, `totals` (per currency: `expense`,
`income`, `exchanged_out`, `exchanged_in`, `entries`, each amount also as `*_minor`), `groups`
(native breakdown per key and currency), `usd` (`expense`, `income`, `groups`, `sources`,
`converted_entries`, `unconverted` with `entries`/`ids`/`totals`, `stale_entries`,
`oldest_rate_date`, `rate_errors`; `null` for `base=none`), `voided_excluded`.

```json
{"period": "month", "group_by": "category"}
```

### fin_rate

Rates to USD, cached as exact fractions. `set` dictates the rate for one entry (beats every
source); `clear` drops it; `list` shows the cache; `refresh` forgets the bank and market rates of a
date (dictated ones stay) and fetches again.

<!-- params:fin_rate -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `action` | string: `set`, `clear`, `list`, `refresh` | no | Default: list. |
| `entry_id` | integer | no | Entry id (#N). Required for set/clear. |
| `rate` | string | no | set: the dictated rate as a decimal STRING, e.g. "3.25". |
| `rate_unit` | string: `per_usd`, `usd_per_unit` | no | set: how the rate reads. Default per_usd for fiat, usd_per_unit for crypto. |
| `currency` | string | no | set/clear: which side of an exchange (default: the entry currency). list/refresh: filter to one currency. |
| `date` | string | no | YYYY-MM-DD. Required for refresh; list: exactly this date. |
| `since` | string | no | list: from this date, YYYY-MM-DD, inclusive. |
| `until` | string | no | list: up to this date, YYYY-MM-DD, inclusive. |
| `limit` | integer | no | list: default 100. |
<!-- /params -->

**Returns:** `set`/`clear` — `entry` and `line` with the new USD equivalent, `rate_errors`
(`clear` also `removed`); `list` — `rates` (`currency`, `date`, `source`, `source_label`,
`units_per_usd`, `usd_per_unit`, `rate_num`, `rate_den`, `entry_id`, `fetched_at`), `count`;
`refresh` — `date`, `refreshed` (per currency: `source`, `rate_date`, `units_per_usd`,
`usd_per_unit`, `error`).

```json
{"action": "set", "entry_id": 1, "rate": "3.25"}
```

```json
{"action": "refresh", "date": "2026-10-14", "currency": "RUB"}
```

### fin_void

Void an entry: the row stays, marked with when and why, and drops out of lists and totals. There
is no delete; the correction is a new `fin_log`. Voiding twice is refused.

<!-- params:fin_void -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `id` | integer | yes | Entry id (the #N in fin_log/fin_list lines). |
| `reason` | string | yes | Why it is voided, e.g. 'wrong amount, re-logged as #14'. |
<!-- /params -->

**Returns:** `entry`, `line`, `voided`.

```json
{"id": 1, "reason": "wrong amount, re-logged as #4"}
```

### fin_categories

Existing categories with the number of live entries and last use, most used first. Called before
`fin_log` so the agent reuses a category instead of inventing a near-duplicate.

<!-- params:fin_categories -->
No parameters.
<!-- /params -->

**Returns:** `categories` (`category`, `count`, `last_used`).

```json
{}
```

### fin_currencies

The currency directory: `list` (default), `add` (or re-enable), `disable`. A currency is added
only after the owner confirms the code; the scale of an existing currency never changes.

<!-- params:fin_currencies -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `action` | string: `list`, `add`, `disable` | no |  |
| `code` | string | no | Currency code, 2-10 latin letters or digits. Required for add/disable. |
| `kind` | string: `fiat`, `crypto` | no | Required for add. |
| `scale` | integer | no | Decimal places, 0-8. Required for add. |
| `include_disabled` | boolean | no | list: also show disabled currencies. |
<!-- /params -->

**Returns:** `list` — `currencies` (`code`, `kind`, `scale`, `enabled`); `add`/`disable` —
`currency`, the changed record. Adding one that already exists and is enabled is an error.

```json
{"action": "add", "code": "GEL", "kind": "fiat", "scale": 2}
```

## Skills

Reusable procedural "how-to" cards with progressive disclosure: the trigger is FTS-indexed
(discoverable), the full markdown body is fetched on demand. The index is injected every session
by a SessionStart hook (`au skills --hook`).

### skill_list

Cheap index — name, trigger, tags and use count, never the body — ranked by usage. Optional
full-text `query` or `tag` filter.

<!-- params:skill_list -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `query` | string | no | Optional FTS filter over skill name/trigger. Omit to list all skills. |
| `tag` | string | no | Optional: only skills carrying this tag |
| `limit` | integer | no | Max skills to return (default: 200) Default: `200`. |
<!-- /params -->

**Returns:** `skills` (`name`, `trigger`, `tags`, `uses`), `count`, `hint`.

```json
{"query": "release"}
```

### skill_get

One card's full markdown body by name, with a fuzzy full-text fallback and disambiguation. Bumps
the card's use count.

<!-- params:skill_get -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Exact skill name (the label shown by skill_list) |
<!-- /params -->

**Returns:** `name`, `trigger`, `tags`, `body`, `uses`, `matched_by` (`exact` or the fallback used).

```json
{"name": "release-checklist"}
```

### skill_save

Create or update a card (upsert by name). The trigger goes into the indexed note; body and tags
into data.

<!-- params:skill_save -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Unique skill name, kebab-case (e.g. 'patch-shaiya-pe'). Used as the lookup key. |
| `trigger` | string | yes | When to apply this skill — one or two sentences. This is what search matches on. |
| `body` | string | yes | Full markdown instructions: steps, commands, gotchas, examples. |
| `tags` | array of string | no | Optional tags for grouping/filtering (e.g. ['shaiya', 'reverse']) |
<!-- /params -->

**Returns:** `id`, `name`, and `created: true` or `updated: true`.

```json
{
  "name": "release-checklist",
  "trigger": "Cutting an aurelius release",
  "body": "1. cargo test\n2. bump version\n3. tag",
  "tags": ["release"]
}
```

## Web Search

### search_web

Search the web through the Brave Search API (`BRAVE_API_KEY`) or the Perplexity Search API
(`PERPLEXITY_API_KEY`), picked by `provider`. Results are cached in SQLite per provider, so a
repeat query does not spend quota; `save_to_graph` also stores them as nodes.

<!-- params:search_web -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `query` | string | yes | Search query |
| `count` | integer | no | Number of results (default: 5, max: 20) Default: `5`. |
| `cache_days` | integer | no | How many days to cache results (default: 7) Default: `7`. |
| `save_to_graph` | boolean | no | Save results as a concept node in the knowledge graph (default: false) Default: `false`. |
| `provider` | string: `brave`, `perplexity` | no | Which search backend to use (default: brave). 'brave' calls the Brave Search API (src/search/brave.rs, key via BRAVE_API_KEY or ~/.config/aurelius/brave.key); 'perplexity' calls the Perplexity Search API (src/search/perplexity.rs, key via PERPLEXITY_API_KEY or ~/.config/aurelius/perplexity.key). Default: `"brave"`. |
<!-- /params -->

**Returns:** `provider`, `query`, `results` (`title`, `url`, `description`), and `cached_at` when
the answer came from the cache.

```json
{"query": "sqlite vacuum into", "count": 3}
```

### search_recall

Full-text search over cached results of past web searches, without calling the API.

<!-- params:search_recall -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `query` | string | yes | FTS query to search through cached results |
| `limit` | integer | no | Maximum results (default: 10) Default: `10`. |
<!-- /params -->

**Returns:** `matches` — per cached search: `original_query` and its `results`.

```json
{"query": "sqlite"}
```

## Documents

Word, PowerPoint, Excel, OpenDocument, RTF, EPUB, CSV, PDF, HTML and plain text into
GitHub-Flavored Markdown, converted locally — no network, no API key. Everything converted stays
searchable afterwards, even once the original file is gone. Out of scope: audio/video
transcription and OCR of scans.

### doc_convert

Convert a file, or every file in a directory, to Markdown and cache it by content hash. Output
larger than `max_inline_chars` is not inlined: the result carries an outline and a preview, and the
text is read with `doc_read`.

<!-- params:doc_convert -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `path` | string | yes | Absolute path to a file, or to a directory to convert in bulk |
| `recursive` | boolean | no | Directory mode: descend into subdirectories. Respects .gitignore either way. Default: `false`. |
| `max_files` | integer | no | Directory mode: cap on files converted (default: 200) Default: `200`. |
| `max_inline_chars` | integer | no | Markdown at or below this length is returned in full; longer output is written to a .md file and returned as outline + preview + path, readable via doc_read (default: 40000) Default: `40000`. |
| `project` | string | no | Project this document belongs to. Used when save_to_graph is set. |
| `save_to_graph` | boolean | no | Also create a 'document' node linked to the project. Metadata and an excerpt only — the full text stays in the cache. Default: `false`. |
| `force` | boolean | no | Re-convert even if this exact content was converted before Default: `false`. |
<!-- /params -->

**Returns:** for a file — `file`, `path`, `format`, `sha256`, `total_chars`, `cached`,
`truncated`, `outline`, `preview` or the full `markdown`, `read_more`, `saved_to`; for a directory
— `converted`, one such item per file.

```json
{"path": "/home/me/contracts/lease.docx", "max_inline_chars": 4000}
```

### doc_read

A slice of an already-converted document, by `sha256` or path. Offsets and limits are characters.

<!-- params:doc_read -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `ref` | string | yes | The sha256 doc_convert returned, or the source file path |
| `offset` | integer | no | Character offset to start at (default: 0) Default: `0`. |
| `limit` | integer | no | Characters to return (default: 40000) Default: `40000`. |
<!-- /params -->

**Returns:** `markdown`, `offset`, `returned_chars`, `next_offset`, `has_more`, `total_chars`,
`sha256`, `path`, `format`.

```json
{"ref": "06b6e4bd37754c093c88014b5f722baee87a83ef24436a54c2c27f057fb840de", "offset": 0, "limit": 2000}
```

### doc_recall

Full-text search across every document ever converted. No listing mode: an empty query or `*`
matches nothing.

<!-- params:doc_recall -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `query` | string | yes | FTS5 query over document text and file names |
| `limit` | integer | no | Max results (default: 10) Default: `10`. |
<!-- /params -->

**Returns:** `query`, `matches` (`file`, `path`, `sha256`, `format`, `snippet`, `total_chars`,
`converted_at`), `read_with`.

```json
{"query": "termination"}
```

## Secrets

Read-only on MCP, deliberately: Aurelius stores where a secret lives, never the value (see
[Secrets](cli.md#secrets) in the CLI docs), and recording a coordinate is a human act — `au secret add`/`rm`
stay CLI-only. Only the read side is exposed here, because it answers a question an assistant
gets asked directly ("where's the Stripe key?") and the coordinate is otherwise unreachable
through MCP: it never appears in `memory_snapshot` or any other automatic dump.

### secret_list

Name, purpose and location of every recorded secret coordinate — never the value.

<!-- params:secret_list -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | no | Filter by project name. Omit to list every project's coordinates. |
<!-- /params -->

**Returns:** `project`, `secrets` (`name`, `purpose`, `location`, `location_kind`), `total`.

```json
{"project": "aurelius"}
```

## Admin tools

**Hidden from `tools/list`** unless the server process has `AURELIUS_MCP_ADMIN=1` in its
environment; **still served when called by name** either way. Three ways to reach them:

- Call by name: a client that sends `tools/call` with the name gets an answer even though the
  tool was never listed. Claude Code only calls tools it was shown, so in practice use one of the
  next two.
- List them: register the server with the variable set, e.g.
  `claude mcp add -s user -e AURELIUS_MCP_ADMIN=1 aurelius au mcp`, and restart the session.
- Use the CLI equivalent named under each tool.

### memory_dump

Paginated export of the graph as JSON, newest first. CLI: `au export` (whole graph).

<!-- params:memory_dump -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `offset` | integer | no | Number of items to skip (default: 0) Default: `0`. |
| `limit` | integer | no | Maximum items to return (default: 50) Default: `50`. |
<!-- /params -->

**Returns:** `nodes`, `edges`, `offset`, `limit`, `total_nodes`, `total_edges`.

```json
{"offset": 0, "limit": 100}
```

### memory_merge

Merge two near-duplicate nodes: rewire every edge from `source` onto `target`, drop self-loops and
duplicate edges, append the source note, delete the source. CLI: `au merge`.

<!-- params:memory_merge -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | UUID or label of the node to merge FROM (will be deleted) |
| `target` | string | yes | UUID or label of the node to merge INTO (survives) |
<!-- /params -->

**Returns:** `source`, `target` (`id`, `label`), `edges_rewired`, `self_loops_removed`,
`duplicate_edges_removed`, `note_merged`.

```json
{"source": "<uuid that goes>", "target": "<uuid that stays>"}
```

### memory_gc

Garbage collection: duplicate edges, orphaned edges, duplicate nodes by content hash; leaves no
vector behind.

<!-- params:memory_gc -->
No parameters.
<!-- /params -->

**Returns:** `duplicate_edges_removed`, `orphan_edges_removed`, `duplicate_nodes_removed`,
`bankrupt_scanned`, `bankrupt_absorbed`.

```json
{}
```

### memory_consolidate

Rebuild a project's Digest node (snapshot layer 7) from recent sessions' next steps and unsolved
problems. Idempotent: one digest per project, overwritten in place.

<!-- params:memory_consolidate -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `project` | string | yes | Project name |
<!-- /params -->

**Returns:** `id`, `label`, `note` (the digest text).

```json
{"project": "aurelius"}
```

### memory_journal

What a given run wrote — the selection a session-end hook needs to tell its own records from
yesterday's. CLI: `au journal`.

<!-- params:memory_journal -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `session_id` | string | no | The run to list. Falls back to AURELIUS_SESSION_ID when omitted. |
| `limit` | integer | no | Cap on entries returned (default: 50) |
<!-- /params -->

**Returns:** `session_id`, `entries` (`id`, `label`, `type`, `created_at`), `count`.

```json
{"session_id": "doc-run-2", "limit": 5}
```

### memory_eval

Score search quality against a frozen fixture. Reads only. A run with `live: true` is marked
incomparable. CLI: `au eval`. The default fixture lives outside the repository; without it the
call fails with "unable to open database file".

<!-- params:memory_eval -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `live` | boolean | no | Run against the live database instead of the frozen fixture. The report comes back marked incomparable. |
| `now` | string | no | RFC3339 instant node ages are measured from. Defaults to meta.as_of of the case file — the clock is a parameter, never the system time. |
| `cases` | string | no | Path to a case file, when it is not the default fixtures/eval/cases.jsonl |
| `db` | string | no | Path to the fixture database to score against, overriding meta.fixture in the case file (ignored under live: true, which always scores the live database). |
<!-- /params -->

**Returns:** `total`, `passed`, `failed`, `skipped`, `mrr`, `by_kind` (per case kind: `cases`,
`eligible`, `passed`, `failed`, `skipped`), `comparable`, `fixture`, `fixture_sha256`, `cases`,
`as_of`, `now_source`, `digest`, `vector_notice`.

```json
{"live": true}
```

### skill_remove

Delete a skill card by name.

<!-- params:skill_remove -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Exact skill name to delete |
<!-- /params -->

**Returns:** `name`, `deleted`.

```json
{"name": "release-checklist"}
```

### db_check

Integrity check. Read-only — never migrates, never writes. CLI: `au db check`.

<!-- params:db_check -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `full` | boolean | no | Full integrity_check instead of the fast quick_check. Slower, and the only one that reads every page. |
<!-- /params -->

**Returns:** `ok`, `problems`, `mode` (`quick_check` or the full check), `path`, `nodes`, `edges`,
`file_bytes`, `wal_bytes`, `page_count`, `page_size`.

```json
{"full": true}
```

### db_backup

Safe snapshot via SQLite `VACUUM INTO`. Writes a new file. CLI: `au db backup`.

<!-- params:db_backup -->
| Name | Type | Required | Meaning |
|---|---|---|---|
| `out` | string | no | Destination path. Defaults to a timestamped file beside the database. |
<!-- /params -->

**Returns:** `source`, `dest`, `bytes`.

```json
{"out": "/tmp/aurelius-before-upgrade.db"}
```

### db_reindex_embeddings

Queue nodes whose vector is missing; the daemon embeds them on its ticks. Returns in milliseconds
and computes nothing itself. CLI: `au db reindex-embeddings`.

<!-- params:db_reindex_embeddings -->
No parameters.
<!-- /params -->

**Returns:** `queued`, `note`.

```json
{}
```

## Regenerating the tables

The parameter tables are generated from the `json!` literal in `tools.rs` — no build and no
database needed:

```bash
python3 scripts/dump-mcp-tools.py                      # counts: total, admin, listed
python3 scripts/dump-mcp-tools.py --update docs/mcp-tools.md
python3 scripts/dump-mcp-tools.py --check docs/mcp-tools.md   # exit 1 when a table is stale
python3 scripts/dump-mcp-tools.py --json               # every schema, with an admin flag
```

`--update` and `--check` fail when a tool has no `<!-- params:NAME -->` block here or a block names
a tool that no longer exists. Purpose, returns and examples are written by hand.
