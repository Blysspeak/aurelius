<p align="center">
  <img src="logo.png" width="200" alt="Aurelius" />
</p>

<h1 align="center">Aurelius</h1>

<p align="center">
  <strong>Self-hosted knowledge graph memory for developers and AI agents.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/v1.11.1-stable-a6e3a1?style=flat-square" alt="v1.11.1">
  <img src="https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white&style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white&style=flat-square" alt="SQLite">
  <img src="https://img.shields.io/badge/MCP-32_tools-a6e3a1?style=flat-square" alt="MCP">
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="#mcp-tools-32">MCP Tools</a> ·
  <a href="#memory-snapshot">Snapshot</a> ·
  <a href="#task-management">Tasks</a> ·
  <a href="#project-sync">Sync</a> ·
  <a href="#web-ui">Graph UI</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

## The Problem

Every AI session starts from zero. You re-explain your projects, your past decisions, your architecture. Tasks scatter across tools with no memory of what was done.

**With Aurelius:** `memory_status` → full project context. `task_view` → complete work history. `memory_session` → nothing lost between sessions.

---

## Quick Start

Aurelius ships as a [Claude Code plugin](plugin/hooks.json): seven session hooks, skill cards and
the `/pickup` command, installed with two `claude plugin` commands instead of hand-edited
`settings.json`. The MCP server is registered separately, user-scope, by `install.sh` (`claude mcp
add -s user aurelius au mcp`) — a plugin-bundled server would rename every tool to
`mcp__plugin_aurelius_aurelius__*`.

**Clean machine (Linux, macOS)**

```bash
git clone https://github.com/Blysspeak/aurelius && cd aurelius
cargo build --release
install -m 755 target/release/au target/release/aurelius ~/.local/bin/   # must be in PATH
au init
claude mcp add -s user aurelius au mcp   # MCP server, user scope: tools stay mcp__aurelius__*
claude plugin marketplace add Blysspeak/aurelius      # or a local clone path: claude plugin marketplace add .
claude plugin install aurelius@blysspeak -s user
```

Restart Claude Code. `claude plugin list` shows `aurelius@blysspeak`, and a fresh session gets a
memory snapshot and skill index at start.

**Existing machine (stood up by hand or by an older `install.sh`)**

```bash
cd aurelius && git pull && ./install.sh
```

`install.sh` builds the binaries, installs the plugin, registers the MCP server user-scope with
`claude mcp add -s user aurelius au mcp`, and removes any legacy hook and `mcpServers.aurelius`
entries it previously wrote into `~/.claude/settings.json` and `~/.claude.json` — printing each
removed entry with its reason and leaving a `.bak-<UTC date>` copy next to each file it touches.
Migration keeps a canonical `au mcp` entry in `~/.claude.json` and removes only the old
wrapper-path entry; re-running the whole script, including the MCP registration, is a no-op once
migrated. Use `./install.sh --migrate-only` to run just the cleanup, without building anything or
registering the server.

**Windows**

```powershell
git clone https://github.com/Blysspeak/aurelius; cd aurelius
cargo build --release
New-Item -ItemType Directory -Force "$env:USERPROFILE\.local\bin" | Out-Null
Copy-Item target\release\au.exe "$env:USERPROFILE\.local\bin\"    # must be in PATH
au init
claude mcp add -s user aurelius au mcp
claude plugin marketplace add Blysspeak/aurelius
claude plugin install aurelius@blysspeak -s user
```

Git Bash and python3 are not required — every plugin hook runs `au` directly. If hooks were
previously added by hand, remove them from `$env:USERPROFILE\.claude\settings.json` (hooks whose
command matches `aurelius-*.sh` or `au … --hook`) — copy the file first. A `mcpServers.aurelius`
entry in `$env:USERPROFILE\.claude.json` is now the right place, when its command is `au mcp`;
only an entry pointing at the old wrapper script is stale — replace it with
`claude mcp add -s user aurelius au mcp`.

```
$ au --version
au 1.11.1
```

---

## MCP Tools (33)

Aurelius runs as an MCP server over stdio. `install.sh` registers it user-scope with
`claude mcp add -s user aurelius au mcp`, and the same command adds it by hand.

### Knowledge Graph

