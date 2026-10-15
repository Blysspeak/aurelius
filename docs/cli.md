# CLI

[Back to README](../README.md)

The `au` binary: one command line for the graph, tasks, reminders, sync and database care.

## Contents

- [Command overview](#command-overview)
- [Exit codes](#exit-codes)
- [Provenance](#provenance)
- [Command groups](#command-groups)
- [Secrets](#secrets)
- [Backups](#backups)
- [Maintenance](#maintenance)

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
au db check [PATH]                 # verify integrity (read-only); PATH verifies a snapshot
au db backup                       # safe snapshot via VACUUM INTO
au db reindex-embeddings           # queue nodes without a vector for the daemon
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

Larger command families live next to the feature they drive:

| Group | Reference |
|-------|-----------|
| `au task …` | [Task Commands](tasks.md#task-commands) |
| `au remind …`, `au daemon` | [Reminder Commands](reminders.md#reminder-commands) |
| `au identity …`, `au share …` | [Sync Commands](sync.md#sync-commands) |

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
au db backup         # snapshot → aurelius-<UTC timestamp>.db next to the database, mode 0600
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
au db prune-backups --keep 7          # snapshots beyond the N newest, next to the database and in backups/
```

`au db prune` has four rules: `technical_orphan` (a run or dependency node without edges, claim
or body), `technical_junk` (any run node, and any dependency node without claim or body, even
with edges — task evidence lives in the task, dependencies in the manifest), `stale_digest` (a
project distillate that is not the newest) and `empty_project` (a project node with no edges,
content or records naming it). Removal is a soft delete in one transaction. Nothing writes run or dependency nodes any more; the
rules clear what older versions left behind.

New trace payloads are masked for secrets before they are written; `scrub-trace` rewrites the ones
stored before that.
