# Phase 0 Research: Database Durability & Integrity Hardening

**Feature**: 002-db-durability-hardening
**Date**: 2026-07-28
**Engine under test**: SQLite 3.45.0 as bundled by `libsqlite3-sys` 0.28 via `rusqlite` 0.31

Everything below was **executed**, not inferred from documentation. Where a number
appears, it was measured.

---

## R1. What the code can and cannot prevent

**Decision**: Scope the feature as *prevent recurrence of the trigger* + *prevent
compounding* + *detect immediately* — not as "make file swapping safe".

**Rationale**: The root cause was an operator-level file copy over the live database.
A repo-wide search confirms the product performs **no** filesystem operations on the
database file at all — no `fs::copy`, `fs::rename`, `fs::remove_file`, `set_len` or
`File::create` anywhere in `crates/`. The only `std::fs` calls near the database are
two `create_dir_all` on the parent directory. `install.sh` touches only binaries, hook
scripts and `~/.claude/settings.json`. So there is no code path to fix that caused the
swap; the code's responsibility is the surrounding three layers.

| Layer | Mechanism chosen |
|---|---|
| Prevent recurrence of the trigger | `au db backup` (`VACUUM INTO`) as the supported alternative, plus "never `cp`" stated in docs *and* in the corruption error text |
| Prevent compounding | `busy_timeout`, transactional migrations, refuse to hand out a connection to a damaged file |
| Detect immediately | `PRAGMA quick_check(1)` on every `db::open`, plus `au db check` |

**Alternatives considered**: a lock file or PID registry to detect "another writer is
live" and refuse to start. Rejected — it cannot stop `cp`, which does not consult it,
and it introduces a new stale-lock failure mode.

---

## R2. `PRAGMA journal_mode=WAL` silently failing

**Finding**: `PRAGMA journal_mode=WAL` returns the **resulting** mode as a result row
and does **not** raise an error when the switch is refused (for example, when another
connection holds a lock during the conversion). It simply returns the old mode.

`rusqlite` 0.31's `execute_batch` steps the statement and discards the row:

```rust
// rusqlite-0.31.0/src/lib.rs
if !stmt.stmt.is_null() && stmt.step()? && cfg!(feature = "extra_check") {
    return Err(Error::ExecuteReturnedResults);
}
```

`extra_check` is not enabled in this workspace, so the returned mode is invisible to
the caller. The current code therefore cannot tell whether it is running in WAL mode or
in rollback-journal mode.

**Decision**: issue the pragma with `query_row` and compare the returned string against
`"wal"` case-insensitively; return `DbError::JournalMode(actual)` on mismatch.

**Aggravating detail**: the current code issues this pragma *before* any busy handler
exists, which is exactly when a refusal is most likely. Hence R3's ordering.

---

## R3. Pragma order

**Decision**:

1. `busy_timeout` — **first**, before anything that can take a lock.
2. `journal_mode=WAL` — via `query_row`, result asserted.
3. `synchronous=FULL` + `foreign_keys=ON`.
4. health gate (`verify`).
5. `migrate`.

**Rationale**: steps 2–5 can all block on a lock. With the default busy handler (none),
any contention fails instantly. The deployment guarantees contention: a `PostToolUse`
hook spawns `au touch` on **every** Edit/Write
([contrib/claude-code/aurelius-track-edit.sh:28](../../contrib/claude-code/aurelius-track-edit.sh)),
a `Stop` hook runs `au reindex`
([contrib/claude-code/aurelius-reindex.sh:14](../../contrib/claude-code/aurelius-reindex.sh)),
a git `post-commit` hook runs `au note`, the viewer opens a connection per HTTP request
([crates/au/src/view.rs](../../crates/au/src/view.rs)), and several `au mcp` servers run
concurrently.

**Timeout value**: 10 s. Long enough to absorb a checkpoint or a migration by another
process; short enough that a genuinely stuck lock surfaces rather than hanging a hook.

**Alternatives considered and rejected**:

- `synchronous=NORMAL` (suggested during audit) — measurably faster in WAL mode but a
  **reduction** in durability. Prohibited by the spec (FR-017) and by Principle I.
