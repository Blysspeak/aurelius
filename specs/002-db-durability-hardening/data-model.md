# Phase 1 Data Model: Database Durability & Integrity Hardening

**Feature**: 002-db-durability-hardening
**Date**: 2026-07-28

This feature changes **no** persisted data. The on-disk schema stays at v5; no table,
column, index or trigger is added, removed or altered. What follows is the in-memory
model introduced in `crates/aurelius-core/src/db.rs`.

---

## Persisted schema: unchanged

| Aspect | State |
|---|---|
| Schema version | 5, unchanged |
| Tables / columns / indexes / triggers | unchanged |
| File format, page size, encoding | unchanged |
| Journal mode | WAL, unchanged (now *verified* rather than assumed) |
| `synchronous` | `FULL`, unchanged in effect (now set explicitly rather than inherited from build flags) |

**Backward compatibility**: a database written by this version is byte-compatible with
1.5.0. `schema_version` is deliberately retained instead of moving to
`PRAGMA user_version` — see [research.md](./research.md) R8.

---

## `SCHEMA_VERSION` — the version this binary understands

```rust
/// Highest schema version this binary understands.
pub const SCHEMA_VERSION: i32 = 5;
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);
```

Previously the number `5` was implicit, repeated inline in `migrate`. Naming it is what
makes `SchemaTooNew` expressible.

**State transitions of the stored version**

```text
absent (no schema_version table)  --migrate--> 5      (fresh database)
1..4                              --migrate--> 5      (upgrade, atomic)
5                                 --fast path-> 5     (no lock, no DDL)
>5                                --refuse---> DbError::SchemaTooNew
unreadable                        --refuse---> DbError::Sqlite / DbError::Corrupt
```

The last row is the behavioural core of the feature. Today an unreadable version
becomes `0`, which routes into "fresh database" and runs destructive DDL over live
data.

---

## `DbError` — typed domain error

Replaces `anyhow::Result` inside `aurelius-core::db`. Four variants, because callers
need to act differently on each:

```rust
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The file is not a usable database image. Carries the engine's own
    /// description plus the operator guidance the user actually needs.
    #[error("...")]
    Corrupt { path: String, detail: String },

    /// Written by a newer binary than this one. Opening it read-write would
    /// mean writing under an older understanding of the schema.
    #[error("database schema is v{found}, this binary supports v{supported} — upgrade `au`")]
    SchemaTooNew { found: i32, supported: i32 },

    /// The connection is not in WAL mode and the switch was refused.
    #[error("could not switch the database to WAL journal mode (it reports '{0}')")]
    JournalMode(String),

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}
```

| Variant | Raised when | Caller's action |
|---|---|---|
| `Corrupt` | `quick_check` fails, or the engine returns `SQLITE_CORRUPT` / `SQLITE_NOTADB` | stop; run `au db check --full`; `au db backup` to salvage |
| `SchemaTooNew` | stored version > `SCHEMA_VERSION` | upgrade the binary |
| `JournalMode` | `PRAGMA journal_mode=WAL` returns something other than `wal` | investigate the lock holder / filesystem |
| `Sqlite` | anything else, including `SQLITE_BUSY` after the timeout | propagate |

**`Corrupt` message content** — the text is part of the contract, because it is the
only place a user meets the rule that caused the incident:

```
database image is damaged: <path>
  <engine detail>
  hint: `au db check --full` for the full report, `au db backup` to snapshot what is still readable.
  never copy or restore aurelius.db with cp/mv/rsync while `au` or an MCP server is running — use `au db backup`
```

**Boundary conversion**: application crates keep `anyhow`. The ripple is one line —
`crates/aurelius/src/mcp/handlers/mod.rs` wraps `db::open(...)` in `Ok(...?)`. The
`view.rs` call sites already use `.map_err(...)` and `commands.rs` sites are inside
`anyhow` functions using `?`; both compile unchanged.

---

## `CheckReport` — result of a read-only inspection

```rust
#[derive(Debug)]
pub struct CheckReport {
    pub ok: bool,
    pub problems: Vec<String>,
    pub page_size: i64,
    pub page_count: i64,
    pub file_bytes: u64,
    pub wal_bytes: u64,
    pub nodes: Option<i64>,
    pub edges: Option<i64>,
}
```

| Field | Meaning | Notes |
|---|---|---|
| `ok` | `problems.is_empty()` | the verdict; drives the process exit code |
| `problems` | engine findings plus derived geometry findings | free text, one entry per problem, printed verbatim |
| `page_size`, `page_count` | header geometry | used to derive the logical size |
| `file_bytes` | actual file size | `file_bytes > page_size × page_count` is the incident's fingerprint |
| `wal_bytes` | size of the `-wal` sibling, 0 if absent | a file smaller than its header describes is only a problem when this is 0 |
| `nodes`, `edges` | record counts | `None` when the tables cannot be read — that is information, not a failure |

**Invariants**

- Producing a `CheckReport` never writes to the database: the connection is opened
  `SQLITE_OPEN_READ_ONLY` and no migration runs. It therefore works on databases older
  than v5, newer than v5, and damaged.
- `ok == problems.is_empty()` always.
- Derived problems are phrased to name the cause, not just the symptom — the
  file-larger-than-header entry says outright that it is the signature of a file-level
  copy over a live database.

---

## Function surface added to `aurelius-core::db`

| Function | Visibility | Contract |
|---|---|---|
| `db_path() -> PathBuf` | `pub` | the one definition of where the database lives; creates the parent directory |
| `open(&Path) -> Result<Connection>` | `pub` (existing) | now: busy timeout → WAL asserted → durability → health gate → migrate |
| `open_readonly(&Path) -> Result<Connection>` | private | read-only connection with a busy timeout, no migration |
| `check(&Path, full: bool) -> Result<CheckReport>` | `pub` | read-only; `quick_check(1)` or full `integrity_check`, plus geometry checks |
| `backup_into(&Path, &Path) -> Result<u64>` | `pub` | `VACUUM INTO` from a read-only connection; returns the resulting size |
| `verify(&Connection, &Path) -> Result<()>` | private | the health gate used by `open` |
| `classify(rusqlite::Error, &Path) -> DbError` | private | maps `SQLITE_CORRUPT` / `SQLITE_NOTADB` to `Corrupt` |
| `read_version(&Connection) -> Result<i32>` | private | `0` only when the `schema_version` table is absent |
| `object_exists` / `column_exists` | private | structural existence checks, no error-text matching |
| `sidecar(&Path, &str) -> PathBuf` | private | `-wal` / `-shm` path derivation |

---

## Entity relationships (conceptual)

```text
db_path() ──► Database file ──┬── open()          ──► Connection   (verified, migrated)
                              ├── check()         ──► CheckReport  (read-only verdict)
                              └── backup_into()   ──► Snapshot file (independent, verifiable)

Snapshot file ──► check()  ──► CheckReport        (a snapshot is verified the same way)
```

A snapshot is an ordinary database file: the same `check` applies to it, which is what
makes "take a backup and verify it" a two-command operation with no special cases.
