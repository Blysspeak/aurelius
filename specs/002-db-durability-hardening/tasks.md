---

description: "Task list for 002-db-durability-hardening"
---

# Tasks: Database Durability & Integrity Hardening

**Input**: Design documents from `/specs/002-db-durability-hardening/`
**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/cli-db.md](./contracts/cli-db.md)

**Tests**: REQUIRED for this feature. Spec FR-026 and constitution Principle V both
demand it, and every test below must be shown failing against the pre-change code
before its implementation task is done. The workspace currently has **zero** tests.

**Organization**: Grouped by user story. Note that four of the five stories edit the
same file (`crates/aurelius-core/src/db.rs`) — that file is the single choke point every
process passes through, which is why the change is small. Consequence: story phases are
**sequential**, and `[P]` appears only where genuinely different files are touched.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US5)
- Exact file paths included in every task

## Path Conventions

Rust workspace: `crates/aurelius-core/`, `crates/aurelius/`, `crates/au/`. Tests live
inline in `#[cfg(test)] mod tests` at the bottom of the module under test — there is no
`tests/` directory and this feature does not create one.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: make the typed-error dependency available. Nothing else is needed — no new
crates, modules or directories.

- [x] T001 Add `thiserror = "2"` to `[workspace.dependencies]` in `Cargo.toml`
- [x] T002 Add `thiserror = { workspace = true }` to `[dependencies]` in `crates/aurelius-core/Cargo.toml`
- [x] T003 Verify the workspace still builds unchanged: `cargo build --workspace`

**Checkpoint**: dependency resolves, nothing else has moved.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the types and constants every user story below depends on. No behaviour
changes yet — after this phase the code compiles and behaves exactly as before.

**⚠️ CRITICAL**: no user story can start until this phase is complete.

- [x] T004 In `crates/aurelius-core/src/db.rs`, replace the `use anyhow::Result;` header with the rusqlite/std imports the feature needs (`ErrorCode`, `OpenFlags`, `OptionalExtension`, `Transaction`, `TransactionBehavior`, `PathBuf`, `Duration`) and declare `pub const SCHEMA_VERSION: i32 = 5;` and `const BUSY_TIMEOUT: Duration = Duration::from_secs(10);`
- [x] T005 In `crates/aurelius-core/src/db.rs`, define `pub enum DbError` with `thiserror` — variants `Corrupt { path, detail }`, `SchemaTooNew { found, supported }`, `JournalMode(String)`, `Sqlite(#[from] rusqlite::Error)` — and a module-local `type Result<T> = std::result::Result<T, DbError>;`. The `Corrupt` message MUST carry all five elements required by the error-message contract in `specs/002-db-durability-hardening/contracts/cli-db.md`, including the "never copy the database with cp/mv/rsync" rule
- [x] T006 In `crates/aurelius-core/src/db.rs`, add `pub fn db_path() -> PathBuf` — `dirs_next::data_dir()` joined with `aurelius/aurelius.db`, creating the parent directory
- [x] T007 In `crates/aurelius-core/src/lib.rs`, re-export `db::{DbError, db_path}` (and `CheckReport` once it exists in T024)
- [x] T008 In `crates/aurelius/src/mcp/handlers/mod.rs`, delete the local `db_path()` and use the core one; wrap the call as `Ok(db::open(&db_path())?)` so `DbError` converts at the `anyhow` boundary
- [x] T009 [P] In `crates/au/src/commands.rs`, delete the local `db_path()` and import the core one
- [x] T010 [P] In `crates/au/src/view.rs`, delete the local `db_path()` and import the core one (this copy also lacked `create_dir_all`, which the shared version provides)
- [x] T011 Add `#[cfg(test)] mod tests` to the bottom of `crates/aurelius-core/src/db.rs` with the shared harness: a `TmpDb` guard that builds a unique path under `std::env::temp_dir()` and removes the `.db`/`-wal`/`-shm` triple on drop, plus an `insert_node` helper. `expect` is permitted here — Principle III scopes its ban to runtime paths
- [x] T012 Confirm `cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings` is green with behaviour unchanged so far

**Checkpoint**: types exist, the database path has exactly one definition, the test
module compiles. No user-visible behaviour has changed yet.