- `wal_autocheckpoint=1000` — that is already the default; a no-op.
- `trusted_schema=OFF` — the schema's triggers call no functions; speculative.

**Durability note**: bundled SQLite 3.45 does not define
`SQLITE_DEFAULT_WAL_SYNCHRONOUS` (checked in `libsqlite3-sys-0.28`'s `build.rs`: the
only `SQLITE_DEFAULT_*` set is `SQLITE_DEFAULT_FOREIGN_KEYS=1`), so the effective mode
today is already `FULL`. Setting it explicitly changes nothing about behaviour and
removes the dependency on build flags.

---

## R4. Detecting the incident's signature

**Finding — `quick_check` catches it**: a database file was taken at 266 pages and its
header page count patched to 26 (the same shape as the incident: header describes less
than the file holds). Result:

```
quick_check: '*** in database main ***
              Tree 2 page 2 cell 0: invalid page number 266'
count(*)   : database disk image is malformed
```

`quick_check` reports the problem **before** any write reaches the file — which is the
whole point of the gate.

**Finding — cost**: on a 2.2 MB database (3 000 nodes + fts5),
`PRAGMA quick_check(1)` takes **2.2 ms warm / 4.5 ms cold**; full `integrity_check`
takes 2.7 ms. Projected to the 7 MB production database: ~10 ms. Process startup for
`au touch` is an order of magnitude more.

**Initial decision — REVISED during implementation, see R4a**: run `quick_check(1)` on
every `db::open`.

**Memoisation explicitly rejected**: caching "this file was verified at startup" in a
`OnceLock` would make the check free for long-lived processes — and would miss exactly
the incident, in which the file was swapped *in the middle of a long-lived MCP
process's life*. Per-open checking is both simpler and strictly stronger. This part of
the decision survived the revision.

**Second detector — header geometry**: `quick_check` describes symptoms
("invalid page number 266") but never names the cause. The signature
`file_bytes > page_size × page_count` is unambiguous and worth reporting in plain
words. Verified: for a healthy WAL database the *inverse* can legitimately occur —
`page_size × page_count` (the logical size seen through the WAL) can exceed the main
file (measured: main file 4 096 B while `page_count × page_size` = 1 089 536 B). The
direction used as a corruption signal — file **larger** than its own header describes —
never occurs legitimately. The incident file: 7 294 976 B against 181 × 4 096 =
741 376 B.

The converse (file smaller than the header describes) is only flagged when there is no
`-wal` to account for the difference.

---

## R4a. Why the gate is geometry-only (revision, found by the tests)

The `quick_check` gate from R4 was implemented, and two of the six tests failed against
it. Both failures were real defects in the design, not in the tests:

```
concurrent_opens_all_succeed:
  Corrupt { detail: "unable to validate the inverted index for FTS5 table
            main.nodes_fts: database is locked" }

backup_captures_uncheckpointed_wal:
  problems: ["unable to validate the inverted index for FTS5 table main.search_fts:
             attempt to write a readonly database", ...]
```

**Cause**: SQLite 3.45 added `xIntegrity` to FTS5, so `quick_check` and
`integrity_check` now also validate the FTS5 inverted indexes — and that validation
needs write access and a lock. Consequences, both observed:

- under this project's own concurrency the gate reports "database is locked" on a
  perfectly **healthy** database;
- on a **read-only** connection it reports "attempt to write a readonly database" for
  every healthy database, which would make `au db check` fail on everything.

A gate that refuses healthy databases is worse than no gate, so:

**Revised decision — the `open()` gate reads the geometry straight out of the 100-byte
header and runs no SQL at all.** It takes no lock, touches no table, cannot fail on a
healthy database, and — crucially — still works on a file the engine refuses to open,
which is exactly when it matters. Nuance implemented: the in-header page count is
authoritative only when the change counter (offset 24) equals the version-valid-for
marker (offset 92); otherwise SQLite derives the size from the file and the comparison
would be noise. In the incident file both were 39, so the check fires.

Structural damage that leaves the geometry consistent is still caught, because every
error on the open path now goes through `classify`, which maps `SQLITE_CORRUPT` /
`SQLITE_NOTADB` to the actionable `DbError::Corrupt`. The first thing `open` does after
the gate is read `sqlite_master`, so such damage surfaces immediately anyway.

