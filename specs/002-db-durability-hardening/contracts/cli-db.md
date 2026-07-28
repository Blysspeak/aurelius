# Contract: `au db` — database maintenance commands

**Feature**: 002-db-durability-hardening
**Surface**: CLI (`au`), additive. No existing command, flag or output changes.

---

## Command tree

```text
au db check  [PATH] [--full]
au db backup [--out PATH]
```

> **Amended in v1.8.0.** `check` gained an optional positional `PATH`. The original
> contract argued against it ("pointing these commands at an arbitrary file is not a use
> case the incident motivates"), and that was wrong: the rolling backup hook added in
> v1.7.x needs to verify the snapshot it just wrote, and without a path it could only
> trust that `au db backup` had not lied. An unverified backup is a guess. `backup` still
> takes no source argument — there is exactly one knowledge graph to snapshot.

Nested exactly like the existing `au task <action>`
([main.rs:137-140](../../../crates/au/src/main.rs)):

```rust
#[derive(Subcommand)]
pub enum DbAction {
    /// Verify database integrity (read-only — never migrates, never writes)
    Check {
        /// Report every problem (full integrity_check) instead of stopping at the first
        #[arg(long)]
        full: bool,
    },
    /// Safe snapshot via SQLite VACUUM INTO — the only correct way to copy a live database
    Backup {
        /// Destination file (default: aurelius-<UTC timestamp>.db next to the database)
        #[arg(short, long)]
        out: Option<String>,
    },
}

// in enum Commands, placed before `Mcp`:
    /// Database maintenance — integrity check and safe backup
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
```

Both commands operate on the single global database resolved by `db::db_path()`. There
is no `--path` flag: pointing these commands at an arbitrary file is not a use case the
incident motivates, and adding it would be speculative (Principle IV).

---

## `au db check [PATH] [--full]`

**Purpose**: read-only verdict on the structural integrity of a database — the knowledge
graph by default, or any database file given as `PATH` (typically a snapshot).

**Guarantees**

- Opens the file `SQLITE_OPEN_READ_ONLY`. Never migrates. Never writes a page.
- A missing `PATH` is reported as `no database at <path>` with a non-zero exit, rather
  than surfacing an engine error.
- Works on a database older than the current schema, newer than it, or damaged.
- Default mode stops at the first problem (`quick_check(1)`); `--full` reports all
  (`integrity_check`).

> Note: opening a WAL database read-only creates empty `-wal`/`-shm` siblings. No page
> of the database itself is written.

**Output — healthy**

```text
Database: C:\Users\blyss\AppData\Roaming\aurelius\aurelius.db
  size:     7208960 bytes (1760 pages × 4096)
  content:  3077 nodes, 5185 edges
✓ Integrity OK (quick_check)
```

The `wal:` line is printed only when a `-wal` sibling exists and is non-empty.
The mode in parentheses is `quick_check` or `integrity_check` per `--full`.

**Output — damaged** (this is the incident file)

```text
Database: C:\Users\blyss\AppData\Roaming\aurelius\aurelius.db.CORRUPT-20260728T231147Z
  size:     7294976 bytes (181 pages × 4096)
  content:  unreadable
✗ Integrity FAILED
    *** in database main ***
    Tree 10 page 169: btreeInitPage() returns error code 11
    file is 7294976 bytes but the header describes only 181 pages of 4096 (741376 bytes)
    — 6553600 bytes lie past the end of the declared database; this is the signature of
    a file-level copy over a live WAL database
  Next: `au db backup` to snapshot what is still readable.
```

**Exit codes**

| Code | Condition |
|---|---|
| 0 | healthy |
| non-zero | any problem found, or the file is missing/unopenable |

Non-zero on damage is required by FR-014 so the command can gate a hook or a script.

**Contract details**

- `content: unreadable` is printed rather than treated as a failure when the counts
  cannot be read — the geometry and problem list are still useful.
- Problems are printed verbatim, one per line, indented four spaces; multi-line
  problems keep their line breaks.
- All output goes to stdout; the non-zero exit carries the failure. No colour crates
  are introduced — the project uses none.

---

## `au db backup [--out PATH]`

**Purpose**: the supported alternative to copying the file.

**Guarantees**

- Uses SQLite's own `VACUUM INTO` from a read-only connection: a consistent
  point-in-time snapshot taken while other processes read and write.
- Includes data still sitting in an un-checkpointed `-wal`.
- Does not modify the source.
- Refuses rather than overwrites when the destination exists.
- Fails loudly on a damaged source instead of producing a plausible-looking bad
  backup — a successful backup is by construction a readable one.

**Default destination**: `aurelius-<UTC timestamp>.db` next to the database, e.g.
`aurelius-20260728T231147Z.db`. The full path is printed.

**Output**

```text
✓ Backup written
  source: C:\Users\blyss\AppData\Roaming\aurelius\aurelius.db
  dest:   C:\Users\blyss\AppData\Roaming\aurelius\aurelius-20260728T231147Z.db
  size:   7208960 bytes
```

**Exit codes**

| Code | Condition |
|---|---|
| 0 | snapshot written |
| non-zero | destination exists, source damaged, source missing, I/O failure |

**Not in this contract**

- `au db restore` — restoring is a manual, documented procedure (stop everything,
  replace, verify). The dangerous step is "stop everything", which the tool cannot
  enforce for processes it did not start. Automating the rest would imply a safety the
  command cannot deliver.
- `au db repair` — `sqlite3_recover` has no rusqlite binding; see
  [research.md](../research.md) R9. A repair that silently under-recovers is worse than
  none.

---

## Error-message contract (all commands and every `db::open`)

When the database is found to be damaged, the message MUST contain, in this order:

1. the path;
2. the engine's own description of the problem;
3. `au db check --full` as the way to see the whole report;
4. `au db backup` as the way to salvage what is readable;
5. the rule: never copy or restore the database with `cp`/`mv`/`rsync` while `au` or an
   MCP server is running.

Point 5 is the one that closes the loop. The incident happened because that rule was
documented nowhere and the product offered no alternative; the alternative now exists,
and the rule now appears at the exact moment the user is most likely to reach for `cp`.

---

## MCP surface

**Unchanged.** No MCP tool is added, renamed or removed by this feature; the tool count
stays at 21. Every existing tool inherits the hardened `db::open` without any change to
its signature or its result shape, which satisfies Principle VI (additive only).
