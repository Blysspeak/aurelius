# CLI

[Back to README](../README.md)

The `au` binary: one command line for the graph, tasks, reminders, finance, sync and database
care. Every subcommand and flag of `au` 3.8.0 is listed in the [Command reference](#command-reference);
its usage lines and flag tables are generated from `au --help` by `scripts/dump-au-help.py`.

## Contents

- [Command overview](#command-overview)
- [Exit codes](#exit-codes)
- [Provenance](#provenance)
- [Command groups](#command-groups)
- [Secrets](#secrets)
- [Backups](#backups)
- [Maintenance](#maintenance)
- [Command reference](#command-reference)
  - [Setup and server](#setup-and-server)
  - [Writing memory](#writing-memory)
  - [Reading memory](#reading-memory)
  - [Project indexing](#project-indexing)
  - [Tasks](#tasks)
  - [Reminders](#reminders)
  - [Finance](#finance)
  - [Secret coordinates](#secret-coordinates)
  - [Identity and sync](#identity-and-sync)
  - [Database](#database)
  - [Documents](#documents)
  - [Hook plumbing](#hook-plumbing)
  - [Evaluation](#evaluation)
  - [Retired commands](#retired-commands)
- [Regenerating the reference](#regenerating-the-reference)

## Command overview

```bash
au init                            # initialize database (indexes nothing)
au note "chose X over Y" -p app   # capture a decision → project
au note --stdin --kind episodic \
  --key precompact:$SESSION --json  # a moment, not a lasting fact: ages out, upserts, reports its id
au session "what happened" -p app \
  -d "decision" -n "next step"     # the layer-4 record: what `memory_session` writes, without MCP
au session --stdin -p app --json    # same record from a hook: {"summary":…,"decisions":[…],"next_steps":[…]}
au relate <from> <to> --type solves # edge between two nodes (`part-of`, `refines`, … — hyphens fine)
au note "…" --session $SESSION_ID   # stamp the run that wrote it (or export AURELIUS_SESSION_ID)
au note "flag is on" --confidence measured \
  --evidence "cat app/.env" \
  --claim "REFUND_REQUESTS_ENABLED=true" \
  --volatility volatile --verify-with "cat app/.env" \
  --subject myapp:.env:REFUND_REQUESTS_ENABLED  # where it came from, how fast it rots, what it is about
au journal --session $SESSION_ID    # everything that run wrote — the selection a session-end hook needs
au context beacon                  # graph around a topic
au search "redis"                  # hybrid search, 5 hits by default (--limit N)
au search "redis" --json --type decision,problem   # one JSON object; `origin` says which half found each hit
au reindex [--path DIR]            # index a project explicitly — the only way a repository enters the graph
au view                            # open web graph UI
au touch path/to/file              # bump access_count of an already indexed file node (creates none)
au export                          # export full graph as JSON
au mcp                             # start MCP server
au skills                          # print the skill index
au snapshot -p myapp [--json]      # layered slice: Markdown, or a fixed JSON shape
au pickup -p myapp [--json]        # bounded ranked payload to rebuild working state after a context wipe
au recall <uuid-or-subject>        # read one record back by its exact subject, not by search (MCP: memory_search id=…)
au recall --prefix myapp:refunds    # every live node whose subject starts with this facet
au path <from> <to> [--json]       # step ladder over next_step/prerequisite edges; --before <id> for everything leading in
au graph import graph.json         # bulk-import an external documentation graph (spec 008)
au graph export --format mermaid   # the same graph back out
au merge <source> <target>         # fold a near-duplicate node into the one that stays
au home current                    # which data/config directory au and aurelius read (`use`, `reset`)
au eval [cases.jsonl] [--json]     # score recall against a frozen fixture — the argument gets a number
au eval-search [--json]            # search quality by query class for FTS, dense, RRF, rerank (see Search)
au trace -m "what just happened"   # append to the action journal (or --hook on PostToolUse)
au judge                           # settle the session: reinforce, erode, fork or null
au daemon                          # reminder clock + embedding model owner (systemd user unit)
au fin log --kind expense --amount 25 --currency BYN --category groceries   # one finance entry
au fin stats --period month        # totals in USD at each entry's date, native currencies beside
au db check [PATH]                 # verify integrity (read-only); PATH verifies a snapshot
au db backup                       # safe snapshot via VACUUM INTO
au db reindex-embeddings           # queue nodes without a vector for the daemon
au db reattach [--apply]           # link records to the project hub their data.project names
au doc convert report.docx         # document → Markdown on stdout
au doc convert ./contracts -r      # convert a whole tree, cached by content hash
au doc recall "termination"        # search everything ever converted
# au task …                       # every task command: docs/tasks.md#task-commands
```

> **Removed:** `au sync` (the TimeForged connector — spec 007 found zero calls to it across
> hooks and 19 repositories) and `au capture` (its calling hook was never wired into any
> project) were pulled after a usage audit found no consumer for either. Both subcommands
> still parse — calling one prints an explanation and what replaced it, and exits `1`, rather
> than failing as an unknown-argument error. If a script of yours still calls `au sync` or
> `au capture`, that script is dead weight: nothing downstream was reading their output either.

## Exit codes

A caller has to tell "I called it wrong" from "the store is unreachable": the first is
fixed by calling differently, the second by hand — retrying it is pointless.

| Code | Meaning |
|------|---------|
| `0` | done (`--help` and `--version` included) |
| `1` | bad call — unknown `--type`, missing argument, node not found, malformed JSON on stdin |
| `2` | storage unreachable — no database, damaged image, locked SQLite |
| `10` | `au task claim`/`renew`/`release`/`give-up`: the pool is empty, or the lease expired and was reissued to someone else. A shift counts consecutive `10`s to know the queue is exhausted |
| `11` | the same commands: the database is held by another writer. Distinct from `10` on purpose — otherwise the owner's own hand-run session reads as an empty queue and the shift exits reporting success |
| `12` | `au task evidence`: nothing to attach the run to — the project has no active task. Nothing is written to the graph: the run stays in the caller's own journal (and in the session's `verify` trace, when a session resolves) |
| `14` | `au eval`: no run, no numbers — the fixture's sha256 did not match `meta.fixture_sha256`, `meta.version` is unknown, or the run needed to write. A failing *case* is not this code: an incomparable number is worse than a missing one |
| `15` | `au daemon`: another daemon is alive and holds the lock. Distinct so a supervisor (systemd `Restart=on-failure`) does not treat a correctly refused second start as a crash |

Codes above `2` are neither: the call was right and the database is intact. Each names a state
the caller has to branch on, and each exists because collapsing it into `1` made some script
report success on a condition retrying cannot fix.

The `--hook` variants (`au snapshot --hook`, `au trace --hook`, `au judge --hook`) are the
deliberate exception: they never fail and stay silent on error. A broken hook is worse than
a missing snapshot.

## Provenance

A fact about the world is worth exactly as much as the answer to "how do you know?".
Without that, a guess lands in the graph looking identical to a measurement — and reads
back as truth six weeks later.

| Field | What it is for |
|-------|----------------|
| `confidence` | `measured` \| `inferred` \| `reported` \| `unverified`. **Required** on `memory_add`. Absent reads as `unverified`, never as "probably measured". `measured` without `evidence` is refused — that is `inferred`. |
| `evidence` | The command or query **verbatim** that produced the fact |
| `measured_at` | When it was measured; defaults to now for `measured` |
| `claim` | The assertion in one or two lines (≤240 chars). A longer one is shortened at write time — its first sentence if that fits, else cut at a word — and the original is kept in `claim_full`; the write response says so. Returned **whole** — recall clips the long note, never the claim |
| `volatility` | `immutable` \| `slow` (30 d) \| `volatile` (1 d). Past that age the fact comes back marked "старше N дн — перепроверь" |
| `verify_with` | The command that re-checks it |
| `subject` | What is being asserted, e.g. `myapp:.env:REFUND_REQUESTS_ENABLED`. A second fact about the same subject is **refused** until `resolution` says `supersede` \| `refine` \| `coexist` — and the resolution becomes an edge, not a memory of one |

A failed probe (`probe_warnings`) does **not** downgrade `confidence`. Probes extract
path-like tokens from prose and check them on disk — weak evidence by construction: an
aliased import, a path on another machine, a file in someone else's repo all fail it
while saying nothing about the fact. `evidence` with a command and an exit code is
strong evidence, and the weak one has no business overruling the strong one. A failed
probe stays a remark, not a verdict.

Both doors take the same fields and share one parser: `au note --confidence …` and
`memory_add(confidence=…)` cannot drift apart on what a measured fact is.

## Command groups

Larger command families also have a narrative page next to the feature they drive:

| Group | Narrative | Flags |
|-------|-----------|-------|
| `au task …` | [Task Commands](tasks.md#task-commands) | [Tasks](#tasks) |
| `au remind …`, `au daemon` | [Reminder Commands](reminders.md#reminder-commands) | [Reminders](#reminders) |
| `au fin …` | [Personal finance](finance.md#cli) | [Finance](#finance) |
| `au identity …`, `au share …` | [Sync Commands](sync.md#sync-commands) | [Identity and sync](#identity-and-sync) |

## Secrets

Aurelius stores where a secret lives, never the value: `--where` names an environment variable,
a file path or a password-manager entry, not the secret itself.

```bash
au secret add --name STRIPE_SECRET_KEY --where "env:STRIPE_SECRET_KEY" \
  --purpose "charge webhooks" --project myapp
au secret list --project myapp     # coordinates only — values were never stored
au secret rm STRIPE_SECRET_KEY --project myapp
```

Coordinates are returned on request only; they never appear in the memory snapshot or any
other automatic dump.

## Backups

```bash
au db check          # quick integrity report; exits non-zero when damaged
au db check --full   # exhaustive check, every table
au db check FILE     # verify a snapshot — a snapshot is an ordinary database
au db backup         # snapshot → backups/aurelius-<UTC timestamp>.db next to the database
au db backup -o FILE # snapshot to a path of your choice (refused if FILE exists)
```

### Automatic snapshots

Snapshots are taken automatically: the plugin's SessionStart hook (`au db backup --hook`) keeps
a rolling set in `backups/` next to the database. Cadence follows activity rather than the
clock — the graph only changes when Claude Code or the CLI touch it, so an
idle machine does not accumulate identical copies. Several sessions in a day cost one
snapshot; ~50 ms for an 8 MB graph. Each snapshot is verified with `au db check` right
after it is written — one that fails is renamed to `.FAILED-CHECK` so it can never be
mistaken for a good backup, and kept, because a bad snapshot is evidence worth reading.

| Variable | Default | Meaning |
|---|---|---|
| `AURELIUS_BACKUP_KEEP` | `7` | snapshots retained |
| `AURELIUS_BACKUP_MIN_HOURS` | `24` | minimum age of the newest snapshot before a new one is taken |

### Copying a live database

> **Never copy, move or restore `aurelius.db` with `cp`/`mv`/`rsync`, a file manager or a
> backup agent while `au` or an MCP server is running.** In WAL mode, cross-process cache
> coherency runs through the `-shm` WAL-index rather than the database header, so replacing
> the file underneath open connections lets a live process keep flushing its cached pages
> into the new file — producing a database whose header describes 181 pages while its body
> holds 1781. `au db backup` uses SQLite's own `VACUUM INTO` and is the only safe way to
> copy a live database.

### Restoring a snapshot

To restore a snapshot: stop everything that touches the database (every `au mcp`, `au view`,
any editor with hooks), move the current database and its `-wal`/`-shm` aside, copy the
snapshot into place, run `au db check`, then restart. The "stop everything" step is why this
is a documented procedure and not a command — `au` cannot stop processes it did not start.

## Maintenance

Every command here is a dry run that only counts or lists, until `--apply` is given.

```bash
au db prune [--json]                  # junk nodes by rule; nothing with a claim goes, run nodes aside
au db purge-orphans                   # vectors whose node is deleted or missing
au db scrub-trace                     # stored trace payloads that secret masking would change
au db trace-retention --days 90       # traces older than N days (at least 30)
au db reattach                        # records whose data.project names a hub they are not linked to
au db prune-backups --keep 7          # snapshots beyond the N newest, next to the database and in backups/
```

`au db prune` has four rules: `technical_orphan` (a run or dependency node without edges, claim
or body), `technical_junk` (any run node, and any dependency node without claim or body, even
with edges — task evidence lives in the task, dependencies in the manifest), `stale_digest` (a
project distillate that is not the newest) and `empty_project` (a project node with no edges,
content or records naming it). Removal is a soft delete in one transaction. Nothing writes run or dependency nodes any more; the
rules clear what older versions left behind.

`au db reattach` repairs records written before v3.8.0, when a record could name its project in
`data.project` without being linked to the project hub — such a record was invisible to
`memory_status(project=…)` and the snapshot. Since v3.8.0 the link is made at write time, and a
missing hub is created then. `reattach` adds the missing `belongs_to` edges in one transaction,
and only counts names with no hub: inventing a project hub needs the owner's word.

New trace payloads are masked for secrets before they are written; `scrub-trace` rewrites the ones
stored before that.

## Command reference

Each entry: what the command does, its usage line, and its arguments and options. `-h, --help`
is accepted everywhere and left out of the tables.

### Setup and server

### au init

Create the database and its schema in the active home. Indexes nothing.

<!-- au:init -->
```
au init
```

No arguments or options besides `--help`.
<!-- /au -->

### au mcp

Start the MCP server on stdio — the command Claude Code runs (`claude mcp add -s user aurelius au mcp`).
See [MCP Tools](mcp-tools.md).

<!-- au:mcp -->
```
au mcp
```

No arguments or options besides `--help`.
<!-- /au -->

### au view

Serve the interactive graph UI and open it in the browser. See [Web UI](architecture.md#web-ui).

<!-- au:view -->
```
au view [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-P, --port <PORT>` | `7175` | Port to serve on |
| `--no-open` |  | Don't open browser automatically |
<!-- /au -->

### au home

Switch or inspect which data/config directory `au` and `aurelius` use. `AURELIUS_HOME` in the
environment overrides the persisted choice for one invocation.

<!-- au:home -->
```
au home <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au home use

Persist a directory as the active home for every later `au`/`aurelius` call from any shell
(created if missing) — for example a second profile that simulates a collaborator.

<!-- au:home use -->
```
au home use <PATH>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<PATH>` |  | Directory to use (created if it doesn't exist) |
<!-- /au -->

### au home current

Print the active home.

<!-- au:home current -->
```
au home current
```

No arguments or options besides `--help`.
<!-- /au -->

### au home reset

Drop the persisted choice and return to the OS default data/config directories.

<!-- au:home reset -->
```
au home reset
```

No arguments or options besides `--help`.
<!-- /au -->

### Writing memory

### au note

Add one knowledge node, with the [provenance](#provenance) fields `memory_add` takes.

<!-- au:note -->
```
au note [OPTIONS] [TEXT]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[TEXT]` |  | The note content (decision, observation, etc.). Omit it when using --stdin, or when --claim already carries the whole assertion |
| `-t, --type <TYPE>` |  | Node type — the same set the MCP tools accept (default: decision). With --key, only an explicit --type may change the type of an existing node; the default never does |
| `-l, --label <LABEL>` |  | Label (short name). Defaults to first 60 chars of text |
| `-p, --project <PROJECT>` |  | Link to a project node (find or create by name) |
| `--kind <KIND>` | `semantic` | Memory layer: `semantic` for a lasting fact, `episodic` for something tied to a moment (a pre-compaction snapshot, a session note) that is meant to age out instead of settling into the graph forever |
| `--key <KEY>` |  | Idempotency key: repeating the call with the same key rewrites that node instead of creating a twin. For hooks that can fire twice for one occasion (PreCompact runs on both auto and manual /compact) |
| `--stdin` |  | Read the note text from stdin — the way past the ~32K limit a Windows command line puts on a single argument |
| `--json` |  | Print one JSON line with the node id instead of a human-readable line, so a caller can record which node it wrote, not merely that it wrote |
| `--session <SESSION>` |  | Stamp the run that wrote this. Without it a session-end hook sees every note of the project and cannot tell its own from yesterday's. Falls back to `AURELIUS_SESSION_ID` |
| `--claim <CLAIM>` |  | The assertion in one or two lines — returned whole, never clipped mid-word. Max 240 chars; long reasoning stays in the note text |
| `--evidence <EVIDENCE>` |  | The command or query VERBATIM that produced this |
| `--confidence <CONFIDENCE>` |  | Where this came from: measured \| inferred \| reported \| unverified. Absent reads as `unverified` — never as "probably measured" |
| `--volatility <VOLATILITY>` |  | How fast this stops being true: immutable \| slow \| volatile |
| `--verify-with <VERIFY_WITH>` |  | Command that re-checks this claim once it goes stale |
| `--subject <SUBJECT>` |  | What is being asserted, e.g. `xhub:.env:REFUND_REQUESTS_ENABLED`. A second fact about the same subject is refused until resolved |
| `--resolution <RESOLUTION>` |  | How this relates to an existing fact about the same subject: supersede \| refine \| coexist |
<!-- /au -->

### au session

Record what happened this session — the same record `memory_session` writes, and the one layer 4
of the snapshot reads.

<!-- au:session -->
```
au session [OPTIONS] [SUMMARY]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[SUMMARY]` |  | What happened this session. Omit it when using --stdin |
| `-p, --project <PROJECT>` |  | Project the session belongs to. Defaults to the current folder name, the same rule `au snapshot` follows |
| `-d, --decision <DECISIONS>` |  | A decision made this session (repeatable) — becomes its own node, linked to the session |
| `-n, --next <NEXT_STEPS>` |  | A tail left for the next session (repeatable) — this is what the digest (layer 8) is built from |
| `--stdin` |  | Read the whole record as JSON from stdin — the same shape `memory_session` takes: {"summary":…, "decisions":[…], "next_steps":[…], "problems_solved":[{"problem":…,"solution":…}], "key_files":[…]}. An unknown key is an error, not a shrug |
| `--json` |  | Print one JSON line with the session id instead of a human-readable line |
| `--session <SESSION>` |  | Stamp the run that wrote this record — the session node and everything it spawns. Falls back to `AURELIUS_SESSION_ID` |
| `--key <KEY>` |  | Idempotency key: the OCCASION of the record, not its text. Without it a repeat is recognised only when `summary` matches the previous one word for word — and a hook that reports running totals rewrites that line every time, so the twin was never caught. With a key the repeat rewrites its own node, exactly like `au note --key` |
| `--claim <CLAIM>` |  | The assertion in one or two lines — returned whole, never clipped mid-word. Max 240 chars; long reasoning stays in the note text |
| `--evidence <EVIDENCE>` |  | The command or query VERBATIM that produced this |
| `--confidence <CONFIDENCE>` |  | Where this came from: measured \| inferred \| reported \| unverified. Absent reads as `unverified` — never as "probably measured" |
| `--volatility <VOLATILITY>` |  | How fast this stops being true: immutable \| slow \| volatile |
| `--verify-with <VERIFY_WITH>` |  | Command that re-checks this claim once it goes stale |
| `--subject <SUBJECT>` |  | What is being asserted, e.g. `xhub:.env:REFUND_REQUESTS_ENABLED`. A second fact about the same subject is refused until resolved |
<!-- /au -->

### au relate

Link two nodes with a typed edge — what `memory_relate` does over MCP.

<!-- au:relate -->
```
au relate [OPTIONS] --type <RELATION> <FROM> <TO>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<FROM>` |  | Source node: UUID (what `au note --json` returns), exact label, or a search phrase |
| `<TO>` |  | Target node — same forms |
| `-t, --type <RELATION>` |  | Relation: solves, refines, part-of (a spelling of belongs_to), depends_on, … — the same vocabulary `memory_relate` accepts, with hyphens allowed |
| `-w, --weight <WEIGHT>` | `1.0` | Edge weight |
| `--json` |  | Print one JSON line instead of a human-readable line |
<!-- /au -->

### au merge

Fold a near-duplicate node into another: edges are rewired from source to target, then the source
is deleted (MCP: `memory_merge`).

<!-- au:merge -->
```
au merge <SOURCE> <TARGET>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<SOURCE>` |  | Source node (UUID or label) — will be deleted |
| `<TARGET>` |  | Target node (UUID or label) — survives with merged edges |
<!-- /au -->

### au graph

Bulk import and mermaid export of an external documentation graph (spec 008).

<!-- au:graph -->
```
au graph <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au graph import

Import a `graph.json` (nodes and edges) in one transaction: everything lands or nothing does.
Re-running the same file changes nothing; a changed body updates only that node.

<!-- au:graph import -->
```
au graph import [OPTIONS] <FILE>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<FILE>` |  | Path to the graph.json file (see specs/008-doc-graph/spec.md, "Key Entities", for the shape) |
| `--json` |  | Print the report as one JSON line instead of human-readable text |
<!-- /au -->

### au graph export

Print the graph as JSON (the same dump as `au export`) or a (sub)graph as mermaid.

<!-- au:graph export -->
```
au graph export [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--format <FORMAT>` | `json` | json \| mermaid |
| `--source-id <SOURCE_ID>` |  | Mermaid only: restrict to one import's nodes (`data.source_id`) and the edges between them, instead of the whole graph |
<!-- /au -->

### au export

Print the full graph as JSON.

<!-- au:export -->
```
au export
```

No arguments or options besides `--help`.
<!-- /au -->

### au journal

List what one run wrote, by session id (MCP: `memory_journal`).

<!-- au:journal -->
```
au journal [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--session <SESSION>` |  | The run to list. Falls back to `AURELIUS_SESSION_ID` |
| `-l, --limit <LIMIT>` | `50` | Cap on entries returned |
| `--json` |  | One JSON object with the whole list, instead of readable lines |
<!-- /au -->

### Reading memory

### au search

Hybrid search (full-text plus vectors, see [Search](memory.md#search)); five hits by default.

<!-- au:search -->
```
au search [OPTIONS] <QUERY>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<QUERY>` |  | Search query |
| `--limit <LIMIT>` | `5` | How many results to print. The default of 5 is where the retrieval pipeline ends (decision of 2026-09-13); a larger value is honoured as asked, never capped |
| `--json` |  | Print one JSON object (same hybrid path; `vector_notice` when the vector half was skipped) |
| `--type <TYPES>` |  | Keep only these node types (repeatable or comma list); a typo is refused, not answered with an empty list |
<!-- /au -->

### au recall

Read one record back by exact key — a node UUID or the exact `subject` — or list a subject facet
with `--prefix`. Nothing fuzzy: a mistyped key is a miss, not a neighbour.

<!-- au:recall -->
```
au recall [OPTIONS] [QUERY]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[QUERY]` |  | Node UUID, or the exact `--subject` a fact was written with |
| `--prefix <PREFIX>` |  | List every live node whose `subject` starts with this facet, one entry per distinct subject, newest family member first |
| `--json` |  | Print the record as one JSON object instead of human-readable lines |
<!-- /au -->

### au context

Show the graph around a topic: full-text seeds, then a breadth-first walk.

<!-- au:context -->
```
au context [OPTIONS] <TOPIC>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<TOPIC>` |  | Topic to expand the graph around |
| `-d, --depth <DEPTH>` | `2` | Graph traversal depth |
| `-v, --verbose` |  | Show sync conflict details (data._sync_conflict) when present |
<!-- /au -->

### au path

Directed step ladder over `next_step`/`prerequisite` edges: the shortest path between two nodes,
or every ancestor of one node (`--before`). Exactly one of the two forms.

<!-- au:path -->
```
au path [OPTIONS] [FROM] [TO]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[FROM]` |  | Start node: UUID, exact `subject`, or exact `label`. Required unless `--before` is given |
| `[TO]` |  | End node — same forms as `from`. Required unless `--before` is given |
| `--before <BEFORE>` |  | List every node that transitively leads to X, in "earliest first" order, instead of a path between two named nodes |
| `--max-depth <MAX_DEPTH>` | `50` | Walk depth cap — guards against a runaway search on a malformed graph |
| `--json` |  | Print machine-readable JSON instead of the human-readable ladder |
<!-- /au -->

### au snapshot

The seven-layer frozen memory snapshot, as Markdown or JSON. See [Memory snapshot](memory.md#memory-snapshot).

<!-- au:snapshot -->
```
au snapshot [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-p, --project <PROJECT>` |  | Project scope; with --hook defaults to the current folder name |
| `--hook` |  | Emit Claude Code SessionStart hook JSON (never fails, silent on error) |
| `--json` |  | Emit machine-readable facts instead of Markdown: {"project":…,"facts":[{"kind","text","at"}]}. An empty `facts` with exit 0 means "nothing to say"; no output or a non-zero exit means "broken". Parsing the Markdown means parsing the layout |
<!-- /au -->

### au pickup

Bounded, ranked payload to rebuild working state after a context wipe (MCP: `memory_pickup`).

<!-- au:pickup -->
```
au pickup [OPTIONS] --project <PROJECT>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-p, --project <PROJECT>` |  | Project to rebuild — required: an anchor facet makes no sense without a scope to compute it in |
| `--json` |  | Print JSON instead of the human-readable line form |
<!-- /au -->

### au skills

Print the skill index (name and trigger); the SessionStart hook runs it with `--hook`.

<!-- au:skills -->
```
au skills [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--hook` |  | Emit Claude Code SessionStart hook JSON instead of plain text |
<!-- /au -->

### Project indexing

### au reindex

Index a project directory — crates and files from `Cargo.toml`, key files otherwise. The only
way a repository enters the graph.

<!-- au:reindex -->
```
au reindex [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-p, --path <PATH>` |  | Project root path (defaults to git root or cwd) |
| `--hook` |  | Stop-hook mode: root comes from the hook payload's `cwd` (raised to the git toplevel), then every sync-enabled project is pushed; never fails |
<!-- /au -->

### au touch

Bump `access_count` of an already indexed file node; creates nothing.

<!-- au:touch -->
```
au touch [OPTIONS] [PATH]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[PATH]` |  | Path to the file. Required unless --hook |
| `--hook` |  | PostToolUse hook mode: read the file path from Claude Code hook JSON on stdin (`tool_input.file_path`, else `tool_input.path`); never fails, creates no new nodes |
<!-- /au -->

### Tasks

The lifecycle behind these commands is described in [Task Management](tasks.md).

### au task

Task management: create, track and log work on tasks.

<!-- au:task -->
```
au task <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au task new

Create a task, optionally with acceptance criteria and a due moment (which also creates an
attached reminder).

<!-- au:task new -->
```
au task new [OPTIONS] <TITLE>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<TITLE>` |  | Task title |
| `-p, --project <PROJECT>` |  | Project name |
| `--priority <PRIORITY>` | `medium` | Priority: critical, high, medium, low |
| `-c, --criteria <CRITERIA>` |  | Acceptance criteria (can be specified multiple times) |
| `-d, --description <DESCRIPTION>` |  | Description |
| `--due <DUE>` |  | When the task is expected to be finished — a different question from whether it is ripe for closing (`tasks::is_ripe` looks only at edits vs. evidence, never at the clock). Accepts everything `reminders::parse_moment` does: RFC 3339, `YYYY-MM-DD[ HH:MM]`, or a bare `HH:MM` for the next occurrence of that time. Also creates a reminder attached to this task, owned by `both`, so the deadline surfaces in `au remind` without a second command to remember it in |
| `--remind-before <REMIND_BEFORE>` |  | Place the attached reminder this much before `--due` instead of exactly on it — same delay grammar as `au remind add --in` |
<!-- /au -->

### au task list

List tasks, sorted by priority.

<!-- au:task list -->
```
au task list [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-p, --project <PROJECT>` |  | Filter by project |
| `-s, --status <STATUS>` |  | Filter by status (comma-separated) |
| `--priority <PRIORITY>` |  | Filter by priority |
| `--limit <LIMIT>` | `30` | Max tasks returned |
| `--json` |  | Print JSON instead of human-readable text |
<!-- /au -->

### au task update

Edit priority, title, description, due moment, or append acceptance criteria. A call with no
mutating flag is an error.

<!-- au:task update -->
```
au task update [OPTIONS] <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Task UUID or label |
| `--priority <PRIORITY>` |  | New priority: critical, high, medium, low |
| `--title <TITLE>` |  | New title — the `[project]` prefix of the label is re-derived, so project attribution is preserved |
| `-d, --description <DESCRIPTION>` |  | New description, replacing the current one |
| `-c, --criteria <CRITERIA>` |  | Acceptance criterion to append (can be specified multiple times) |
| `--due <DUE>` |  | New due moment — same grammar as `au task new --due`. Moving it moves the task's existing attached reminder through `reminders::snooze` rather than deleting and recreating it, so the journal records the move (a task without one yet gets one created, same as `au task new --due` would) |
| `--remind-before <REMIND_BEFORE>` |  | Place the moved reminder this much before the new `--due` instead of exactly on it |
| `--json` |  | Print one line of JSON instead of human-readable text |
<!-- /au -->

### au task show

Full task details with the work-log branch, timestamps, resolution and evidence. `au task view`
is an alias.

<!-- au:task show -->
```
au task show <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Task UUID or label |
<!-- /au -->

### au task log

Record work done on a task. Never changes its status.

<!-- au:task log -->
```
au task log <ID> <TEXT>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Task UUID or label |
| `<TEXT>` |  | Description of work done |
<!-- /au -->

### au task done

Close a task. The resolution is assembled from the commit and the linked edits; the flags refine it.

<!-- au:task done -->
```
au task done [OPTIONS] <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Task UUID or label |
| `--commit <COMMIT>` |  | Commit that resolved the task. When absent, the system tries to determine it on its own (`git rev-parse --short HEAD`) |
| `--pr <PULL_REQUEST>` |  | Link to the pull request |
| `--unconfirmed` |  | Explicitly mark "closed without confirmation", even if part of the resolution was determined on its own |
<!-- /au -->

### au task block

Block a task with a reason.

<!-- au:task block -->
```
au task block <ID> <REASON>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Task UUID or label |
| `<REASON>` |  | Reason for blocking |
<!-- /au -->

### au task activate

Take a task into work and bind it to a git branch. Other active tasks of the project stay active.

<!-- au:task activate -->
```
au task activate [OPTIONS] <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Task UUID or label |
| `--branch <BRANCH>` |  | Git branch the task is worked on. Without it the branch is read from the repository of the current directory and then follows file edits; a branch named here is never moved automatically |
| `--worktree <WORKTREE>` |  | Path of the git worktree that `--branch` is checked out in |
<!-- /au -->

### au task evidence

Attach a run (command and exit code) to a task, or to the active task of `--project`. Called by
the ulika hook, not by a person.

<!-- au:task evidence -->
```
au task evidence [OPTIONS] --command <COMMAND> --exit <EXIT> [ID]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[ID]` |  | Task UUID or label. May be omitted when `--project` is given |
| `--project <PROJECT>` |  | Project whose active task receives the evidence — an alternative to `id` |
| `--command <COMMAND>` |  | The command that was run |
| `--exit <EXIT>` |  | Exit code of the run |
| `--artifact <ARTIFACT>` |  | Path to the artifact of the run, if there is one |
| `--subject <SUBJECT>` |  | What this run asserts, e.g. `aurelius:verify:cargo`. Only this field is asked of the caller: `confidence` and `evidence` are derived here — a run that happened is measured, and the command is its own evidence |
| `--json` |  | Print a single JSON line instead of human-readable text |
<!-- /au -->

### au task ripe

Tasks ripe for closing, each with its grounds; `--decline` hides one until its next edit.

<!-- au:task ripe -->
```
au task ripe [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-p, --project <PROJECT>` |  | Filter by project |
| `--json` |  | Print JSON instead of human-readable text |
| `--decline <DECLINE>` |  | Decline the offer to close this task — do not present it again until a new edit appears on it (FR-015) |
<!-- /au -->

### au task stats

Task analytics: counts, completion rate, durations, blocked, oldest active.

<!-- au:task stats -->
```
au task stats [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-p, --project <PROJECT>` |  | Filter by project |
| `--since-days <SINCE_DAYS>` |  | Window in days for "done_in_window" metric |
<!-- /au -->

### au task criterion

Mark one acceptance criterion met or unmet by its handle; with neither flag, list the criteria
and their handles.

<!-- au:task criterion -->
```
au task criterion [OPTIONS] <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Task UUID or label |
| `--met <CRITERION>` |  | Criterion to mark met: a handle prefix, the exact criterion text, or `#N` |
| `--unmet <CRITERION>` |  | Criterion to unmark, addressed the same way as `--met` |
<!-- /au -->

### au task claim

Work order: take one machine work order out of the pool. Called by the shift, not the executor.

<!-- au:task claim -->
```
au task claim [OPTIONS] --owner <OWNER> --run <RUN> --lease-minutes <LEASE_MINUTES>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--owner <OWNER>` |  | Who is taking it: `smena@<host>/<pid>` |
| `--run <RUN>` |  | Number of the run that issued the work order |
| `--lease-minutes <LEASE_MINUTES>` |  | Lease duration in minutes (twice the wall-clock budget of one work order) |
| `--project <PROJECT>` |  | Filter by project (not part of the wave 1 contract — groundwork for fitness) |
| `--json` |  | JSON instead of human-readable text |
<!-- /au -->

### au task renew

Work order: extend the lease on a taken work order.

<!-- au:task renew -->
```
au task renew [OPTIONS] --id <ID> --owner <OWNER> --lease-minutes <LEASE_MINUTES>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--id <ID>` |  | Identifier of the work order issued by `claim` |
| `--owner <OWNER>` |  | The same owner that took the work order |
| `--lease-minutes <LEASE_MINUTES>` |  | New lease duration in minutes |
| `--json` |  | Print JSON instead of human-readable text |
<!-- /au -->

### au task release

Work order: record the outcome of the work.

<!-- au:task release -->
```
au task release [OPTIONS] --id <ID> --owner <OWNER> --verdict <VERDICT> --evidence <EVIDENCE>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--id <ID>` |  | Work order id issued by `claim` |
| `--owner <OWNER>` |  | The same owner that took the work order |
| `--verdict <VERDICT>` |  | done \| failed |
| `--evidence <EVIDENCE>` |  | Command or check that confirmed the outcome |
| `--json` |  | Print JSON instead of human-readable text |
<!-- /au -->

### au task give-up

Work order: hand it back as stuck on a human. Blocks the task; it does not return to the queue.

<!-- au:task give-up -->
```
au task give-up [OPTIONS] --id <ID> --owner <OWNER> --why <WHY>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--id <ID>` |  | Work order id issued by `claim` |
| `--owner <OWNER>` |  | The same owner that took the work order |
| `--why <WHY>` |  | Reason why the task cannot be closed by a machine |
| `--json` |  | Print JSON instead of human-readable text |
<!-- /au -->

### au task fitness

Work order: set an executability verdict by hand, or dry-run the labelling across the queue.

<!-- au:task fitness -->
```
au task fitness [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--id <ID>` |  | Task identifier — together with --verdict and --why it sets the verdict by hand. Required unless --dry-run is given |
| `--verdict <VERDICT>` |  | machine \| human \| split — required together with --id |
| `--why <WHY>` |  | Rationale — required and non-empty together with --id (FR-003a) |
| `--dry-run` |  | Dry run across all open tasks — writes nothing (SC-001) |
| `--project <PROJECT>` |  | Filter by project — takes effect only together with --dry-run |
| `--json` |  | Print JSON instead of human-readable text |
<!-- /au -->

### Reminders

The reminder model is described in [Reminders](reminders.md).

### au remind

Reminders with an explicit state and a journal of postponements. Bare `au remind` lists the open
ones; `--hook` is the session-side consumer wired to the Stop hook.

<!-- au:remind -->
```
au remind [OPTIONS] [COMMAND]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--hook` |  | Session-side consumer, wired to the Stop hook: prints reminders addressed to the AI and marks each one delivered through the conditional UPDATE in `reminders::mark_delivered`, so a crash between print and stamp cannot double-deliver it. Never fails — a broken reminder must not break the turn |
<!-- /au -->

### au remind add

Set a reminder. Exactly one of `--at` or `--in`.

<!-- au:remind add -->
```
au remind add [OPTIONS] <TEXT>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<TEXT>` |  | What to be reminded of |
| `--at <AT>` |  | Absolute moment — RFC 3339, `YYYY-MM-DD[ HH:MM]`, or a bare `HH:MM` for the next occurrence of that time |
| `--in <IN>` |  | Delay from now: a bare number of minutes, or a number suffixed `m`/`h`/`d`/`w` |
| `--task <TASK>` |  | Attach to a task (UUID or label) — inherits the task's project instead of `--project` |
| `--project <PROJECT>` |  | Project this reminder belongs to, when it is not attached to a task |
| `--repeat <REPEAT>` |  | Re-arm on delivery instead of settling into `delivered` for good — same delay grammar as `--in` |
| `--for <FOR>` | `both` | Who may be shown this: `me` (out-of-session channel only), `ai` (a session-side consumer only), or `both` |
| `--json` |  | Print one line of JSON instead of human-readable text |
<!-- /au -->

### au remind list

List reminders — pending and delivered by default.

<!-- au:remind list -->
```
au remind list [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--project <PROJECT>` |  | Filter by project |
| `--state <STATE>` |  | Filter by state: pending, delivered, done, cancelled |
| `--all` |  | Include done and cancelled reminders too |
| `--json` |  | Print JSON instead of human-readable text |
<!-- /au -->

### au remind show

One reminder with its full journal.

<!-- au:remind show -->
```
au remind show [OPTIONS] <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Reminder UUID or unique prefix (4+ characters) |
| `--json` |  | Print JSON instead of human-readable text |
<!-- /au -->

### au remind done

Mark a reminder done — acted on.

<!-- au:remind done -->
```
au remind done <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Reminder UUID or unique prefix |
<!-- /au -->

### au remind cancel

Cancel a reminder — the other outcome. CLI-only: there is no MCP `reminder_cancel`.

<!-- au:remind cancel -->
```
au remind cancel <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Reminder UUID or unique prefix |
<!-- /au -->

### au remind snooze

Postpone a reminder. Exactly one of `--in` or `--at`; the move is journaled.

<!-- au:remind snooze -->
```
au remind snooze [OPTIONS] <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Reminder UUID or unique prefix |
| `--in <IN>` |  | Delay from now, same grammar as `au remind add --in` |
| `--at <AT>` |  | Absolute moment, same grammar as `au remind add --at` |
<!-- /au -->

### au daemon

The single owner of the reminder clock and of the embedding model: each tick delivers overdue
`me`/`both` reminders to the out-of-session channel, then embeds a bounded batch from the
embedding queue. Runs as a systemd user unit; a second instance exits `15`.

<!-- au:daemon -->
```
au daemon [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--interval <INTERVAL>` | `60` | Pause between ticks, seconds |
| `--grace <GRACE>` | `15m` | How long after the due moment a reminder waits for a live session before the out-of-session channel takes it; same delay grammar as `au remind add --in` |
| `--once` |  | One tick instead of a loop, then exit: how the daemon is tested without sleeping, and how a systemd timer can replace the long-running service |
| `--json` |  | Print each tick as one JSON line instead of human-readable output |
<!-- /au -->

### Finance

The model behind these commands is described in [Personal finance](finance.md).

### au fin

Personal finance: spending, income and currency exchanges.

<!-- au:fin -->
```
au fin <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au fin log

Record one entry. Batches go through MCP `fin_log`.

<!-- au:fin log -->
```
au fin log [OPTIONS] --kind <KIND> --amount <AMOUNT> --currency <CURRENCY>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--kind <KIND>` |  | expense \| income \| exchange |
| `--amount <AMOUNT>` |  | Amount in currency units, e.g. `25` or `11.6`; for an exchange, the amount given |
| `--currency <CURRENCY>` |  | Currency code; for an exchange, the currency given |
| `--counter-amount <COUNTER_AMOUNT>` |  | Exchange only: the amount received |
| `--counter-currency <COUNTER_CURRENCY>` |  | Exchange only: the currency received |
| `--category <CATEGORY>` |  | Category, lowercase; required for expense and income |
| `--note <NOTE>` |  | Anything else worth keeping: card, place, counterparty |
| `--at <AT>` |  | When: `YYYY-MM-DD`, `YYYY-MM-DD HH:MM` or RFC 3339; defaults to now |
| `--source <SOURCE>` | `manual` | voice \| text \| manual \| import |
| `--provenance <PROVENANCE>` |  | Where the entry came from, e.g. a bot message id |
| `--json` |  | Print the written entry as JSON |
<!-- /au -->

### au fin list

Entries for a period, newest first, each with its USD equivalent.

<!-- au:fin list -->
```
au fin list [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--period <PERIOD>` |  | today \| yesterday \| week \| month \| year \| all \| <N>d |
| `--since <SINCE>` |  | Start, inclusive: YYYY-MM-DD or RFC 3339; overrides the period start |
| `--until <UNTIL>` |  | End, exclusive: YYYY-MM-DD or RFC 3339 |
| `--kind <KIND>` |  | expense \| income \| exchange |
| `--category <CATEGORY>` |  | Only this category |
| `--currency <CURRENCY>` |  | Only this currency, either side of an exchange |
| `--all` |  | Show voided entries too |
| `--limit <LIMIT>` | `50` | Most entries to print |
| `--json` |  | Print JSON instead of entry lines |
<!-- /au -->

### au fin stats

Totals and a breakdown in USD at the rate of each entry's date, native currencies beside them;
exchanges kept apart from spending.

<!-- au:fin stats -->
```
au fin stats [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--period <PERIOD>` |  | today \| yesterday \| week \| month \| year \| all \| <N>d; defaults to month |
| `--since <SINCE>` |  | Start, inclusive: YYYY-MM-DD or RFC 3339 |
| `--until <UNTIL>` |  | End, exclusive: YYYY-MM-DD or RFC 3339 |
| `--by <BY>` | `category` | category \| month \| currency |
| `--base <BASE>` | `USD` | USD \| none (none: native currencies only, no rates, no network) |
| `--json` |  | Print JSON instead of the text summary |
<!-- /au -->

### au fin void

Void an entry: the row stays, marked with when and why, and drops out of lists and totals.

<!-- au:fin void -->
```
au fin void [OPTIONS] --reason <REASON> <ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ID>` |  | Entry id (the #N in entry lines) |
| `--reason <REASON>` |  | Why the entry is voided, e.g. `wrong amount, re-logged as #14` |
| `--json` |  | Print JSON instead of the entry line |
<!-- /au -->

### au fin categories

Existing categories with their entry counts.

<!-- au:fin categories -->
```
au fin categories [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--json` |  | Print JSON instead of human-readable lines |
<!-- /au -->

### au fin currency

The currency directory.

<!-- au:fin currency -->
```
au fin currency <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au fin currency list

Enabled currencies; `--all` adds disabled ones.

<!-- au:fin currency list -->
```
au fin currency list [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--all` |  | Include disabled currencies |
| `--json` |  | Print JSON instead of human-readable lines |
<!-- /au -->

### au fin currency add

Add a currency, or re-enable a disabled one.

<!-- au:fin currency add -->
```
au fin currency add [OPTIONS] --kind <KIND> --scale <SCALE> <CODE>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<CODE>` |  | Currency code, 2-10 latin letters or digits |
| `--kind <KIND>` |  | fiat \| crypto |
| `--scale <SCALE>` |  | Decimal places, 0-8 |
| `--json` |  | Print JSON instead of a human-readable line |
<!-- /au -->

### au fin currency disable

Disable a currency: new entries are refused, old ones stay in totals.

<!-- au:fin currency disable -->
```
au fin currency disable [OPTIONS] <CODE>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<CODE>` |  | Currency code |
| `--json` |  | Print JSON instead of a human-readable line |
<!-- /au -->

### au fin rate

Rates to USD: a dictated rate for one entry, the cache, refreshing it.

<!-- au:fin rate -->
```
au fin rate <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au fin rate set

Dictate the rate for one entry — it beats every source.

<!-- au:fin rate set -->
```
au fin rate set [OPTIONS] <ENTRY_ID> <RATE>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ENTRY_ID>` |  | Entry id (the #N in entry lines) |
| `<RATE>` |  | The rate as a decimal string, e.g. 3.25 |
| `--currency <CURRENCY>` |  | Exchange only: which side the rate is for; defaults to the entry currency |
| `--unit <UNIT>` |  | per_usd (units for 1 USD, default for fiat) \| usd_per_unit (USD for 1 unit, default for crypto) |
| `--json` |  | Print JSON instead of the entry line |
<!-- /au -->

### au fin rate clear

Drop an entry's dictated rate; conversion falls back to the exchange, national bank or market rate.

<!-- au:fin rate clear -->
```
au fin rate clear [OPTIONS] <ENTRY_ID>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<ENTRY_ID>` |  | Entry id (the #N in entry lines) |
| `--currency <CURRENCY>` |  | Exchange only: which side to clear; defaults to the entry currency |
| `--json` |  | Print JSON instead of the entry line |
<!-- /au -->

### au fin rate list

Cached rates, newest first.

<!-- au:fin rate list -->
```
au fin rate list [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--currency <CURRENCY>` |  | Only this currency |
| `--date <DATE>` |  | Exactly this date, YYYY-MM-DD |
| `--since <SINCE>` |  | From this date, YYYY-MM-DD, inclusive |
| `--until <UNTIL>` |  | Up to this date, YYYY-MM-DD, inclusive |
| `--entry-id <ENTRY_ID>` |  | Only the dictated rate of this entry |
| `--limit <LIMIT>` | `100` | Most rates to print |
| `--json` |  | Print JSON instead of human-readable lines |
<!-- /au -->

### au fin rate refresh

Forget the cached national-bank and market rates for a date and fetch them again.

<!-- au:fin rate refresh -->
```
au fin rate refresh [OPTIONS] --date <DATE>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--date <DATE>` |  | YYYY-MM-DD |
| `--currency <CURRENCY>` |  | One currency; defaults to every enabled one |
| `--json` |  | Print JSON instead of human-readable lines |
<!-- /au -->

### Secret coordinates

See [Secrets](#secrets) above for what is stored and why.

### au secret

Where each project's secrets live — never the value.

<!-- au:secret -->
```
au secret <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au secret add

Record a coordinate. A `--where` that looks like a secret value itself is refused with exit `1`
and the rule that matched.

<!-- au:secret add -->
```
au secret add [OPTIONS] --name <NAME> --where <LOCATION>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--name <NAME>` |  | Secret name, e.g. STRIPE_SECRET_KEY |
| `--where <LOCATION>` |  | Where it is kept: an environment variable, a file path or a password-manager entry, NOT the value |
| `--purpose <PURPOSE>` |  | What the secret is for, in plain words |
| `--project <PROJECT>` |  | Project the secret belongs to |
<!-- /au -->

### au secret list

Recorded coordinates, without values.

<!-- au:secret list -->
```
au secret list [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-p, --project <PROJECT>` |  | Filter by project |
| `--json` |  | Print JSON instead of human-readable lines |
<!-- /au -->

### au secret rm

Remove a coordinate by name.

<!-- au:secret rm -->
```
au secret rm [OPTIONS] <NAME>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<NAME>` |  | Secret name |
| `--project <PROJECT>` |  | Project the secret belongs to |
<!-- /au -->

### Identity and sync

How sync works is described in [Project Sync](sync.md).

### au identity

Personal identity for sync attribution.

<!-- au:identity -->
```
au identity <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au identity set

Set the name and email stamped on every node and edge you write. Required once per machine
before `au share <SERVER> <TOKEN>`.

<!-- au:identity set -->
```
au identity set --name <NAME> --email <EMAIL>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--name <NAME>` |  | Your name |
| `--email <EMAIL>` |  | Your email |
<!-- /au -->

### au share

Share a project with another Aurelius instance over a sync server. The bare form
`au share <SERVER> <TOKEN>` connects a project with a token.

<!-- au:share -->
```
au share <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au share admin-set

Admin: store this machine's admin token for a server, so `issue` and `revoke` need no
`AURELIUS_SYNC_ADMIN_TOKEN` in the environment.

<!-- au:share admin-set -->
```
au share admin-set <SERVER> <TOKEN>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<SERVER>` |  | Sync server host or URL |
| `<TOKEN>` |  | The admin token the server was started with (AURELIUS_SYNC_ADMIN_TOKEN) |
<!-- /au -->

### au share issue

Admin: issue a collaborator a token for an existing local project.

<!-- au:share issue -->
```
au share issue [OPTIONS] --server <SERVER> <PROJECT>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<PROJECT>` |  | Project label — must already exist locally |
| `--for <FOR>` |  | Who the token is for, e.g. "Tester <tester@example.com>". Defaults to your own `au identity set` when issuing a token for yourself (e.g. to connect this same project from another machine) |
| `--server <SERVER>` |  | Sync server host or URL, e.g. aurelius.boostix.space or http://localhost:8181/sync |
<!-- /au -->

### au share revoke

Admin: revoke a collaborator's access to a project.

<!-- au:share revoke -->
```
au share revoke --for <FOR> --server <SERVER> <PROJECT>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<PROJECT>` |  | Project label |
| `--for <FOR>` |  | Collaborator email |
| `--server <SERVER>` |  | Sync server host or URL |
<!-- /au -->

### au share push

Push local changes to the sync server.

<!-- au:share push -->
```
au share push [PROJECT]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[PROJECT]` |  | Project to push (default: every project with sync enabled) |
<!-- /au -->

### au share pull

Pull changes from the sync server.

<!-- au:share pull -->
```
au share pull [PROJECT]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[PROJECT]` |  | Project to pull (default: every project with sync enabled) |
<!-- /au -->

### au share list

List every project connected to sync.

<!-- au:share list -->
```
au share list
```

No arguments or options besides `--help`.
<!-- /au -->

### au share disable

Disable sync for a project, locally; synced data is not deleted.

<!-- au:share disable -->
```
au share disable <PROJECT>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<PROJECT>` |  | Project label |
<!-- /au -->

### Database

See [Backups](#backups) and [Maintenance](#maintenance) above.

### au db

Database maintenance.

<!-- au:db -->
```
au db <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au db check

Verify integrity. Read-only — never migrates, never writes. A path checks a snapshot.

<!-- au:db check -->
```
au db check [OPTIONS] [PATH]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[PATH]` |  | Database to check (default: the knowledge graph). Use it to verify a snapshot |
| `--full` |  | Report every problem instead of stopping at the first |
<!-- /au -->

### au db migrate

Apply pending schema migrations.

<!-- au:db migrate -->
```
au db migrate
```

No arguments or options besides `--help`.
<!-- /au -->

### au db backup

Safe snapshot via SQLite `VACUUM INTO` — the only correct way to copy a live database.

<!-- au:db backup -->
```
au db backup [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-o, --out <OUT>` |  | Destination file (default: <data_dir>/backups/aurelius-<UTC timestamp>.db) |
| `--hook` |  | SessionStart-hook mode: snapshot into <data_dir>/backups, throttled by --min-hours, verified, rotated to --keep; never fails |
| `--keep <KEEP>` |  | Snapshots to keep (default: AURELIUS_BACKUP_KEEP env var, else 7). Requires --hook |
| `--min-hours <MIN_HOURS>` |  | Minimum age in hours of the newest snapshot before another is taken (default: AURELIUS_BACKUP_MIN_HOURS env var, else 24). Requires --hook |
<!-- /au -->

### au db reindex-embeddings

Queue live nodes that have no vector; the daemon embeds them. Loads no model itself.

<!-- au:db reindex-embeddings -->
```
au db reindex-embeddings
```

No arguments or options besides `--help`.
<!-- /au -->

### au db prune

List what the four prune rules would remove; `--apply` soft-deletes them. Nothing with a claim
is removed. (The binary's help text still says "three rules"; the code has four, listed in
[Maintenance](#maintenance).)

<!-- au:db prune -->
```
au db prune [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--apply` |  | Remove the listed nodes (soft delete, one transaction) |
| `--json` |  | Machine-readable plan: every candidate, not a sample |
<!-- /au -->

### au db purge-orphans

Count vectors whose node is deleted or missing; `--apply` removes them.

<!-- au:db purge-orphans -->
```
au db purge-orphans [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--apply` |  | Remove the orphan vectors |
<!-- /au -->

### au db scrub-trace

Count stored trace payloads that secret masking would change; `--apply` rewrites them.

<!-- au:db scrub-trace -->
```
au db scrub-trace [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--apply` |  | Rewrite the payloads |
<!-- /au -->

### au db trace-retention

Count traces older than `--days`; `--apply` removes them.

<!-- au:db trace-retention -->
```
au db trace-retention [OPTIONS] --days <DAYS>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--days <DAYS>` |  | Age in days (at least 30) |
| `--apply` |  | Remove the old traces |
<!-- /au -->

### au db reattach

Link records that name a project in `data.project` to that project's hub with a `belongs_to`
edge. Missing hubs are only counted. Dry run unless `--apply`.

<!-- au:db reattach -->
```
au db reattach [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--apply` |  | Add the missing belongs_to edges (one transaction) |
<!-- /au -->

### au db prune-backups

List snapshots beyond the `--keep` newest, next to the database and in `backups/`; `--apply`
removes them.

<!-- au:db prune-backups -->
```
au db prune-backups [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--keep <KEEP>` | `7` | Snapshots to keep, newest first |
| `--apply` |  | Remove the listed snapshots |
<!-- /au -->

### Documents

### au doc

Convert documents to Markdown locally, and search what was converted.

<!-- au:doc -->
```
au doc <COMMAND>
```

No arguments or options besides `--help`.
<!-- /au -->

### au doc convert

Convert a file, or a directory with `-r`, to Markdown; cached by content hash.

<!-- au:doc convert -->
```
au doc convert [OPTIONS] <PATH>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<PATH>` |  | File or directory to convert |
| `-o, --out <OUT>` |  | Write the Markdown here instead of to stdout |
| `-r, --recursive` |  | Directory mode: descend into subdirectories |
| `--force` |  | Re-convert even if this content was converted before |
<!-- /au -->

### au doc recall

Full-text search over everything ever converted, even if the file is gone.

<!-- au:doc recall -->
```
au doc recall [OPTIONS] <QUERY>
```

| Argument / option | Default | Meaning |
|---|---|---|
| `<QUERY>` |  | FTS5 query over document text and file names |
| `-l, --limit <LIMIT>` | `10` | Max results |
<!-- /au -->

### Hook plumbing

These are called by the plugin hooks listed in [Hooks](hooks.md), not by hand.

### au session-hook

Record the Claude Code session id for its process (SessionStart).

<!-- au:session-hook -->
```
au session-hook [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--hook` |  | Read Claude Code SessionStart hook JSON from stdin |
<!-- /au -->

### au hint

Show what memory knows about a file before its first edit in a session (PreToolUse).

<!-- au:hint -->
```
au hint [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--hook` |  | Read Claude Code PreToolUse hook JSON from stdin |
<!-- /au -->

### au trace

Append an action trace to the journal; `--hook` reads PostToolUse JSON from stdin.

<!-- au:trace -->
```
au trace [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-k, --kind <KIND>` |  | tool_call\|file_edit\|error\|commit\|msg_sent\|user_correction |
| `-m, --payload <PAYLOAD>` |  | Trace payload text |
| `-s, --session <SESSION>` |  | Session id (defaults to "cli") |
| `-e, --exit-code <EXIT_CODE>` |  | Exit code of the traced action |
| `--hook` |  | PostToolUse hook mode: parse hook JSON from stdin, never fail |
<!-- /au -->

### au judge

Close ripe labile windows and apply outcome verdicts (Stop hook).

<!-- au:judge -->
```
au judge [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `-m, --min-age <MIN_AGE>` | `0` | Only close windows at least this many seconds old (default 0) |
| `--hook` |  | Stop-hook mode: never fail |
<!-- /au -->

### Evaluation

### au eval

Score recall against a frozen fixture. Reads only; exit `14` means no run, no numbers.

<!-- au:eval -->
```
au eval [OPTIONS] [CASES]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[CASES]` |  | Case file (JSONL, format in `contracts/eval-cases.md`); defaults to `fixtures/eval/cases.jsonl` under the repository root |
| `--db <DB>` |  | Fixture dump to run against; defaults to `meta.fixture` of the case file. A path ending in `.zst` is unpacked into a temporary file |
| `--now <NOW>` |  | RFC3339 instant the node ages are measured from; defaults to `meta.as_of`. The clock is a parameter, never the system time: without a fixed instant two runs of one fixture print two numbers |
| `--live` |  | Run against the live database instead of a fixture. The report is marked incomparable and its numbers never go into `research.md` |
| `--json` |  | Machine-readable report: one JSON object on one line |
<!-- /au -->

### au eval-search

Search-quality baseline — recall@5, recall@10 and MRR@10 by query class for FTS-only, dense-only
and fused search. Dense and fused are skipped when the embedding daemon does not answer.

<!-- au:eval-search -->
```
au eval-search [OPTIONS] [CASES]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `[CASES]` |  | Case file; defaults to `fixtures/eval/search-baseline.jsonl` |
| `--db <DB>` |  | Database to measure; defaults to the live one |
| `--json` |  | One JSON object instead of the table |
<!-- /au -->

### Retired commands

Both still parse, print why they were removed, and exit `1`.

### au sync

The retired TimeForged connector.

<!-- au:sync -->
```
au sync
```

No arguments or options besides `--help`.
<!-- /au -->

### au capture

Retired; its hook was never wired into any project.

<!-- au:capture -->
```
au capture [OPTIONS]
```

| Argument / option | Default | Meaning |
|---|---|---|
| `--hook` |  | Accepted for parsing compatibility only |
| `-c, --command <COMMAND>` |  | Accepted for parsing compatibility only |
<!-- /au -->

## Regenerating the reference

The usage lines and tables above come from the binary:

```bash
python3 scripts/dump-au-help.py --update docs/cli.md   # after a CLI change
python3 scripts/dump-au-help.py --check docs/cli.md    # exit 1 when the doc is stale
AU=target/release/au python3 scripts/dump-au-help.py --check docs/cli.md   # a freshly built binary
```

The script fails when a subcommand has no block here, when a block names a command the binary
does not have, or when a help string printed in Russian has no English text in the script.