**Revised decision for `au db check`**: run the check **per ordinary table**
(`PRAGMA quick_check('nodes')`, …) rather than whole-database, skipping virtual tables.
FTS5 shadow tables are ordinary tables and are still checked, so page-level b-tree
integrity is covered without ever entering the fts5 `xIntegrity` path.

**Verified on the incident file itself** — `au db check` against a copy now prints the
geometry line first and then the engine's own findings per table, and exits 1:

```
file is 7294976 bytes but the header describes only 181 pages of 4096 (741376 bytes)
— 6553600 bytes lie past the end of the declared database; this is the signature of a
file-level copy over a live WAL database
edges: *** in database main ***  Tree 5 page 123: btreeInitPage() returns error code 11
nodes: *** in database main ***  Tree 3 page 134 cell 1: invalid page number 167772160
```

And 22 consecutive operations against that file changed **zero bytes** (SHA-256
identical before and after), which is spec SC-001.

**Cost of the revised gate**: two `stat` calls and a 100-byte read. Immeasurable next
to process startup, and strictly cheaper than the 2.2–4.5 ms measured for
`quick_check(1)`.

---

## R5. Safe snapshot: `VACUUM INTO` vs the Online Backup API

**Decision**: `VACUUM INTO`.

| | `VACUUM INTO` | `rusqlite::backup` |
|---|---|---|
| Crate feature needed | none | `features = ["backup"]` — a new build dependency |
| Code | one SQL statement | `step(n)` loop plus `Busy`/`Locked` handling |
| Behaviour when the source is written during the copy | single read transaction — consistent snapshot | the backup **restarts from scratch** |
| Result | defragmented copy | byte copy, free pages and all |
| On a damaged source | fails → "a successful backup is a readable backup" | may faithfully copy the corruption |

**Verified empirically**: `VACUUM INTO` runs from a **read-only** connection
(`file:db?mode=ro`), accepts a **bind parameter** (`VACUUM INTO ?1`), and **captures
rows still in an un-checkpointed `-wal`** (1 000 rows written and left in the WAL → 1 000
rows present in the copy). Requires SQLite ≥ 3.27; bundled is 3.45.0.

**Side effect worth documenting**: opening a WAL database read-only creates empty
`-wal`/`-shm` siblings. No page of the main file is written, and a read-only connection
cannot checkpoint.

---

## R6. Transactional migrations in rusqlite 0.31

**Finding**: `Transaction::new_unchecked(&Connection, TransactionBehavior)` exists
(`rusqlite-0.31.0/src/transaction.rs:116`) and takes `&Connection`. This matters
because it lets `migrate(conn: &Connection)` keep its signature, and `&tx` reaches the
existing `fn migrate_vN(conn: &Connection)` bodies through `Deref` coercion — so
**none of the migration bodies need to be edited**.

**Decision**: two paths.

- **Fast path**: read the version *outside* any transaction; if it already equals
  `SCHEMA_VERSION`, return immediately. No write lock, no DDL, not even
  `CREATE TABLE IF NOT EXISTS`. This is what keeps per-tool-call and per-HTTP-request
  opens cheap.
- **Slow path**: `BEGIN IMMEDIATE` (takes the write lock at once rather than upgrading
  mid-way and risking `SQLITE_BUSY`), re-read the version **inside** the transaction,
  apply the steps, bump the version, commit.

**Why this removes v4's destructiveness without touching v4**: `migrate_v4` drops and
rebuilds the FTS table and its triggers. Under the new structure:

- the version is read inside a write transaction, so two processes cannot both observe
  `current = 3`;
- the body and the version bump commit atomically, so the state "FTS dropped, triggers
  gone, version still 3" becomes unrepresentable;
- `unwrap_or(0)` no longer converts BUSY/CORRUPT into "fresh database", so v4 can never
  re-run over live data.

Adding `IF NOT EXISTS` guards inside v4 would be extra code for a problem the structure
already eliminates — rejected under Principle IV.

**Detail**: an early `return` (e.g. `SchemaTooNew`) drops the `Transaction`, whose
default `DropBehavior` is rollback. The pragmas must be set *before* the transaction —
inside one they are a no-op or an error.

