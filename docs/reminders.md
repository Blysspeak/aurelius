# Reminders

[Back to README](../README.md)

Reminders set on purpose, delivered either by one always-on daemon or at the next turn of a session.

## Contents

- [Model](#model)
- [MCP tools](#mcp-tools)
- [The daemon](#the-daemon)
- [Reminder Commands](#reminder-commands)

## Model

A reminder is a moment set on purpose, with an explicit `state` (`pending` → `delivered` → `done`/`cancelled`, where `delivered` is not an outcome — a promise shown to someone and not yet resolved either way) and an auditable trail of every postponement. It has an addressee (`owner`: `me` / `ai` / `both`) that decides which channel may deliver it. **No timer lives in the MCP server or in these tools** — every one of them is a single request/response read or write over the same `reminders`/`reminder_events` tables `au remind` uses; the clock belongs to one always-on daemon (see [The daemon](#the-daemon)) precisely because the MCP server itself is respawned once per Claude Code session, and a scheduler living inside it would multiply by the number of open sessions.

## MCP tools

`reminder_add`, `reminder_list`, `reminder_show`, `reminder_done` and `reminder_snooze` are described in [MCP Tools](mcp-tools.md#reminders).

## The daemon

See [Model](#model) for the
state machine (`pending` → `delivered` → `done`/`cancelled`) and the journal. `au remind` and the
MCP `reminder_*` tools only ever read or move a row; the clock itself belongs to `au daemon`, a
single always-on process installed as a systemd **user** unit
(`contrib/systemd/aurelius-remind.service`):

```bash
mkdir -p ~/.config/systemd/user && cp contrib/systemd/aurelius-remind.service ~/.config/systemd/user/
systemctl --user enable --now aurelius-remind.service
```

Each tick reads `reminders::overdue_undelivered` for `owner: me`/`both` that outlived `--grace`
past their due moment (default `15m`) — `owner: ai` belongs to a session and the daemon never
touches it — and hands each one to `$AURELIUS_NOTIFY_CMD` (run through `sh -c`, reminder text
passed as the `AURELIUS_REMINDER_TEXT` environment variable, never interpolated into the command
string itself) or, absent that, to `notify-send` if it's on `PATH`; a reminder is stamped
delivered only once that channel exits 0, so a failed notification is retried on the next tick
instead of being marked and lost. `--interval` (default `60s`) sets the pause between ticks,
`--once` runs a single tick and exits (how the daemon is tested without sleeping, and how a
systemd timer could stand in for the long-lived service). A file lock next to the database, not
the unit being started once, is what keeps the daemon a singleton: a second `au daemon` — a stray
manual start over an already-enabled unit, a double `enable` after copying the file twice — finds
a live PID in the lock, refuses with its own exit code (`15`), and leaves the database untouched.
Outside `--once`, the same daemon also owns the embedding models and serves `embed.sock` — see
[Search](memory.md#embedding-daemon).

## Reminder Commands

Hook wiring for `au remind --hook` is listed in [Hooks](hooks.md#registered-hooks).

```bash
au remind add "check the deploy" --in 2h --for ai          # --for: me | ai | both (default both)
au remind add "call the client" --at "2030-01-01 09:00" --for me
au remind add "standup" --in 30m --task <task-id-or-label> --repeat 1d  # attaches to a task, inherits
                                                                         # its project, re-arms daily
au remind list [--project myapp] [--state pending] [--all]   # --all also shows done/cancelled
au remind show <id>                # the reminder plus its full journal — every postponement, oldest first
au remind done <id>
au remind cancel <id>
au remind snooze <id> --in 1h      # or --at <moment> — exactly one, same rule as `add`
au remind --hook                   # session-side consumer, wired to Stop/UserPromptSubmit (see Hooks):
                                    # reminders addressed to ai/both, marked delivered BEFORE printing so a
                                    # crash between the two can't double-deliver one. UserPromptSubmit prints
                                    # plain text to stdout; Stop (or anything else) prints {"systemMessage": …}.
                                    # Silent, exit 0, when nothing is due — a hook that talks every turn stops
                                    # being read.
```

A reminder addressed to `ai`/`both` therefore arrives at the next turn boundary, not at the exact
wall-clock second — the session-side hook only gets to look at the table when the session is
already talking. A reminder that must land at an exact moment regardless of whether a session
happens to be open needs `owner me`/`both`: only the daemon's out-of-session channel is independent
of a turn in progress, which is also why the MCP server and `au remind` are deliberately not allowed
to run a timer of their own (see [Model](#model)).
