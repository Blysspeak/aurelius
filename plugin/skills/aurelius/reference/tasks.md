# Work a task

Reach for this when moving a task through its life: take it, log work, attach a proof run, check whether it can close, then close or block it. Real commands, no `--help` lookups. MCP names are in brackets.

1. Create: `new`, never `create` (there is no such subcommand). [`task_create`]

       au task new "Move sessions to Redis" -p myapp --priority high \
         -c "load test passes" -c "no session lost on deploy" -d "workers need a shared store"

   `--due <date>` also creates an attached reminder. `au task update <ID> --priority|--title|-d|-c|--due` edits later; criteria are appended, never replaced.

2. Activate: a project may hold several active tasks at once; activating one leaves the others as they are. Activation records the git branch and worktree of the current directory (when that repository carries the project's name); `--branch` / `--worktree` name them explicitly. [`task_update` with `status=active`]

       au task activate <ID-or-label>

3. Log: a human note only. It neither activates nor ripens the task. [`task_log`]

       au task log <ID> "traced session reads, found 3 call sites in the auth middleware"

   Ripeness needs a recorded code edit, and `log` never records one. The edit comes from the plugin's PostToolUse hook (`au trace --hook`) on a real Edit/Write. It goes to an active task whose project is named like the edited repository's directory (a worktree counts under its main repository's name): the one bound to that branch, else the one taken into work last. No hook, no edit record.

4. Evidence: attach a verify run. Name the task, or name the project and it goes to that project's active task.

       au task evidence <ID> --command "cargo test -p myapp" --exit 0
       au task evidence --project myapp --command "cargo test -p myapp" --exit 0

   Meant for a verify hook to call right after a gate run, but calling it yourself after a green run works the same. It exits 12 when the project has no active task. Optional: `--artifact <path>`, `--subject`, `--json`.

5. Ripe: can it close? `au task ripe -p myapp` [`task_ripe`]. It fires only when all hold: status active; at least one recorded edit; an evidence run with exit code 0 timestamped after the last edit; no `--decline` newer than that edit. Evidence right after a `log` is not enough, and a later edit un-ripens the task until a fresh green run covers it. `au judge --hook` (the Stop hook) shows the same list unprompted.

6. Close or block. [`task_update` with `status=done` / `blocked`]

       au task done <ID>                      # commit detected from the project's repo
       au task done <ID> --commit $(git rev-parse --short HEAD) --pr https://github.com/org/myapp/pull/42
       au task block <ID> "waiting on the CDN API key"

   Without `--commit`, `done` takes the commit from `git rev-parse --short HEAD` in the project's repository (the one recorded by `au reindex`, else the current directory when it is named like the project) and prints the commit and branch on stderr; close again with `--commit` if it is the wrong one. With nothing to determine it still closes, marked "closed without confirmation"; pass `--unconfirmed` to say so on purpose.

Also useful: `au task list -p myapp -s active,backlog` (`--json` for programs), `au task show <ID>` (three timestamps, branch state, evidence, work log), `au task criterion <ID>` lists acceptance criteria with their handles and `--met <handle>` / `--unmet <handle>` marks one. A met mark records progress only; neither `ripe` nor `done` reads it.

Do NOT:
- expect `task log` to satisfy ripeness;
- treat `--decline` as permanent: it holds only until the next edit;
- call `au task claim|renew|release|give-up|fitness`: they belong to an external dispatcher that leases work orders, not to a normal session.