---

## Phase 3: User Story 1 — A damaged database is refused, not silently rewritten (P1) 🎯 MVP

**Goal**: any open of a structurally damaged database fails immediately, explains
itself, and leaves the file byte-identical.

**Independent test**: point the binary at a copy of the preserved incident file
(`aurelius.db.CORRUPT-<timestamp>`), run ten ordinary operations, and confirm every one
refuses and the file's hash is unchanged.

### Tests for User Story 1

- [x] T013 [US1] In `crates/aurelius-core/src/db.rs` tests, add `corrupt_header_is_detected_at_open`: build a database, insert 500 nodes, `PRAGMA wal_checkpoint(TRUNCATE)`, then patch bytes 28..32 (big-endian header page count) to 3, and assert `open()` returns `DbError::Corrupt`. **Must fail before T014** — the current code opens the file and proceeds to write

### Implementation for User Story 1

- [x] T014 [US1] In `crates/aurelius-core/src/db.rs`, add `fn classify(err: rusqlite::Error, path: &Path) -> DbError` mapping `ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase` to `DbError::Corrupt`, and `fn verify(conn: &Connection, path: &Path) -> Result<()>` running `PRAGMA quick_check(1)` and returning `Corrupt` unless the answer is `ok`
- [x] T015 [US1] In `crates/aurelius-core/src/db.rs`, call `verify(&conn, path)?` inside `open()` **before** `migrate()`, so no schema work can touch a damaged file
- [x] T016 [US1] Run T013 and confirm it now passes; confirm the cost claim by timing `au search` against the real 7 MB database before and after (spec SC-007 budget: < 10 ms added)

**Checkpoint**: US1 is independently shippable — the failure class is stopped even if
nothing else in this feature lands.

---

## Phase 4: User Story 2 — A safe way to snapshot the database (P1)

**Goal**: one command produces a complete, consistent snapshot of a live database, so
users never reach for `cp`.

**Independent test**: with a writer holding rows in an un-checkpointed WAL, take a
snapshot and confirm it opens cleanly and contains those rows.

### Tests for User Story 2

- [x] T017 [US2] In `crates/aurelius-core/src/db.rs` tests, add `backup_captures_uncheckpointed_wal`: open a database, insert rows without checkpointing, call `backup_into` to a second path, open that copy and assert the rows are present and `integrity_check` returns `ok`. **Must fail before T019** — the function does not exist

### Implementation for User Story 2

- [x] T018 [US2] In `crates/aurelius-core/src/db.rs`, add `fn open_readonly(&Path) -> Result<Connection>` using `OpenFlags::SQLITE_OPEN_READ_ONLY` plus `busy_timeout`, and `fn sidecar(&Path, &str) -> PathBuf` for `-wal`/`-shm` paths
- [x] T019 [US2] In `crates/aurelius-core/src/db.rs`, add `pub fn backup_into(src: &Path, dest: &Path) -> Result<u64>` executing `VACUUM INTO ?1` on a read-only connection and returning the resulting file size
- [x] T020 [US2] In `crates/au/src/main.rs`, add `pub enum DbAction` (above `enum Commands`, next to `TaskAction`) with the `Backup { #[arg(short, long)] out: Option<String> }` variant, add `Commands::Db { #[command(subcommand)] action: DbAction }` before `Mcp`, and add the `Commands::Db { action } => commands::db(action).await` dispatch arm
- [x] T021 [US2] In `crates/au/src/commands.rs`, add `pub async fn db(action: DbAction)` dispatching on `DbAction`, and `fn db_backup_cli(path, out)` — default destination `aurelius-<UTC timestamp>.db` beside the database, refuse an existing destination via `anyhow::bail!`, print source/dest/size in the `✓` idiom used elsewhere in the file
- [x] T022 [US2] Run T017; then run `au db backup` against the real database while an MCP server is running, and verify the result with a fresh integrity check

**Checkpoint**: US1 + US2 together are a coherent release — damage is stopped, and the
operation that caused it now has a safe replacement.

---

## Phase 5: User Story 3 — Verify the database on demand (P2)

**Goal**: a read-only command that gives a plain verdict, usable from scripts.

