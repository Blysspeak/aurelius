# Quickstart: Database Durability & Integrity Hardening

**Feature**: 002-db-durability-hardening

Two audiences: the user who wants to protect their knowledge graph, and the developer
who has to prove this change works.

---

## For the user

### Take a backup

```bash
au db backup
```

Writes `aurelius-<UTC timestamp>.db` next to the database and prints the path. Safe to
run while Claude Code sessions, MCP servers and the graph viewer are all running.

To choose the destination:

```bash
au db backup --out /mnt/backup/aurelius-before-upgrade.db
```

### Verify it

```bash
au db check
```

Exit code 0 means healthy, non-zero means damaged — so it works in a script or a hook:

```bash
au db backup && au db check || echo "backup is not trustworthy"
```

For the exhaustive report:

```bash
au db check --full
```

### The one rule

> **Never copy, move or restore `aurelius.db` with `cp`, `mv`, `rsync`, a file manager,
> or a backup agent while `au` or an MCP server is running.**

In WAL mode, cross-process cache coherency runs through the `-shm` WAL-index rather
than through the database header. Replacing the file underneath open connections lets a
live process keep flushing its cached pages into the new file — which is how a database
ends up with a header describing 181 pages while its body holds 1781. Use
`au db backup`; it is the only safe way to copy a live database.

### Restoring a backup (manual, deliberately not a command)

1. Stop **everything** that touches the database — every `au mcp` process, `au view`,
   any editor with hooks configured. Verify none remain.
2. Move the current database aside (do not delete it) together with any `-wal` / `-shm`
   siblings.
3. Copy the backup into place.
4. `au db check`.
5. Restart your sessions.

Step 1 is the dangerous one and the tool cannot enforce it for processes it did not
start, which is exactly why this is a documented procedure rather than a command that
would imply a safety it cannot provide.

---

## For the developer: verifying this change

### Gates (constitution: Quality Gates)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
```

`cargo test --workspace` runs the first tests this repository has ever had. Each one
must fail against the pre-change code — that asymmetry is the evidence required by
Principle V, not a formality.

| Test | Asserts | Fails before the change because |
|---|---|---|
| `fresh_open_migrates_and_is_idempotent` | after `open`, version == `SCHEMA_VERSION`, `nodes_fts` exists; a second `open` changes nothing | (passes — regression guard) |
| `failed_migration_rolls_back_migrate_v4` | a failure after v4 leaves version 3 **and** leaves v4's re-index undone | v4 auto-commits; version reaches 4 and the index is rebuilt |
| `corrupt_header_is_detected_at_open` | `open` returns `DbError::Corrupt`; `check` flags the size discrepancy | the old code opens the file and keeps writing |
| `concurrent_opens_all_succeed` | 8 threads opening simultaneously all succeed | no busy timeout → `SQLITE_BUSY`, plus the v4 race |
| `schema_newer_than_binary_is_rejected` | a version-99 database yields `SchemaTooNew` | the old code opens it silently |
| `backup_captures_uncheckpointed_wal` | a snapshot taken with a live writer contains its rows | `backup_into` does not exist |

### End-to-end run against the real database

```bash
./target/release/au db check
./target/release/au db backup
./target/release/au db check --full
```

Then confirm the ordinary paths still work — the point of the change is that nothing
else moves:

```bash
./target/release/au search aurelius
./target/release/au task list
```

### The decisive check — replay the incident

The damaged file from 2026-07-27 is preserved next to the database as
`aurelius.db.CORRUPT-<timestamp>`. Point the new binary at a **copy** of it and confirm
three things:

1. `au db check` exits non-zero;
2. its output contains both an engine finding and the derived line
   `file is 7294976 bytes but the header describes only 181 pages of 4096 …
   this is the signature of a file-level copy over a live WAL database`;
3. the file's hash is **identical** before and after — ten consecutive ordinary
   operations must not change a single byte (spec SC-001).

```bash
cp "$APPDATA/aurelius/aurelius.db.CORRUPT-"* /tmp/incident.db
sha256sum /tmp/incident.db > /tmp/before.txt
for i in $(seq 1 10); do ./target/release/au db check || true; done
sha256sum -c /tmp/before.txt
```

This is what makes the claim "the failure class is closed" verifiable rather than
asserted: the exact file that broke the system is now refused, explained, and left
untouched.

### Before tagging a release

Resolve the discrepancy recorded in the plan's Complexity Tracking table: the installed
`au.exe` reports **1.6.0** and exposes `au skills` plus four `skill_*` MCP tools that
exist in no commit. Reinstalling from this tree would delete a command a live
`SessionStart` hook depends on. Decide first — restore the source, or retire the hook
and the burnt version number — then release.
