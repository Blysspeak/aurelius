# Agent checkpoint

For a delegated agent that just finished an expensive or irreversible step (a production query, a long run, a measurement) or is about to return findings to its caller: checkpoint into aurelius now, not only at the end.

Checkpoint incrementally. Write an `au note` right after each expensive step, while it is still in front of you, not saved up for the final report. The process can die between here and your final message (a `/clear`, a crash, a kill, a dropped connection), and only what is already written survives. The write right before you return still happens, as a backstop for whatever changed since the last checkpoint, not as the only write.

## The four things a checkpoint must carry

A finding alone is not a checkpoint. The reader of your note is a parent process whose context was wiped while you worked: it no longer remembers what it asked you or why. A note that carries only the finding leaves that reader holding a fact it cannot place, and it will re-order the same work.

1. **Asked**: the assignment in one phrase, as given to you.
2. **Why**: what the caller intended to DO with the result. Not "for context": the decision, edit or gate that was waiting on it.
3. **Found**: the finding itself, with the identifier or number that makes it checkable.
4. **Next**: what should happen now: the follow-up call, the file to edit, or "nothing, this closes it".

Items 1, 2 and 4 go in the positional TEXT. Item 3 goes in `--claim`, because `--claim` is what a later ranked recall shows first.

## Copyable line, every provenance flag filled in

    au note --project myapp --type concept --confidence measured --volatility slow \
      --subject "myapp:scripts/export.sh:exit-code" \
      --label "export.sh exits 0 on a failed upload" \
      --claim "scripts/export.sh exits 0 even when the upload step fails; the only signal is the 'upload: FAILED' line in its log" \
      --evidence "bash scripts/export.sh; echo exit=\$?  - run r-0142, agent checkpoint-writer" \
      "export.sh exits 0 on a failed upload. ASKED: find out why green export runs left the bucket empty. WHY: the caller was about to wrap the job in a retry on non-zero exit and needed to know whether the job or the upload was failing. FOUND: see claim. NEXT: do not retry on the exit code; read the log line instead."

`--label` is what search and the snapshot show first, so it names the finding in a few words. Leave it out and the label is the first 60 characters of TEXT, which is why TEXT opens with the finding, never with `ASKED:`. With TEXT omitted the label is the claim. TEXT itself is optional when `--claim` is set (`au note --claim "..."` alone is a valid call), but prefer to pass it: that is where ASKED / WHY / NEXT live, and a claim by itself carries only FOUND.

`--type`: `decision` | `concept` | `problem` | `solution` | `file` (the full list is in `au note --help`).
`--volatility`: `immutable` (never changes) | `slow` (changes across releases) | `volatile` (changes within days).

`--confidence`, pick honestly:
- `measured`: you ran it and saw the result yourself. Needs `--evidence`.
- `inferred`: you concluded it from something you read.
- `reported`: someone told you (the caller, a doc, a teammate).
- flag omitted reads as `unverified`, never as "probably measured".

Restored-snapshot ceiling: knowledge you hold only because a context restore handed it back (`au pickup`, `au snapshot`, `au context`, or their MCP twins) is at most `inferred`, never `measured` or `reported`. A snapshot is a retelling of a past session, not something this run observed. `--evidence` must name where it came from, e.g. `--evidence "au pickup -p myapp, after /clear"`, so a later reader can see the claim traces back to a retelling and not to this run's own eyes.

`--subject` is the lookup key a later search hits: a stable identifier such as `project:path/to/file`, `project:area:setting`, or a named concept. Never a sentence. A second note on the same subject is REFUSED until you add `--resolution`:
- `supersede`: the old fact is now wrong.
- `refine`: the old fact holds, this sharpens it.
- `coexist`: both are true under different conditions.
None of the three hides the older node; they build a lineage.

`--evidence` carries the run id of the order that produced it and your agent label next to the verbatim command, so the record traces back to its origin.

One record per unit of knowledge, not one per task. Three findings means three notes, each with its own subject, each written when that finding was established, not batched for later.

Do NOT write what is recoverable: file contents, diffs, git history, command output, the list of files you touched. Reference those by path, hash or command instead. Write only what nothing can reconstruct: why this option and not the one you rejected, what the trap looked like from the outside, the value you tuned by experiment. Never write secret values.

If the `au` CLI is unavailable, use `mcp__aurelius__memory_add` with the same fields (`label`, `note`, `claim`, `evidence`, `confidence`, `volatility`, `subject`, `resolution`, `project`, `type`).