**Independent test**: run against a healthy database (exit 0), the incident file
(exit non-zero, names the size discrepancy) and a snapshot; confirm none are modified.

### Tests for User Story 3

- [x] T023 [US3] In `crates/aurelius-core/src/db.rs` tests, extend `corrupt_header_is_detected_at_open` (or add `check_flags_file_larger_than_its_header`) to assert `check(path, false)` returns `ok == false` with a problem containing `past the end`. **Must fail before T024** — `check` does not exist

### Implementation for User Story 3

- [x] T024 [US3] In `crates/aurelius-core/src/db.rs`, add `pub struct CheckReport { ok, problems, page_size, page_count, file_bytes, wal_bytes, nodes, edges }` and `pub fn check(path: &Path, full: bool) -> Result<CheckReport>` — read-only connection, `quick_check(1)` or `integrity_check` per `full`, never migrating
- [x] T025 [US3] In `check()`, add the geometry findings: report `file_bytes > page_size × page_count` as the file-level-copy signature in plain words, and report `file_bytes < logical && wal_bytes == 0` as truncation. Do **not** flag the legitimate inverse case where a live `-wal` makes the logical size exceed the main file
- [x] T026 [US3] In `crates/au/src/main.rs`, add the `Check { #[arg(long)] full: bool }` variant to `DbAction`
- [x] T027 [US3] In `crates/au/src/commands.rs`, add `fn db_check_cli(path, full)` printing path, size/geometry, optional `wal:` line, record counts (`unreadable` when absent), then either `✓ Integrity OK (<mode>)` or `✗ Integrity FAILED` with the indented problem list and the `Next: au db backup` hint; exit non-zero on failure via `anyhow::bail!`
- [x] T028 [US3] Run T023; then run `au db check` and `au db check --full` against the real database, a snapshot, and a copy of the incident file — confirming the exit codes and that the incident file's hash is unchanged (spec SC-001, SC-002)

**Checkpoint**: the user can now verify anything, including their own backups.

---

## Phase 6: User Story 4 — Concurrent use does not lose writes (P2)

**Goal**: contention waits instead of failing, and the journaling mode is what the code
believes it is.

**Independent test**: 8 threads open the same fresh database simultaneously; all
succeed and the schema ends up correct.

### Tests for User Story 4

- [x] T029 [US4] In `crates/aurelius-core/src/db.rs` tests, add `concurrent_opens_all_succeed`: 8 threads released together by a `Barrier`, each calling `open()` on the same path, all results `Ok`, and the final version equal to `SCHEMA_VERSION`. **Must fail before T030/T031** — no busy timeout plus the migration race. Note in a comment that this test has a timing component

### Implementation for User Story 4

- [x] T030 [US4] In `crates/aurelius-core/src/db.rs`, call `conn.busy_timeout(BUSY_TIMEOUT)?` as the **first** statement after `Connection::open` in `open()` — before any statement that can take a lock
- [x] T031 [US4] In `crates/aurelius-core/src/db.rs`, issue `PRAGMA journal_mode=WAL` via `query_row`, compare the returned mode to `wal` case-insensitively, and return `DbError::JournalMode(actual)` on mismatch; then set `PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;` with `execute_batch`. Do **not** lower `synchronous` (spec FR-017)
- [x] T032 [US4] Audit the write paths flagged in the audit for discarded errors — `graph::add_edge(...).ok()` in `crates/aurelius/src/mcp/handlers/session.rs` (lines ~75, 89, 90, 117–121, 132) and `graph::touch_node(...).ok()` in `crates/aurelius/src/mcp/handlers/crud.rs` (~line 55) — and propagate the failures instead of discarding them, per spec FR-018
- [x] T033 [US4] Run T029 and confirm it passes

**Checkpoint**: everyday concurrency no longer silently drops writes.

---

## Phase 7: User Story 5 — An interrupted upgrade never leaves the graph half-migrated (P3)

**Goal**: migrations are atomic, idempotent, and can never be triggered by a
misclassified read error.

**Independent test**: force a failure after the destructive step; the database must be
indistinguishable from its pre-upgrade state, version included.

### Tests for User Story 5