**`set_schema_version`**: switch to `INSERT OR IGNORE`. `version` is
`INTEGER PRIMARY KEY`, so a plain `INSERT` conflicts on any re-run; one word removes the
whole class.

---

## R7. Replacing error-text matching

**Finding**: `pragma_table_info(?1)` is a table-valued function and accepts a bind
parameter — verified against the live engine. `sqlite_master` (not `sqlite_schema`) is
used for object existence, since it works on every SQLite build.

**Decision**: `migrate_v2` checks each column with
`SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2` instead of running the `ALTER
TABLE` and swallowing errors whose message contains `"duplicate column"`. Matching
English error text is prohibited by Principle III and breaks silently if the engine
rewords the message.

---

## R8. Schema version storage stays as-is

**Decision**: keep the `schema_version` table; do **not** move to `PRAGMA user_version`.

**Rationale**: `user_version` is a single integer in the header and would be marginally
cleaner, but migrating to it is itself a schema migration, and a database written by
the new binary would then look like version 0 to 1.5.0 — which would re-run every
migration, including destructive v4. The constraint "readable by the previous version"
(spec, Assumptions) rules it out.

---

## R9. `au db repair` — dropped from scope

**Finding**: recovering the incident file required a raw page scan that reconstructs
records from table b-tree leaf pages while ignoring the damaged interior structure —
i.e. what the `sqlite3` shell's `.recover` does via the `sqlite3_recover` extension.
**`rusqlite` exposes no binding for it.**

**Decision**: do not ship `au db repair` in this feature. A repair command that
silently under-recovers is worse than no command, because the user would treat its
output as complete. The remedy shipped instead: refuse, report, and let the user
snapshot whatever is still readable. The roadmap line about `au repair` stays.

**Actual recovery performed for this incident** (out of band, retained for the record):
a Python page-level scanner extracted 3 012 nodes and 5 091 edges from the damaged file
and 411 more nodes from two older sibling backups, merged them by primary key, rebuilt
the FTS indexes, and produced a database that passes `integrity_check` with 3 421 nodes
and 5 529 edges; the project's own `memory_gc` then removed 344 content-hash duplicates,
landing at **3 077 nodes / 5 185 edges** against 3 071 nodes reported before the failure.

---

## R10. Unifying `db_path()`

**Finding**: three implementations exist and they disagree.

| Location | Fallback when `data_dir()` is `None` | Creates the directory |
|---|---|---|
| [crates/aurelius/src/mcp/handlers/mod.rs:19](../../crates/aurelius/src/mcp/handlers/mod.rs) | `/tmp` | yes |
| [crates/au/src/commands.rs:8](../../crates/au/src/commands.rs) | `~/.local/share` (a literal, un-expanded string) | yes |
| [crates/au/src/view.rs:18](../../crates/au/src/view.rs) | `~/.local/share` (same literal) | **no** |

**Decision**: one `db_path()` in `aurelius-core::db`, re-exported; the three local
copies deleted. Principle II calls divergent path logic a correctness bug: on a machine
where `data_dir()` returns `None`, the MCP server and the CLI would operate on two
different files while appearing to share one.

---

## Resolved unknowns

| Question | Answer | Source |
|---|---|---|
| Does `execute_batch` surface a refused `journal_mode`? | No — row stepped and discarded | R2 |
| Is a per-open integrity check affordable? | Yes — 2.2–4.5 ms at 2 MB, ~10 ms at 7 MB | R4 |
| Can `quick_check` catch the incident's shape? | Yes, before any write | R4 |
| Is `quick_check` usable as the per-open gate? | **No** — in 3.45 it validates FTS5, which needs write access and a lock, so it false-positives on healthy databases under concurrency and on any read-only connection. Gate reads the header instead | R4a |
| Does `VACUUM INTO` work read-only / with binds / with a live WAL? | Yes to all three | R5 |
| Can migrations be wrapped without touching their bodies? | Yes — `Transaction::new_unchecked` takes `&Connection` | R6 |
| Can column presence be checked structurally? | Yes — `pragma_table_info(?1)` | R7 |
| Is an in-process repair implementable? | No — no `sqlite3_recover` binding | R9 |
