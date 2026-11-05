# Write a session

Reach for this at the end of a session, or right before a compact, when you have a summary of what happened plus decisions and next steps. A standalone fact is `au note` instead (see `write-fact.md`).

Ready command (adjust the strings):

    au session -p myapp --json \
      "Fixed the backup verify hanging on bundles over 2 GB; switched to streaming git bundle verify" \
      -d "Verify bundles in chunks instead of loading the whole bundle into memory" \
      -n "Confirm the chunked verify also catches truncated bundles, not just oversized ones"

`-d` and `-n` are repeatable: one per decision, one per next step. Over MCP it is `memory_session(summary, project, decisions, next_steps, problems_solved, key_files, session_id, tasks)`.

Where each part lands in the snapshot (`au snapshot -p myapp`; sections are numbered by their position among the printed ones, so find them by name):
- The summary goes to "Recent sessions", project-scoped. Only the newest session record is shown there.
- Each `-d` becomes its own decision node, shown in "Decisions and knowledge", the same place `au note --type decision` writes to.
- Each `-n` is a tail for the next session. `au pickup -p myapp` (MCP `memory_pickup`) prints the freshest session tail that has next steps, and the snapshot's "Distillate" section carries it until something newer is recorded.
- The node kind is `episodic` (ages out) by default, unlike `au note`, whose default is `semantic`.

Traps:
- Do not write a session as `au note --type session`. It has no decisions or next steps, and as the newest `session` node it takes the "Recent sessions" slot and hides the real record.
- `-p` defaults to the current folder name, the same rule `au snapshot` follows. Wrong directory, wrong project.
- Without `--session <ID>` (or `AURELIUS_SESSION_ID`) a session-end hook cannot tell this run's record from a prior one in the same project. Without `--key`, a repeat is recognised as a duplicate only when the summary matches the previous one word for word; a hook that reports running totals should pass `--key <occasion>` so the repeat rewrites its own node.
- `--stdin` wants the whole record as JSON: `summary`, `decisions`, `next_steps`, `problems_solved` (a list of `{"problem": ..., "solution": ...}`), `key_files`, `project`, `session`. An unknown key is a hard error, not ignored.

Verify: `au snapshot -p myapp` shows the summary under "Recent sessions" and the `-d` decision under "Decisions and knowledge"; `au pickup -p myapp` shows the `-n` steps as the tail.