- [x] T034 [US5] In `crates/aurelius-core/src/db.rs` tests, add `failed_migration_rolls_back_migrate_v4`: build a v5 database, clear the FTS index so v4's re-index is observable, delete `schema_version` rows ≥ 4, then create an **index** named `search_cache` to make the later `migrate_v5` fail deterministically. Assert `open()` errors, the recorded version is still 3, and the FTS index is still empty. **Must fail before T036/T037** — v4 auto-commits today
- [x] T035 [US5] In `crates/aurelius-core/src/db.rs` tests, add `schema_newer_than_binary_is_rejected` (write version 99, expect `DbError::SchemaTooNew { found: 99, supported: 5 }`) and `fresh_open_migrates_and_is_idempotent` (fresh `open` reaches `SCHEMA_VERSION` with `nodes_fts` present; a second `open` changes nothing). The first **must fail before T036** — the current code opens a newer database silently
- [x] T036 [US5] In `crates/aurelius-core/src/db.rs`, replace `get_schema_version` with `fn read_version(&Connection) -> Result<i32>` returning `0` **only** when the `schema_version` table is absent (checked via `object_exists` against `sqlite_master`) and propagating every other error; add `fn object_exists` and `fn column_exists` using `sqlite_master` and `pragma_table_info(?1)` respectively
- [x] T037 [US5] Rewrite `migrate()`: fast path returning immediately when `read_version == SCHEMA_VERSION` (no lock, no DDL); `SchemaTooNew` when greater; otherwise `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)`, create `schema_version` inside it, re-read the version inside the transaction, run the `v1..v5` chain with their version bumps, and `commit()`. Leave the `migrate_v1/v3/v4/v5` bodies untouched
- [x] T038 [US5] Change `set_schema_version` to `INSERT OR IGNORE INTO schema_version (version) VALUES (?1)`
- [x] T039 [US5] Rewrite `migrate_v2` to check each column with `column_exists` before its `ALTER TABLE`, removing the `e.to_string().contains("duplicate column")` match (constitution Principle III forbids classifying errors by message text)
- [x] T040 [US5] Run T034 and T035 and confirm they pass; confirm an existing v5 database takes the fast path (no DDL, no write lock) by opening the real database and observing that no `-wal` growth occurs

