# What not to write

Reach for this before any write (`au note`, `memory_add`, `au task new`, `task_create`) to decide whether the fact belongs in memory at all, not how to phrase it.

RULE: only what cannot be recovered goes in. Recoverable from code, git or command output: cite where to find it, do not store it. Not recoverable anywhere: always write it. That is why an option was picked, what was rejected and why, how a trap looked from outside, a measured value.

NEVER WRITE:
- File contents, signatures, config values. `grep` or `git show <path>` recovers them.
- Command output or telemetry (build logs, test runs, benchmarks). Store the command as `--evidence`, not a paste of what it printed.
- Narrative retelling ("read the file, then checked X, then found Y"). The graph is not a session diary.
- A claim with no rejected alternative. "Used --confidence" alone is a smear, indistinguishable from restating the code.
- Secret values: tokens, passwords, keys, connection strings. Notes are stored verbatim, backups included. Store where the secret lives instead: `au secret add --name STRIPE_KEY --where "env:STRIPE_KEY" --purpose "charge webhooks" --project myapp`.

SMEAR vs SHARP, same kind of fact:

    SMEAR: "au task has no create subcommand, only new."
      Do not write this at all: one `--help` call recovers it forever.
    SMEAR: "Tuned the connection pool."
    SHARP:
      au note --project myapp --type decision --confidence measured --volatility slow \
        --subject "myapp:db:pool-size" \
        --claim "Connection pool is 20; 50 made checkout p95 worse (310 ms vs 180 ms) on the 4-core staging box." \
        --evidence "k6 run load/checkout.js --env POOL=20, then --env POOL=50" \
        "Rejected: pool 50. More connections than cores only added lock contention in Postgres."

TRAP: no `--confidence` reads as `unverified`, never "probably measured". Set it explicitly or skip the write. Over MCP, `memory_add` refuses a call without `confidence`.

TRAP: `--resolution` (supersede / refine / coexist) only records lineage. The superseded claim still prints next to the new one in the snapshot's "Decisions and knowledge" section. To get it out of the snapshot rather than merely outranked, use `au merge <SOURCE> <TARGET>` (MCP `memory_merge`, an admin tool): it rewires the source's edges onto the target and deletes the source.

Verify: `au snapshot -p myapp`. A fresh `measured` claim prints with no `[mark]` suffix; if every line carries a bracket tag (`[unverified]`, `[inferred]`), `--confidence` was skipped at write time across the board.
