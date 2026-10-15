# Task Management

[Back to README](../README.md)

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

## Contents

- [Status Lifecycle](#status-lifecycle)
- [Three Timestamps](#three-timestamps)
- [Branch and Worktree](#branch-and-worktree)
- [Evidence and Ripening](#evidence-and-ripening)
- [Closing with a Resolution](#closing-with-a-resolution)
- [Acceptance Criteria](#acceptance-criteria)
- [Task Leasing](#task-leasing-au-task-claim--renew--release--give-up)
- [Fitness Gate](#fitness-gate-au-task-fitness)
- [Integration](#integration)
- [Task Commands](#task-commands)

## Status Lifecycle

```
backlog → active → done
                 → blocked (with reason)
                 → cancelled
```

`task_log` never activates a task — recording work is an observation, not a decision to take it
into work. Activation is explicit: `task_update` with `status=active`, or `au task activate`.
`task_update` tracks timestamps automatically.
A project may hold several active tasks at once — activating one leaves the others as they
are. The "taken into work" time (`activated_at`) is stamped once, by whichever comes first:
activation, binding the task to a branch, or closing it. A run's evidence and a file edit go
to the active task bound to the branch they happen on; with no such task, to the one taken
into work last.

## Three Timestamps

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

## Branch and Worktree

A task taken into work remembers the git branch and worktree it is worked on. Activation reads
them from the repository of the current directory (only when that repository carries the task's
project name), and the edit hook then moves the binding to wherever files are actually edited —
a task is usually activated before its branch exists. Two things it never does: move a branch
you named yourself, and move from a feature branch back to the default one.

```bash
au task activate <id>                                  # branch read from HEAD
au task activate <id> --branch feat/x --worktree ~/wt  # named: the hook leaves it alone
```

`au task show` and `task_view` print the branch state against its upstream (`not pushed`,
`pushed, in sync`, `pushed, ahead 2`, `upstream gone`) and the branch's pull request as `gh`
reports it (`OPEN`, `MERGED`, `CLOSED`). `au task list` and `task_list` show the branch and its
local state only — no network. "Merged" comes from the pull request, never from commit
ancestry: a squash merge leaves the branch's commits outside the default branch.

## Evidence and Ripening

A verify run (`ulika`'s gate, e.g. `node scripts/verify-run.mjs …`) reports its outcome back
to Aurelius with `au task evidence` — command, exit code, and artifact path, attached to the
project's active task. **This is not a command you run by hand**: the ulika hook
(`record-verify.mjs`) calls it after every gate run, because it is the one thing that knows a
run just happened. You will see it in scripts and hook logs, not type it yourself.

The run is stored in the task itself (`data.evidence`: command, exit code, time, artifact path
and whether the artifact still exists) — no separate run node and no `verified_by` edge is
written; `au task show` and `task_view` list the runs. When the command runs inside a Claude
Code session that the plugin's SessionStart hook registered, the real exit code is also
recorded as a `verify` trace of that session for the judge.

A task **ripens** once it has at least one code edit *and* a green (exit-0) evidence run newer
than that edit — discussing a task without touching code never ripens it, and a later edit
un-ripens it again until a fresh green run covers it. `au task ripe` lists what's ripe along
with why: which evidence, when, what changed. `au judge --hook` (the Stop hook run at the end
of a turn) surfaces the same list unprompted, so a finished task gets presented for closing
without anyone having to ask — closing itself still requires a human decision (`au task done`).
`au task list` marks a ripe task inline too. Declining a proposal (`au task ripe --decline
<id>`) suppresses it on that task until new work lands.

## Closing with a Resolution

`au task done <id>` records *how* a task was closed, assembled from existing traces of work
rather than asked of the human:

```bash
au task done <id>                          # commit auto-detected via `git rev-parse --short HEAD` in the project's repo
au task done <id> --commit abc1234         # override the detected commit
au task done <id> --pr https://github.com/…/pull/42
au task done <id> --unconfirmed            # force "closed without confirmation", even if a commit was detected
```

The auto-detected commit is recorded together with its branch, and `au task done` prints a
notice on stderr naming both — close again with `--commit` if it is the wrong one. The project's repository is the path recorded on its
project node by `au reindex`; without one, the repository of the current directory, but only if
it is named like the project (a worktree counts under its main repository's name). A repository
of another name is never used, so a session sitting in one project cannot stamp another
project's task with its HEAD.

If nothing about the resolution can be determined and `--unconfirmed` isn't passed, the task
still closes — just marked **closed without confirmation**, so the history never silently
lies about how sure the record is. A task closed straight from the backlog, without ever
being taken into work (`activated_at` absent), gets no auto-collected files and no
auto-detected commit either — there is no work window to read either from, only an
explicit `--commit`/`--pr` is kept.

## Acceptance Criteria

Every task can have a Definition of Done checklist:

```bash
au task new "Implement auth" --project myapp --priority high \
  -c "JWT tokens work" \
  -c "Refresh flow tested" \
  -c "Rate limiting active"
```

## Task Leasing (`au task claim` / `renew` / `release` / `give-up`)

A task handed to a runner used to be indistinguishable from one still sitting in the
queue — two processes could grab the same one, and an abandoned one never came back.
`au task claim` leases one machine-fit task to an owner for a fixed number of minutes
and hands it to nobody else while that lease holds; `au task renew` extends the lease
while the runner is still alive; `au task release` records the outcome. The grant is a
single `UPDATE … RETURNING`, so two concurrent `claim` calls cannot land on the same
task — not "unlikely", but structurally impossible.

The queue hands out one work order per project at a time: if the project already has an
active task under someone else's live lease, `claim` declines, and the task it had just
taken is rolled back untouched. An active task without a lease — one a person took into
work — does not stand in the way and stays active.

```bash
au task claim --owner smena@host/123 --run 42 --lease-minutes 50   # take one machine-fit task
au task renew --id <id> --owner smena@host/123 --lease-minutes 50  # keep the lease alive
au task release --id <id> --owner smena@host/123 \
  --verdict done --evidence "cargo test — 186 passed"              # or --verdict failed
au task give-up --id <id> --owner smena@host/123 \
  --why "needs a human decision"                                   # blocks, does not requeue
```

### Release verdicts

`release --verdict done` closes the task through the same rule as `au task done` and the
MCP `task_update`: it stamps the close time and assembles the resolution from traces of
the work, and the `--evidence` text is kept as an ordinary run record, visible under
`au task show`. It does **not** yet judge the verdict it is handed — the runner's word
that the work is done is taken at face value. Gating the close on the run's exit code, on
a green check postdating the lease, and on every acceptance criterion having a recorded
check is designed but not implemented; until it is, that judgement lives in the driver
that calls these commands.

### Requeue, expiry and give-up

`--verdict failed` always requeues and starts a cooldown. A
lease that simply expires is picked up again by the next `claim` the same way, attempts
climbing each time; a task claimed three times without a `done` verdict drops out of
`claim`'s selection instead of cycling through the queue forever. `give-up` is the one
exit that does **not** requeue: the runner recognized the block needs a human, so the
task is left in place with a reason attached rather than retried.

### Dispatcher-only

These four are
**dispatcher-only** commands — a single external driver is meant to call them in a loop
against the whole queue; that outer loop is not shipped yet, only the primitives it will
call.

## Fitness Gate (`au task fitness`)

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

## Integration

- **`memory_status`** shows active/blocked tasks at session start
- **`memory_session`** accepts `tasks` parameter to link sessions to tasks, returns active tasks as hints
- **`memory_recall`** includes tasks in search results
- **`task_view`** aggregates the full work branch via BFS traversal
- **`au judge --hook`** surfaces ripe tasks unprompted at the end of a turn — see
  [Evidence and Ripening](#evidence-and-ripening)

## Task Commands

The canonical reference for every `au task` command; [CLI](cli.md) links here. The MCP side is in [MCP Tools](mcp-tools.md#task-management).

```bash
au task new "Title" -p myapp --priority high -c "Tests pass"
au task new "Title" -p myapp --due 2026-09-20 --remind-before 1d   # --due also creates an attached reminder (owner both)
au task list --project myapp --status active,blocked
au task show <id>                  # full details with work log branch and three timestamps
au task log <id> "Did X, Y, Z"    # record work (does not activate; see `au task activate`)
au task ripe [--project myapp]     # tasks ripe to close, with evidence + what changed
au task done <id>                  # mark complete; resolution assembled from traces
au task done <id> --commit <sha> --pr <url>   # override/add to the detected resolution
au task done <id> --unconfirmed    # force "closed without confirmation"
au task block <id> "waiting on API keys"
au task activate <id>              # resume blocked task; other active tasks stay active
au task update <id> --due 2026-09-25   # moves the due date; the attached reminder moves with it via snooze,
                                        # recorded in its journal instead of being silently overwritten

au task claim --owner … --run … --lease-minutes 50   # dispatcher-only: lease one machine-fit task
au task renew --id … --owner … --lease-minutes 50    # dispatcher-only: extend a held lease
au task release --id … --owner … --verdict done --evidence "…"  # dispatcher-only: report the outcome
au task give-up --id … --owner … --why "…"           # dispatcher-only: block, don't requeue
au task fitness --dry-run [--project myapp]           # is the open queue machine-checkable? read-only

# called by the ulika verify hook, not by hand:
au task evidence --project myapp --command "npm test" --exit 0 --artifact run.log
```
