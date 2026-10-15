# Hooks (Auto-Capture)

[Back to README](../README.md)

`plugin/hooks.json` is the single source of truth for Claude Code hooks — shipped by the
`aurelius` Claude Code plugin (`claude plugin install aurelius@blysspeak`) instead of being
hand-edited into `settings.json`. Every hook calls the `au` binary directly — no bash, no python3.

## Contents

- [Registered hooks](#registered-hooks)
- [What each hook does](#what-each-hook-does)
- [Hint on first edit](#hint-on-first-edit)
- [Hooks the plugin no longer registers](#hooks-the-plugin-no-longer-registers)
- [au remind --hook](#au-remind---hook)
- [Legacy wrappers and the git hook](#legacy-wrappers-and-the-git-hook)

## Registered hooks

| Event | Matcher | `au` command | Timeout |
|-------|---------|-----|---------|
| SessionStart | `""` | `au session-hook --hook` | 5s |
| SessionStart | `""` | `au skills --hook` | 10s |
| SessionStart | `""` | `au db backup --hook` | 30s |
| UserPromptSubmit | `""` | `au remind --hook` | 5s |
| PreToolUse | `Edit\|Write\|MultiEdit\|NotebookEdit` | `au hint --hook` | 5s |
| PostToolUse | `Bash\|PowerShell\|Edit\|Write\|NotebookEdit` | `au trace --hook` | 5s |
| Stop | `""` | `au judge --hook` | 20s |
| Stop | `""` | `au remind --hook` | 5s |

## What each hook does

`session-hook` records the Claude Code session id under the Claude process's pid, so anything
that process spawns (the MCP server, `au task evidence` run by a verify hook) can tell which
session it belongs to. `skills` injects the skill index, `db backup` a throttled rolling database
snapshot, `trace` appends to the action journal (secrets masked before the write), `judge`
settles the session (reinforce/erode/fork/null). A failing hook is always swallowed: memory has
no right to break the start — or the end — of a session. `remind` is wired to **both**
`UserPromptSubmit` and `Stop` on purpose, not by oversight: `Stop` delivers at the end of a turn,
`UserPromptSubmit` the moment the owner next types, and whichever fires first wins the
conditional `UPDATE` inside `reminders::mark_delivered` — the other finds the row already taken
and says nothing. Registering both costs nothing extra and halves the wait for a reminder that
matures mid-turn.

## Hint on first edit

`hint` brings memory in on an event rather than on every turn: on the **first** edit of a file in
a session it looks up at most three live decisions, problems and solutions that mention the
file's repository-relative path and hands them to the model as additional context. Later edits of
the same file in that session stay silent, files under `target/`, `node_modules/` and `.git/` are
skipped, and the edit is never blocked — any failure prints nothing.

## Hooks the plugin no longer registers

The plugin no longer registers `au touch --hook` or `au reindex --hook`: nothing indexes a
repository behind your back any more (see [Quick Start](../README.md#quick-start)). Both commands and their
`--hook` flags stay on the CLI. Sync push at the end of a session is `memory_session`'s job.

`au snapshot --hook`, which used to inject the memory slice at session start, was removed from the
manifest as well: the slice surfaces the *freshest* nodes, which is not the same as the *needed*
ones. The flag stays on the CLI, so a hand-written `settings.json` can still call it. Otherwise
pull memory in deliberately — `memory_status` at session start, `au pickup` after a context wipe.

## au remind --hook

Output format and delivery rules are in [Reminder Commands](reminders.md#reminder-commands). Like every hook mode here,
`--hook` is a plain CLI flag, so a hand-written `settings.json` can call it without the plugin.

## Legacy wrappers and the git hook

The bash wrappers in `contrib/claude-code/*.sh` and its own `install.sh` are **deprecated since
3.4.0** — kept only for hand installs that never adopt the plugin, and removed in the next major
release. The `post-commit` git hook (`contrib/git-hooks/`) is a separate mechanism, unrelated to
the Claude Code plugin, and opt-in: `install.sh` does not install it. It records every commit
message as an unverified decision node, which search then ranks beside real decisions — install it
in a repository only if you want that mirror:

```bash
cd /path/to/repo && /path/to/aurelius/contrib/git-hooks/install.sh
```
