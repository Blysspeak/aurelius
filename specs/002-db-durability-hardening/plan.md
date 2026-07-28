# Implementation Plan: Database Durability & Integrity Hardening

**Branch**: `002-db-durability-hardening` | **Date**: 2026-07-28 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/002-db-durability-hardening/spec.md`

## Summary

Close the failure class exposed by the 2026-07-27 corruption: a file-level copy over
the live database, followed by 24 hours of the code making it worse because a failed
schema-version read was silently read as "brand-new database".

Five changes, all in the storage layer plus one new CLI subcommand tree:

1. **Configure the connection correctly** — wait for contended locks, verify that WAL
   mode actually took effect, pin durability explicitly.
2. **Gate every open on a health check** — refuse to hand out a connection to a
   damaged file, with an error that names the problem and the next step.
3. **Make migrations atomic** — one immediate-write transaction, version re-read
   inside it, so a failure mid-way rolls the whole thing back and two processes
   cannot both decide the upgrade is pending.
4. **Stop misreading errors as state** — a version read that fails is an error, never
   "version 0"; column presence is checked structurally, not by matching English
   error text.
5. **Give users a safe alternative to `cp`** — `au db check` and `au db backup`.

Recovery of an already-damaged database is deliberately out of scope (see spec
Assumptions). The incident's data was salvaged out-of-band by a page-level scan.

## Technical Context

**Language/Version**: Rust 2021 edition
**Primary Dependencies**: rusqlite 0.31 (`bundled` → SQLite 3.45.0), clap 4 (derive), anyhow 1; **adds** thiserror 2
**Storage**: single local SQLite file at `data_dir()/aurelius/aurelius.db` (`%APPDATA%\aurelius\` on Windows, `~/.local/share/aurelius/` on Linux), WAL journal, schema v5
**Testing**: `cargo test` — `#[cfg(test)] mod tests` in `crates/aurelius-core/src/db.rs`. The workspace currently has **zero** tests; these are the first.
**Target Platform**: Windows 11 (primary, where the incident occurred), Linux, macOS
**Project Type**: Rust workspace — one library crate + two binaries (CLI `au`, daemon/MCP `aurelius`)
**Performance Goals**: the health check must stay cheap enough to run on every open. As delivered the gate is two `stat` calls plus a 100-byte header read — immeasurable next to process startup. (The originally planned `PRAGMA quick_check(1)` gate measured 2.2 ms warm / 4.5 ms cold at 2.2 MB but was replaced during implementation; see [research.md](./research.md) R4a.)
**Constraints**: no durability regression; no schema or file-format change; a database written by this version must remain readable by 1.5.0; MCP tool surface additive only
**Scale/Scope**: ~3 100 nodes / ~5 200 edges / 7 MB today; up to ~10 concurrent processes (3+ MCP servers, per-edit hook writers, viewer)

No NEEDS CLARIFICATION items remain — every open question was resolved empirically
against the bundled engine; see [research.md](./research.md).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

Evaluated against [constitution.md](../../.specify/memory/constitution.md) v1.0.0.

| Principle | Gate | Pre-design | Post-design |
|---|---|---|---|
| **I. Data Durability First** | No write to an unverified database; migrations atomic; write errors propagate; no destructive path reachable from a misclassified state; a supported snapshot exists | ❌ current code violates all five | ✅ this feature *is* the remedy: `verify()` gate, single `BEGIN IMMEDIATE` migration, `read_version` returns `Result`, `au db backup` |
| **II. One Local File, Many Processes** | Bounded lock wait; journaling mode verified; race-safe init; one shared path definition | ❌ no `busy_timeout`, WAL result discarded, 3 divergent `db_path()` | ✅ `busy_timeout` first, `journal_mode` asserted, migration serialised by the immediate transaction, `db_path()` unified in core |
| **III. Rust Clean Code** | No `unwrap`/`expect`/`panic` on runtime paths; `thiserror` in domain / `anyhow` at boundary; no classification by error text; dependencies justified | ❌ `unwrap_or(0)` on the version read; `e.to_string().contains("duplicate column")` | ✅ typed `DbError` via thiserror; `pragma_table_info` replaces text matching; one new dependency (`thiserror`), justified below |
| **IV. Surgical Simplicity** | Minimum code; no speculative abstraction; adjacent code untouched | — | ✅ `migrate_v1/v3/v4/v5` bodies untouched; `au db repair` rejected; `wal_autocheckpoint` / `trusted_schema` / `OnceLock` memoisation all rejected as speculative |
| **V. Verify Before Done** | End-to-end run against a real database; every fix ships a test that failed before | ❌ zero tests in the workspace | ✅ 6 tests, each with a stated pre-fix failure, plus a manual replay against the preserved incident file |
| **VI. MCP Surface Stability** | Additive only; installed binary must not expose uncommitted tools | ⚠️ **pre-existing violation, not caused by this feature** | ⚠️ still open — see Complexity Tracking |

**New dependency justification (Principle III)**: `thiserror` 2 is required by the
constitution's own rule ("`thiserror` for typed domain errors in library crates").
`aurelius-core` currently returns `anyhow::Result`, which cannot express "corrupt vs
schema-too-new vs everything else" for callers to act on. Nothing in std produces
`#[derive(Error)]` ergonomics; hand-rolling `Display`/`Error` for four variants is more
code for the same result.