**Checkpoint**: all five stories delivered.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [x] T041 [P] Update `README.md`: version badge → the released version, MCP tool badge stays at 21, `au 1.x` sample output, add `au db check` / `au db backup` to the CLI block, add a **Backups** section carrying the "never copy the database while `au` is running" warning with the reason, and update the storage bullet to mention busy timeout / WAL verification / integrity gate and atomic migrations
- [x] T042 [P] Add the release entry to `CHANGELOG.md` in the file's existing style (`## [vX.Y.Z] — YYYY-MM-DD`, em-dash, `### Fixed` / `### Added`, short commit hash per item). No `[Unreleased]` section — this file does not use one
- [x] T043 [P] Update `CLAUDE.md`: add `db` to the `au` subcommand list (`merge` is also missing), correct the documented database path from `~/.local/share/aurelius/aurelius.db` to the platform-resolved `data_dir()/aurelius/aurelius.db` (`%APPDATA%\aurelius\` on Windows), and record that file-level copying of the database is prohibited
- [x] T044 Run the full gate set: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --release`
- [x] T045 End-to-end run against the live database with the release binary: `au db check`, `au db backup`, `au db check --full`, then `au search aurelius` and `au task list` to confirm nothing else moved
- [x] T046 Incident replay (the decisive check): copy `aurelius.db.CORRUPT-<timestamp>`, hash it, run ten operations against the copy, confirm every one refuses, confirm the output names the file-larger-than-header signature, and confirm the hash is unchanged (spec SC-001, SC-002)
- [x] T047 Resolve the release blocker recorded in `plan.md` Complexity Tracking before tagging: the installed `au.exe` reports 1.6.0 and exposes `au skills` plus four `skill_*` MCP tools present in no commit. Owner decision required — restore the source, or retire the `SessionStart` hook and treat 1.6.0 as burnt

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (Phase 1)** → blocks everything
- **Foundational (Phase 2)** → blocks all user stories; `DbError` and `db_path` are used by every one of them
- **US1 (Phase 3)** → depends only on Foundational. **This is the MVP.**
- **US2 (Phase 4)** → depends on Foundational; independent of US1 in principle, but shares `open_readonly`/`sidecar` with US3
- **US3 (Phase 5)** → depends on Foundational; reuses `open_readonly`/`sidecar` from T018, so run after US2 or hoist T018 earlier
- **US4 (Phase 6)** → depends on Foundational; edits the same `open()` body as US1
- **US5 (Phase 7)** → depends on Foundational; edits `migrate()`, which `open()` calls
- **Polish (Phase 8)** → after all stories

### Story independence

US1–US5 are independently *testable*, but four of them edit `crates/aurelius-core/src/db.rs`,
so they are **not** independently *parallelizable*. Sequential story order is the
correct execution plan here; the small blast radius is a feature, not an obstacle.

### Genuine parallel opportunities

- T009 and T010 — different files (`commands.rs`, `view.rs`)
- T041, T042, T043 — three different documentation files
- Nothing else. Any two tasks both touching `db.rs` must be sequential.

---

## Implementation Strategy

### MVP (stop the bleeding)

Phases 1–3 only: Setup + Foundational + US1. That alone converts "silently keeps
destroying the database" into "refuses and explains", which is the entire difference
between the incident and a bad afternoon. Shippable on its own.

### Recommended release scope

Phases 1–8. US1 stops the damage; US2 removes the trigger. Shipping US1 without US2
leaves the user with a refusal and still no safe way to take a backup — they would reach
for `cp` again.

### Verification discipline

Per constitution Principle V, each test task must be **run and observed failing** before
its implementation task is marked done. "The test was added" is not evidence; the
before/after asymmetry is. T046 is the acceptance gate for the feature as a whole.

---

## Summary

| Phase | Tasks | Story |
|---|---|---|
| 1 — Setup | T001–T003 | — |
| 2 — Foundational | T004–T012 | — |
| 3 — Integrity gate | T013–T016 | US1 (P1, MVP) |
| 4 — Safe snapshot | T017–T022 | US2 (P1) |
| 5 — On-demand check | T023–T028 | US3 (P2) |
| 6 — Concurrency | T029–T033 | US4 (P2) |
| 7 — Atomic migrations | T034–T040 | US5 (P3) |
| 8 — Polish | T041–T047 | — |

**Total**: 47 tasks — 6 tests, 27 implementation, 8 verification, 3 documentation,
3 setup. Parallel opportunities: 2 pairs/triples (T009+T010, T041+T042+T043).

---

## Execution notes (filled in during implementation)

- **T014/T015 — the gate changed shape.** `PRAGMA quick_check(1)` was implemented as the
  per-open gate and two tests failed against it: in SQLite 3.45 it also validates the FTS5
  inverted indexes, which needs write access and a lock, so it reported "database is locked"
  on a healthy database under concurrency and "attempt to write a readonly database" on
  read-only connections. The gate now reads the 100-byte header directly (no SQL, no lock,
  cannot false-positive) and every error on the open path is mapped through `classify`.
  `au db check` runs its check per ordinary table, skipping virtual tables. See
  [research.md](./research.md) R4a.
- **T025 nuance added:** the in-header page count is authoritative only when the change
  counter (offset 24) equals the version-valid-for marker (offset 92); otherwise SQLite
  derives the size from the file and the comparison would be noise.
- **T032** — `add_edge` failures now propagate (20 call sites). `touch_node` and
  `ensure_indexed` stay best-effort by design (an access counter must never fail a read) but
  are logged via `tracing::warn!` instead of discarded.
- **Two pre-existing clippy findings** had to be fixed for the `-D warnings` gate to pass
  (`redundant_closure` in `crud.rs`, `manual_is_multiple_of` in `task.rs` and `commands.rs`).
  Unrelated to this feature; the gate had never been run before.
- **T046 result:** 22 consecutive operations against the incident file changed **zero bytes**
  (SHA-256 identical before and after), and both `au db check` and ordinary commands refuse
  with the geometry signature named in plain words. Spec SC-001 and SC-002 met.
- **T047 resolved by the owner:** release as **1.7.0**, treating 1.6.0 as burnt, and remove
  the `aurelius-skills.sh` SessionStart hook. The skills store was empty, so no data is lost.