| Tool | Description |
|------|-------------|
| `memory_status` | Session start — full project snapshot with active tasks and a `server` block (running MCP server version, `started_at`, and `restart_needed` when the binary on disk is newer than this running process — installing a new build over the old one doesn't kill an already-running MCP server). Optional `project` filter. |
| `memory_session` | Session end — save decisions, problems/solutions, next steps. Links to tasks. Returns active tasks hint. SHA-256 dedup. Accepts the same provenance fields as `memory_add` (`confidence`, `evidence`, `subject`, `volatility`, `claim`, `measured_at`, `verify_with`); spawned decisions/problems/solutions inherit confidence/evidence but never subject/claim. |
| `memory_recall` | Smart topic recall — FTS + BFS, grouped by type (incl. tasks), skips structural noise. |
| `memory_search` | Full-text search with `type`, `since`, and `limit` filters. `*` for recent. Words are OR-ed and ranked by how many of them matched, so one bad word form spoils the order rather than the result; `unmatched_terms` names the words that matched nothing, telling "no such knowledge" apart from "the query didn't work". |
| `memory_context` | Raw BFS graph traversal from FTS seed nodes. |
| `memory_path` | Directed step ladder over `next_step`/`prerequisite` edges: shortest path between two nodes, or every node that transitively leads to one target. Same computation as `au path`. |
| `memory_snapshot` | The seven-layer frozen slice as Markdown, under a hard budget. |
| `memory_consolidate` | Rebuild a project's distillate — open next steps plus unsolved problems. Idempotent. |
| `memory_add` | Create node with label, type, note, data (JSON), memory_kind. Pass `project` to link it; without a link the response says so instead of reporting a silent success. |
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
| `task_create` | Create structured task — title, description, acceptance criteria, priority, subtask/blocking relations. Accepts the same provenance fields as `memory_add` (`confidence`, `evidence`, `subject`, `volatility`, `claim`, `measured_at`, `verify_with`). |
| `task_update` | Update status, priority, criteria. Auto-tracks `started_at`/`completed_at`. Accepts the same provenance fields as `memory_add` — a task's confidence can change after a measurement. |
| `task_list` | Filter by project, status, priority. Sorted by priority, shows work log count. Evidence is an `EvidenceSummary` (total/green/last_green), not the full run array — that stays in `task_view`. `full_notes=true` returns each note whole instead of truncated. |
| `task_log` | Record work done — creates WorkLog + optional Decision/Problem/Solution nodes. Never changes the task's status; the response includes `task_status`, activation is explicit via `task_update status=active`. Accepts the same provenance fields as `memory_add`; spawned decisions/problems/solutions inherit confidence/evidence but never subject/claim. |
| `task_view` | Full task branch — timeline of work logs, decisions, problems, solutions, subtasks. |
| `task_stats` | Task analytics — counts by status/priority, completion rate, avg/median duration, blocked count, oldest active. |
| `task_ripe` | Tasks ready to close — active, with a passing evidence run newer than the last edit, plus the basis (which run, when, files touched). Same computation as `au task ripe`; closing itself is still `task_update`. |

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
| `search_web` | Brave Search API or Perplexity Search API, picked via `provider` (default `brave`). SQLite cache is scoped per provider. Repeat queries served from cache. Optional `save_to_graph`. |
| `search_recall` | FTS search through cached web search results from past sessions. |

### Documents

Word, PowerPoint, Excel, OpenDocument, RTF, EPUB, CSV, PDF, HTML and plain text into GitHub-Flavored Markdown, converted locally — no network, no API key. Everything converted stays searchable afterwards, even once the original file is gone. Out of scope: audio/video transcription and OCR of scans.

| Tool | Description |
|------|-------------|
| `doc_convert` | Convert a file, or every file in a directory, to Markdown. Large output spills to a `.md` file and returns an outline + preview instead of filling the context. Optional `save_to_graph`. |
| `doc_read` | Paginated read of an already-converted document, by content hash or path. |
| `doc_recall` | FTS search across every document ever converted. |

### Secrets

Read-only on MCP, deliberately: Aurelius stores where a secret lives, never the value (see
[Secrets](#secrets-1) below), and recording a coordinate is a human act — `au secret add`/`rm`
stay CLI-only. Only the read side is exposed here, because it answers a question an assistant
gets asked directly ("where's the Stripe key?") and the coordinate is otherwise unreachable
through MCP: it never appears in `memory_snapshot` or any other automatic dump.

| Tool | Description |
|------|-------------|
| `secret_list` | Name, purpose, and location of every recorded secret coordinate — never the value. |

---

## Memory Snapshot

The snapshot is how the graph reaches a session. It is a small curated slice injected
**once** at session start — frozen for the session, so it does not break the prefix
cache — rather than a large JSON blob fetched on demand.

```
1 · Owner                       what is known about you
2 · In progress                 open tasks and unsolved problems
3 · Pressure                    obligations taken on and not yet settled
4 · Recent sessions             what the last sessions concluded
5 · Decisions and knowledge     decisions, then concepts
6 · Practices                   skills
7 · Archive                     node/edge counts and where to dig deeper
8 · Distillate                  the structural residue of the layers above
```

Every layer has a hard character budget (~4.5K in total, on the order of 1.5K tokens),
and empty layers are omitted entirely.

**Active tasks are the one exception.** A task actually in progress — status `active` —
is pulled out of layer 2 before the budget is applied and rendered in full, uncut by the
per-layer character limit that trims everything else. There is still a hard ceiling (20
active tasks per project) purely against unbounded growth; past it the snapshot says how
many did not fit rather than dropping them silently. Whatever an active task's own
rendering costs beyond the layer's normal budget is taken from **layer 5 (Decisions and
knowledge)**, not from the active tasks themselves and not from the layers in between —
the layer immediately below "In progress" in priority pays for it.

```bash
au snapshot --project myapp          # Markdown, for humans and for context
au snapshot --project myapp --json   # fixed shape, for programs
au snapshot --hook                   # Claude Code SessionStart envelope
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

`task_log` never activates a task — recording work is an observation, not a decision to take it
into work. Activation is explicit: `task_update` with `status=active`, or `au task activate`.
`task_update` tracks timestamps automatically.
A project has at most one active task at a time — activating another demotes the previous
active task back to `backlog`, keeping its accumulated timestamps and history intact.

### Three Timestamps

Every task carries three moments: when it was **created** (Заведена), when it was **taken
into work** (Взята — first activation), and when it was **closed** (Закрыта). `au task show`
and `task_view` print all three, an unfilled one as an explicit dash (`—`), never blank space:

```
Заведена: 2026-08-30T11:32:11+00:00
Взята:    —
Закрыта:  —
```

Reopening a closed task never erases these, or the resolution recorded at close time; they
become history the task carries forward.

### Evidence and Ripening

A verify run (`ulika`'s gate, e.g. `node scripts/verify-run.mjs …`) reports its outcome back
to Aurelius with `au task evidence` — command, exit code, and artifact path, attached to the
project's active task. **This is not a command you run by hand**: the ulika hook
(`record-verify.mjs`) calls it after every gate run, because it is the one thing that knows a
run just happened. You will see it in scripts and hook logs, not type it yourself.

A task **ripens** once it has at least one code edit *and* a green (exit-0) evidence run newer
than that edit — discussing a task without touching code never ripens it, and a later edit
un-ripens it again until a fresh green run covers it. `au task ripe` lists what's ripe along
with why: which evidence, when, what changed. `au judge --hook` (the Stop hook run at the end
of a turn) surfaces the same list unprompted, so a finished task gets presented for closing
without anyone having to ask — closing itself still requires a human decision (`au task done`).
`au task list` marks a ripe task inline too. Declining a proposal (`au task ripe --decline
<id>`) suppresses it on that task until new work lands.

### Closing with a Resolution

`au task done <id>` records *how* a task was closed, assembled from existing traces of work
rather than asked of the human:

```bash
au task done <id>                          # commit auto-detected via `git rev-parse --short HEAD`
au task done <id> --commit abc1234         # override the detected commit
au task done <id> --pr https://github.com/…/pull/42
au task done <id> --unconfirmed            # force "closed without confirmation", even if a commit was detected
```

If nothing about the resolution can be determined and `--unconfirmed` isn't passed, the task
still closes — just marked **closed without confirmation**, so the history never silently
lies about how sure the record is. A task closed straight from the backlog, without ever
being taken into work (`activated_at` absent), gets no auto-collected files and no
auto-detected commit either — there is no work window to read either from, only an
explicit `--commit`/`--pr` is kept.

### Acceptance Criteria

Every task can have a Definition of Done checklist:

```bash
au task new "Implement auth" --project myapp --priority high \
  -c "JWT tokens work" \
  -c "Refresh flow tested" \
  -c "Rate limiting active"
```

### Task Leasing (`au task claim` / `renew` / `release` / `give-up`)

A task handed to a runner used to be indistinguishable from one still sitting in the
queue — two processes could grab the same one, and an abandoned one never came back.
`au task claim` leases one machine-fit task to an owner for a fixed number of minutes
and hands it to nobody else while that lease holds; `au task renew` extends the lease
while the runner is still alive; `au task release` records the outcome. The grant is a
single `UPDATE … RETURNING`, so two concurrent `claim` calls cannot land on the same
task — not "unlikely", but structurally impossible.

`claim` also honours the one-active-task-per-project rule the other two entry points
(`au task activate`, MCP `task_update`) enforce: if the project already has a different
active task, that one is evicted back to the queue first. The exception is an active task
still under someone else's live lease — there `claim` declines instead, and the task it
had just taken is rolled back untouched. Evicting would return that task to the pool
while its lease still holds, so a third owner could claim it: one double-grant would be
traded for another.

```bash
au task claim --owner smena@host/123 --run 42 --lease-minutes 50   # take one machine-fit task
au task renew --id <id> --owner smena@host/123 --lease-minutes 50  # keep the lease alive
au task release --id <id> --owner smena@host/123 \
  --verdict done --evidence "cargo test — 186 passed"              # or --verdict failed
au task give-up --id <id> --owner smena@host/123 \
  --why "needs a human decision"                                   # blocks, does not requeue
```

`release --verdict done` closes the task through the same rule as `au task done` and the
MCP `task_update`: it stamps the close time and assembles the resolution from traces of
the work, and the `--evidence` text is kept as an ordinary run record, visible under
`au task show`. It does **not** yet judge the verdict it is handed — the runner's word
that the work is done is taken at face value. Gating the close on the run's exit code, on
a green check postdating the lease, and on every acceptance criterion having a recorded
check is designed but not implemented; until it is, that judgement lives in the driver
that calls these commands. `--verdict failed` always requeues and starts a cooldown. A
lease that simply expires is picked up again by the next `claim` the same way, attempts
climbing each time; a task claimed three times without a `done` verdict drops out of
`claim`'s selection instead of cycling through the queue forever. `give-up` is the one
exit that does **not** requeue: the runner recognized the block needs a human, so the
task is left in place with a reason attached rather than retried. These four are
**dispatcher-only** commands — a single external driver is meant to call them in a loop
against the whole queue; that outer loop is not shipped yet, only the primitives it will
call.

### Fitness Gate (`au task fitness`)

Before a task can be leased at all, something has to decide whether a machine could
possibly finish it. `au task fitness` writes that verdict onto `fitness` and nothing
else. A criterion counts as machine-checkable only when the check itself reads as a
command — at the start of a line, wrapped in backticks, or next to an explicit pass/fail
marker — not merely mentioned in prose (an earlier pass over the live queue counted
"reads NodeInbound" as if it were a runnable check and overstated the machine-fit pool by
half). A task with no such criterion is marked `human`; one with a mix of checkable and
non-checkable criteria is marked `split` rather than partially auto-run. Every verdict
requires a non-empty reason and is stamped with a hash of the task's content — edit the
task afterward and the verdict goes stale instead of quietly outliving the text it judged.

```bash
au task fitness --id <id> --verdict machine --why "single command, exit code checked"
au task fitness --dry-run --project myapp   # verdict + reason for every open task, writes nothing
```

`--dry-run` writes nothing; it exists to be read for the *why* — the verdict is
unattended, nobody confirms it before a task becomes claimable.

### Integration

- **`memory_status`** shows active/blocked tasks at session start
- **`memory_session`** accepts `tasks` parameter to link sessions to tasks, returns active tasks as hints
- **`memory_recall`** includes tasks in search results
- **`task_view`** aggregates the full work branch via BFS traversal
- **`au judge --hook`** surfaces ripe tasks unprompted at the end of a turn — see
  [Evidence and Ripening](#evidence-and-ripening)

---

## Project Sync

Share a single project between two Aurelius instances — an owner and one or
more collaborators — over a self-hosted sync server, so new decisions, tasks,
and work log entries made on either side show up on the other at the next
session, attributed to who made them.

```
[Owner instance]  <--push/pull-->  [aurelius-sync-server]  <--push/pull-->  [Collaborator]
```

- **Bootstrap once, per machine:** `au identity set` configures who you are
  (stamped as `"Name <email>"` on everything you create/update). The owner
  then issues each collaborator a one-time token (`au share issue`); the
  collaborator connects with a single command, `au share <server> <token>` —
  no manual export/import, and their instance receives the project's full
  existing history.
- **Automatic thereafter:** `memory_status` pulls and `memory_session` pushes
  for any sync-enabled project; `au share push`/`pull` do the same from the
  CLI. Both are best-effort — an unreachable server never blocks local work.
- **Deletions and conflicts are safe:** deletions propagate as tombstones
  (never resurrected on a later sync); a same-record conflict resolves
  deterministically last-writer-wins, with the losing edit retained under
  `data._sync_conflict` for recovery (`au context <project> -v`).

See [`deploy/aurelius-sync-server/README.md`](deploy/aurelius-sync-server/README.md)
to self-host a sync server.

---

## CLI

```bash
au init                            # initialize database
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
  --subject xhub:.env:REFUND_REQUESTS_ENABLED  # where it came from, how fast it rots, what it is about
au journal --session $SESSION_ID    # everything that run wrote — the selection a session-end hook needs
au context beacon                  # graph around a topic
au search "redis"                  # full-text search
au reindex                         # index current project
au view                            # open web graph UI
au touch path/to/file              # track file access
au export                          # export full graph as JSON
au mcp                             # start MCP server
au skills                          # print the skill index
au snapshot -p myapp [--json]      # seven-layer slice: Markdown, or a fixed JSON shape
au trace -m "what just happened"   # append to the action journal (or --hook on PostToolUse)
au judge                           # settle the session: reinforce, erode, fork or null
au db check [PATH]                 # verify integrity (read-only); PATH verifies a snapshot
au db backup                       # safe snapshot via VACUUM INTO
au doc convert report.docx         # document → Markdown on stdout
au doc convert ./contracts -r      # convert a whole tree, cached by content hash
au doc recall "termination"        # search everything ever converted
au task claim --owner … --run … --lease-minutes 50   # dispatcher-only: lease one machine-fit task
au task renew --id … --owner … --lease-minutes 50    # dispatcher-only: extend a held lease
au task release --id … --owner … --verdict done --evidence "…"  # dispatcher-only: report the outcome
au task give-up --id … --owner … --why "…"           # dispatcher-only: block, don't requeue
au task fitness --dry-run [--project myapp]           # is the open queue machine-checkable? read-only
```

> **Removed:** `au sync` (the TimeForged connector — spec 007 found zero calls to it across
> hooks and 19 repositories) and `au capture` (its calling hook was never wired into any
> project) were pulled after a usage audit found no consumer for either. Both subcommands
> still parse — calling one prints an explanation and what replaced it, and exits `1`, rather
> than failing as an unknown-argument error. If a script of yours still calls `au sync` or
> `au capture`, that script is dead weight: nothing downstream was reading their output either.

### Exit codes

A caller has to tell "I called it wrong" from "the store is unreachable": the first is
fixed by calling differently, the second by hand — retrying it is pointless.

| Code | Meaning |
|------|---------|
| `0` | done (`--help` and `--version` included) |
| `1` | bad call — unknown `--type`, missing argument, node not found, malformed JSON on stdin |
| `2` | storage unreachable — no database, damaged image, locked SQLite |

The `--hook` variants (`au snapshot --hook`, `au trace --hook`, `au judge --hook`) are the
deliberate exception: they never fail and stay silent on error. A broken hook is worse than
a missing snapshot.

### Provenance

A fact about the world is worth exactly as much as the answer to "how do you know?".
Without that, a guess lands in the graph looking identical to a measurement — and reads
back as truth six weeks later.

| Field | What it is for |
|-------|----------------|
| `confidence` | `measured` \| `inferred` \| `reported` \| `unverified`. **Required** on `memory_add`. Absent reads as `unverified`, never as "probably measured". `measured` without `evidence` is refused — that is `inferred`. |
| `evidence` | The command or query **verbatim** that produced the fact |
| `measured_at` | When it was measured; defaults to now for `measured` |
| `claim` | The assertion in one or two lines (≤240 chars). Returned **whole** — recall clips the long note, never the claim |
| `volatility` | `immutable` \| `slow` (30 d) \| `volatile` (1 d). Past that age the fact comes back marked "старше N дн — перепроверь" |
| `verify_with` | The command that re-checks it |
| `subject` | What is being asserted, e.g. `xhub:.env:REFUND_REQUESTS_ENABLED`. A second fact about the same subject is **refused** until `resolution` says `supersede` \| `refine` \| `coexist` — and the resolution becomes an edge, not a memory of one |

A failed probe (`probe_warnings`) does **not** downgrade `confidence`. Probes extract
path-like tokens from prose and check them on disk — weak evidence by construction: an
aliased import, a path on another machine, a file in someone else's repo all fail it
while saying nothing about the fact. `evidence` with a command and an exit code is
strong evidence, and the weak one has no business overruling the strong one. A failed
probe stays a remark, not a verdict.

Both doors take the same fields and share one parser: `au note --confidence …` and
`memory_add(confidence=…)` cannot drift apart on what a measured fact is.

### Sync Commands

Project sync (`identity`/`share`, below) — not the removed `au sync` TimeForged connector
(see the CLI section's removal note).

```bash
au identity set --name "Name" --email you@example.com          # once per machine

au share issue <project> --for "Name <email>" --server <host>   # owner: mint a token
au share revoke <project> --for <email> --server <host>         # owner: revoke access

au share <server> <token>          # collaborator: bootstrap + connect (once per project)
au share push [project]            # push local changes (default: every enabled project)
au share pull [project]            # pull remote changes (default: every enabled project)
au share list                      # show connected projects
au share disable <project>         # stop syncing (local data kept)
```

### Task Commands

```bash
au task new "Title" -p myapp --priority high -c "Tests pass"
au task list --project myapp --status active,blocked
au task show <id>                  # full details with work log branch and three timestamps
au task log <id> "Did X, Y, Z"    # record work (does not activate; see `au task activate`)
au task ripe [--project myapp]     # tasks ripe to close, with evidence + what changed
au task done <id>                  # mark complete; resolution assembled from traces
au task done <id> --commit <sha> --pr <url>   # override/add to the detected resolution
au task done <id> --unconfirmed    # force "closed without confirmation"
au task block <id> "waiting on API keys"
au task activate <id>              # resume blocked task, demotes any other active task

# called by the ulika verify hook, not by hand:
au task evidence --project myapp --command "npm test" --exit 0 --artifact run.log
```

### Secrets

Aurelius stores where a secret lives, never the value. `--where` is checked for strings that
look like the value itself (a key, a token, a connection string with credentials embedded) and
the write is refused, with an explanation, when it matches.

```bash
au secret add --name STRIPE_SECRET_KEY --where "env:STRIPE_SECRET_KEY" \
  --purpose "charge webhooks" --project myapp
au secret list --project myapp     # coordinates only — values were never stored
au secret rm STRIPE_SECRET_KEY --project myapp
```

Coordinates are returned on request only; they never appear in the memory snapshot or any
other automatic dump.

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
    src/graph/       — crud.rs, search.rs, traverse.rs, snapshot.rs (layers + distillate)
    src/db.rs        — SQLite setup, migrations V1-V12
    src/models.rs    — Node, Edge, NodeType, Relation, MemoryKind
    src/indexer.rs   — Cargo.toml project indexer
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
      handlers/      — status.rs, session.rs, crud.rs, search.rs, task.rs
      tools.rs       — MCP tool definitions (JSON schemas)
      mod.rs         — JSON-RPC 2.0 server
    src/search/
      brave.rs       — Brave Search API client
      perplexity.rs  — Perplexity Search API client
      cache.rs       — SQLite search cache with FTS5, scoped per provider
  au/                — CLI + web UI server (+ `identity`/`share` sync commands)
ui/                  — React + TypeScript + Tailwind (graph visualization)
contrib/
  claude-code/       — session hooks (reindex, track edits)
  git-hooks/         — post-commit (captures decisions)
deploy/
  aurelius-sync-server/ — Dockerfile + docker-compose for self-hosting the sync server
```

### Key Design

- **SQLite + WAL** — concurrent reads, single writer, local-first. Every connection sets a busy timeout, verifies that WAL mode actually took effect, and checks the file header against the file size before use
- **FTS5** — indexes label + note (not raw JSON), kept in sync via triggers
- **12 schema migrations** — V1 core, V2 access tracking, V3 indexes + edge dedup, V4 clean FTS, V5 search cache, V6 sync attribution/tombstones, V7-V8 documents and skills, V9 the action journal (`act_trace`, `probes`, `pathways`, `labile_window`, `corrections`), V10 `codec`/`delta`/`node_version`, V11 obligations, V12 readable obligation objects. Applied atomically in a single `BEGIN IMMEDIATE` transaction
- **Sync attribution** — `created_by`/`updated_by` stamped from the local identity config; deletes are soft (`deleted_at`) so they propagate as tombstones instead of resurrecting on the next sync
- **Batch BFS** — `WHERE id IN (...)` per level, not N+1 per node
- **Session dedup** — SHA-256 content hash on (project, summary)
- **Edge dedup** — UNIQUE constraint on (from_id, to_id, relation)
- **Task hub nodes** — tasks collect work logs, decisions, problems, solutions via `contains` edges
- **Problem lifecycle** — unsolved = no Solution node with `solves` edge
- **Relevance ranking** — FTS results boosted by access_count
- **Project hub nodes** — auto-created by `memory_session`, `task_create` and `memory_add(project=…)`, all children linked via `belongs_to`
- **Project scoping** — membership is the label prefix `[project-name]` **or** an edge to the project node, in either direction and under any relation. Checking only the label made everything written via `memory_add` + `memory_relate` invisible to project queries
- **Label convention** — child nodes prefixed `[project-name] description`, project nodes use plain names

### Node Types

`project` · `task` · `work_log` · `decision` · `concept` · `problem` · `solution` · `session` · `skill` · `user_fact` · `digest` · `crate` · `file` · `dependency` · `module` · `config` · `person` · `server` · `language`

### Relations

`belongs_to` · `contains` · `solves` · `subtask_of` · `blocks` · `depends_on` · `uses` · `caused_by` · `related_to` · `implements` · `configures` · `tracked_by` · `inspired_by` · `conflicts_with` · `supersedes` · `learned_from` · `imports` · `exports`

---

## Hooks (Auto-Capture)

`plugin/hooks.json` is the single source of truth for Claude Code hooks — shipped by the
`aurelius` Claude Code plugin (`claude plugin install aurelius@blysspeak`) instead of being
hand-edited into `settings.json`. Every hook calls the `au` binary directly — no bash, no python3.

| Event | Matcher | `au` command | Timeout |
|-------|---------|-----|---------|
| SessionStart | `""` | `au skills --hook` | 10s |
| SessionStart | `""` | `au snapshot --hook` | 10s |
| SessionStart | `""` | `au db backup --hook` | 30s |
| PostToolUse | `Edit\|Write` | `au touch --hook` | 5s |
| PostToolUse | `Bash\|PowerShell\|Edit\|Write\|NotebookEdit` | `au trace --hook` | 5s |
| Stop | `""` | `au reindex --hook` | 15s |
| Stop | `""` | `au judge --hook` | 20s |

`skills` injects the skill index, `snapshot` the seven-layer memory slice, `db backup` a throttled
rolling database snapshot, `touch` increments access_count on edited files, `trace` appends to the
action journal, `reindex` re-indexes the project and pushes sync-enabled projects, `judge` settles
the session (reinforce/erode/fork/null). A failing hook is always swallowed: memory has no right
to break the start — or the end — of a session.

The bash wrappers in `contrib/claude-code/*.sh` and its own `install.sh` are **deprecated since
3.4.0** — kept only for hand installs that never adopt the plugin, and removed in the next major
release. The `post-commit` git hook (`contrib/git-hooks/`) is a separate mechanism, unrelated to
the Claude Code plugin, and is still installed by `install.sh` for the current repo.

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
- [x] v1.9 — Documents to Markdown, converted locally (3 MCP tools + `au doc`)
- [x] v1.10 — Seven-layer snapshot; Bit-i-Delo stages 1-4: action journal, ground-truth probes, surprise gate, outcome judge, clearing and obligations
- [x] v1.11 — Project scoping by edge, not just by label prefix; `au snapshot --json`; `memory_add` warns on unattached nodes
- [ ] Next — npm distribution, `au repair`, `au doctor`, context-ranked search, git log connector

A release bumps `plugin/.claude-plugin/plugin.json` together with `Cargo.toml` — `crates/au/tests/plugin_manifest.rs`
asserts the two versions are equal, so `cargo test` stays red until both are updated.

---

## License

[MIT](LICENSE)