**Gate result**: PASS with one recorded deviation (VI, pre-existing).

## Project Structure

### Documentation (this feature)

```text
specs/002-db-durability-hardening/
├── spec.md              # Feature specification
├── plan.md              # This file
├── research.md          # Phase 0 — empirical findings against SQLite 3.45
├── data-model.md        # Phase 1 — types introduced/changed
├── quickstart.md        # Phase 1 — how to use it, how to verify it
├── contracts/
│   └── cli-db.md        # Phase 1 — `au db` command contract
├── checklists/
│   └── requirements.md  # Spec quality checklist
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
crates/
├── aurelius-core/
│   ├── Cargo.toml               # + thiserror
│   └── src/
│       ├── db.rs                # nearly all of the change lives here:
│       │                        #   + DbError, SCHEMA_VERSION, BUSY_TIMEOUT
│       │                        #   + db_path(), open_readonly(), verify(), classify()
│       │                        #   + check() -> CheckReport, backup_into()
│       │                        #   ~ open(), migrate(), read_version(), set_schema_version()
│       │                        #   ~ migrate_v2()  (structural column check)
│       │                        #   = migrate_v1 / v3 / v4 / v5 UNCHANGED
│       │                        #   + #[cfg(test)] mod tests  <- first tests in the workspace
│       └── lib.rs               # re-export db::{DbError, CheckReport, db_path}
├── aurelius/
│   └── src/mcp/handlers/mod.rs  # local db_path() removed -> core; open_db() wraps DbError
└── au/
    └── src/
        ├── main.rs              # + DbAction enum, + Commands::Db, + dispatch arm
        ├── commands.rs          # local db_path() removed -> core; + db(), db_check_cli(), db_backup_cli()
        └── view.rs              # local db_path() removed -> core

Cargo.toml                       # + thiserror in [workspace.dependencies]
README.md                        # + Backups section, + "never cp the database" warning
CHANGELOG.md                     # + release entry
```

**Structure Decision**: No new crates, no new modules. The entire behavioural change is
concentrated in `crates/aurelius-core/src/db.rs`, which is already the single choke
point every process passes through — all nine `db::open` call sites inherit the fix
without being edited. The CLI addition follows the existing `Commands::Task { action }`
nesting idiom exactly ([main.rs:137-140](../../crates/au/src/main.rs)), so `au db` is
structurally identical to `au task`.

## Phase 0 — Research

See [research.md](./research.md). Every finding was verified by running against the
bundled engine rather than reasoned from documentation. Headline results:

- `PRAGMA journal_mode=WAL` returns the *resulting* mode as a row and raises **no
  error** when the switch is refused; `execute_batch` steps and discards that row, so
  the current code cannot detect a failed switch.
- `PRAGMA quick_check(1)` detects the incident's exact signature (patched header page
  count) *before* any write touches the file, and costs 2.2–4.5 ms at 2 MB — **but it
  cannot be the gate**: in SQLite 3.45 it also validates the FTS5 inverted indexes,
  which needs write access and a lock, so it reports "database is locked" on a healthy
  database under this project's own concurrency and "attempt to write a readonly
  database" on any read-only connection. Two tests caught this. The gate reads the
  100-byte header instead, and `au db check` checks per ordinary table (R4a).
- `VACUUM INTO ?` works from a **read-only** connection, accepts a bind parameter, and
  captures rows still sitting in an un-checkpointed `-wal`.
- `Transaction::new_unchecked(&Connection, Immediate)` exists in rusqlite 0.31 and
  takes `&Connection`, so `migrate(conn: &Connection)` keeps its signature and the
  `migrate_vN` bodies compile unchanged through `Deref`.
- `pragma_table_info(?1)` accepts a bind parameter — a structural column check needs no
  string formatting.
- `sqlite3_recover` has no rusqlite binding → `au db repair` cannot be implemented
  honestly and is dropped from scope.

## Phase 1 — Design

- [data-model.md](./data-model.md) — `DbError`, `CheckReport`, and the (unchanged)
  on-disk schema.
- [contracts/cli-db.md](./contracts/cli-db.md) — exact `au db check` / `au db backup`
  surface, output shape and exit codes.
- [quickstart.md](./quickstart.md) — how a user takes and verifies a backup, plus the
  full verification procedure for this change including the incident-file replay.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| **Principle VI** — the installed `au.exe` reports 1.6.0 and exposes `au skills` plus four `skill_*` MCP tools that exist in **no commit** (the repo is at 1.5.0; `main.rs` has no `Skills` variant). | Not introduced by this feature — found while auditing for it. Recorded because the constitution forbids tagging a release while the shipped surface and the repository disagree, and this feature ends in a release. | It cannot be ignored: reinstalling from this tree would delete `au skills`, which a live `SessionStart` hook invokes. It also cannot be fixed inside this feature — the source does not exist to merge. **The owner must decide before tagging**: restore the skills source, or retire the hook and treat 1.6.0 as burnt. Tracked as a release blocker, not as work in this plan. |
| One new dependency (`thiserror`) in a project whose constitution requires justification for each. | Principle III mandates typed domain errors, and the whole feature turns on callers distinguishing `Corrupt` from `SchemaTooNew` from everything else. | Hand-written `impl Display + Error` for four variants: more code, no compile-time guarantee the messages stay in sync with the variants, and it is precisely the boilerplate `thiserror` exists to remove. |
