# Write a fact

Reach for this when something worth keeping just happened: a decision made, a bug's root cause found, a fix that worked, a config value found by experiment. `au note` (MCP `memory_add`) writes a lasting fact; `au session` (MCP `memory_session`) writes a session summary. Do not use one for the other (see `sessions.md`).

Ready command:

    au note --project myapp --type solution --confidence measured --volatility slow \
      --subject "myapp:deploy:bundle-verify" \
      --claim "Verify git bundles after every transfer: an scp copy failed 'git bundle verify', an rsync -a copy of the same file passed" \
      --evidence "git bundle verify myapp-full.bundle" \
      --json \
      "bundle verify failed with 'not a bundle' after scp; re-copied with rsync -a instead of scp, verify passed clean"

Over MCP the same record is `memory_add` with `label` (required), `confidence` (required), `project`, `type`, `note` (the long text), `claim`, `evidence`, `volatility`, `subject`, `resolution`, `session_id`. Omitting `project` leaves the node unattached: it appears in no project-scoped query, and the answer carries an `attachment_warning`. Link related nodes with `memory_relate` / `au relate <from> <to> --type solves`.

## Field cheat, only the parts that bite

- Positional TEXT is the long story: the why, the rejected alternative. It is optional when `--claim` is set, but pass it anyway. The label comes from the first 60 characters of TEXT (so open TEXT with the finding), or from `--label`; with TEXT omitted the label is the claim.
- `--claim` is one line, up to 240 characters, returned whole by search. A longer one is shortened at write time and the original is kept.
- `--confidence` omitted reads as `unverified`, not "probably measured"; the snapshot tags the line `[unverified]`. Always set it. `measured` without `--evidence` is refused (exit 1, nothing written): drop to `inferred` or add the command.
- `--subject` is a key for what is asserted, scoped as `project:area:thing`. A second `au note` on the same subject is REFUSED (exit 1, nothing written) until you add `--resolution supersede|refine|coexist`. None of the three hides the older fact; all keep it visible and add lineage.
- `--evidence` is the exact command that produced the claim, verbatim, not a description of it.
- `--volatility`: `immutable` (a commit sha), `slow` (a schema; stale after 30 days), `volatile` (an `.env` value; stale after 1 day). A stale fact comes back marked "recheck"; `--verify-with` names the command that rechecks it.
- `--key <KEY>` makes a hook-fired note idempotent: the same key rewrites the node instead of creating a twin. Use it for anything a hook might fire twice on. A repeat keeps the node's type unless `--type` is passed.
- `--session <ID>` (or `AURELIUS_SESSION_ID`) stamps the run, so `au journal --session <ID>` lists what this run wrote.
- `--stdin` reads the text from stdin, the way past a command-line length limit.
- `--type` decides where the fact shows up. `decision` and `concept` print in the snapshot's "Decisions and knowledge"; `problem` in "In progress"; every other type (`solution`, `file`, `server`, `dependency`, ...) is not in the snapshot at all and is reached only through search or recall.
- `--kind episodic` is for something tied to a moment that should age out; the default `semantic` is a lasting fact.
