use rusqlite::{
    params, Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

// SCHEMA_VERSION must be incremented whenever the database schema changes.
// The migration chain in `migrate()` updates older instances on connection.
pub const SCHEMA_VERSION: i32 = 18;

/// How long a connection waits for a lock another process holds. Long enough to
/// absorb a checkpoint or a migration, short enough that a genuinely stuck lock
/// surfaces instead of hanging an editor hook.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// Size of the WAL file header. A `-wal` no longer than this holds no frames.
const WAL_HEADER_BYTES: u64 = 32;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(
        "database image is damaged: {path}\n  {detail}\n  \
         hint: `au db check --full` for the full report, `au db backup` to snapshot what is \
         still readable.\n  \
         never copy or restore aurelius.db with cp/mv/rsync while `au` or an MCP server is \
         running — use `au db backup`. A process still holding the old file keeps writing \
         to it, and on a clean exit SQLite deletes `aurelius.db-wal`/`-shm` by name — the \
         new file's. Stop every `au` process (MCP servers, `au daemon`) before a swap"
    )]
    Corrupt { path: String, detail: String },

    #[error(
        "database schema is v{found}, this binary supports only v{supported}: the installed \
         `au` is older than the database. If you are an agent: do not retry in a loop, do not \
         copy, restore or re-create the database, and do not run a freshly built binary \
         against it (the MCP memory tools hit this same check). The fix is for the owner to \
         install the new binary into both ~/.local/bin and ~/.cargo/bin (the service starts \
         the first one). Until then memory is unavailable in this session: continue the task \
         without it and say so in your report"
    )]
    SchemaTooNew { found: i32, supported: i32 },

    #[error("could not switch the database to WAL journal mode (it reports '{0}')")]
    JournalMode(String),

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

type Result<T> = std::result::Result<T, DbError>;

/// The one definition of where the knowledge graph lives. Every binary resolves
/// the database through this function — divergent copies would let the CLI and
/// the MCP server operate on different files while appearing to share one.
pub fn db_path() -> PathBuf {
    // Active home override (AURELIUS_HOME env var, or a persisted `au home
    // use`; see crate::home) — the DB lives directly under it instead of
    // the real OS data dir. Neither set means 100% unchanged default
    // behavior.
    if let Some(base) = crate::home::resolve() {
        std::fs::create_dir_all(&base).ok();
        return base.join("aurelius.db");
    }
    let base = dirs_next::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("aurelius");
    std::fs::create_dir_all(&base).ok();
    base.join("aurelius.db")
}

/// File name of the write marker, kept beside the database it describes.
pub const WRITE_MARKER: &str = "last-write";

/// Where the write marker for the database behind `conn` lives, or `None` when
/// that database has no file — `Connection::path` reports `Some("")` for an
/// in-memory or temporary database.
///
/// Derived from the live connection instead of from [`db_path`] on purpose: a
/// test, or a process pointed at another `AURELIUS_HOME`, must not be able to
/// stamp the marker belonging to the real graph.
pub fn write_marker_path(conn: &Connection) -> Option<PathBuf> {
    let path = conn.path().filter(|p| !p.is_empty())?;
    Path::new(path).parent().map(|dir| dir.join(WRITE_MARKER))
}

/// Stamps the write marker — the mtime of that file is the datum, and it
/// answers "when was a knowledge record last written".
///
/// The database file's own mtime cannot answer it. `au trace`, wired to
/// PostToolUse, appends a row on every single tool call, so `aurelius.db` is
/// rewritten on the order of a thousand times an hour without a single record
/// being added; anything reading that mtime sees an age that never leaves
/// zero. A separate file touched only here is the only signal a caller that
/// may do nothing but `stat` — the status line is rendered every turn — can
/// read.
///
/// Never propagates a failure: a marker that could not be stamped only leaves
/// a reader showing a stale age, which is a far smaller harm than refusing to
/// store the record it accompanies.
pub fn mark_write(conn: &Connection) {
    let Some(path) = write_marker_path(conn) else {
        return;
    };
    let _ = std::fs::write(
        path,
        b"aurelius write marker - the mtime of this file is when a record was last written\n",
    );
}

type SqliteExtensionEntryPoint = unsafe extern "C" fn(
    db: *mut rusqlite::ffi::sqlite3,
    pz_err_msg: *mut *const std::os::raw::c_char,
    p_thunk: *const rusqlite::ffi::sqlite3_api_routines,
) -> std::os::raw::c_int;

pub fn init_sqlite_extensions() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            SqliteExtensionEntryPoint,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    });
}

pub fn open(path: &Path) -> Result<Connection> {
    init_sqlite_extensions();
    // Health gate first: never let a connection — let alone the migration
    // chain — touch an image whose own header disagrees with the file.
    verify(path)?;

    let conn = Connection::open(path).map_err(|e| classify(e, path))?;

    // Before anything that can take a lock: SQLite's default busy handler
    // fails immediately. The deployment guarantees contention — a hook spawns
    // a writer on every file edit, several MCP servers run at once.
    conn.busy_timeout(BUSY_TIMEOUT)?;

    ensure_wal(&conn, path)?;

    // Durability is pinned explicitly rather than inherited from build flags.
    conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
        .map_err(|e| classify(e, path))?;

    migrate(&conn).map_err(|e| match e {
        DbError::Sqlite(inner) => classify(inner, path),
        other => other,
    })?;
    Ok(conn)
}

/// Put the connection in WAL mode and *confirm* it.
///
/// `PRAGMA journal_mode` returns the RESULTING mode as a row and raises no
/// error when the switch is refused; `execute_batch` steps that row and throws
/// it away, so the current code cannot tell WAL from rollback-journal. Read the
/// row and check it.
///
/// Converting a database into WAL needs a brief exclusive lock, and that
/// acquisition can return SQLITE_BUSY without consulting the busy handler — so
/// a fresh database opened by several processes at once needs bounded retries.
/// Once the database is in WAL the pragma is a no-op and takes no lock at all.
fn ensure_wal(conn: &Connection, path: &Path) -> Result<()> {
    let deadline = std::time::Instant::now() + BUSY_TIMEOUT;
    loop {
        match conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0)) {
            Ok(mode) if mode.eq_ignore_ascii_case("wal") => return Ok(()),
            Ok(mode) => return Err(DbError::JournalMode(mode)),
            Err(e) => {
                let busy = matches!(
                    &e,
                    rusqlite::Error::SqliteFailure(f, _)
                        if matches!(f.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                );
                if !busy || std::time::Instant::now() >= deadline {
                    return Err(classify(e, path));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Read-only connection: no migration, no page ever written.
///
/// Public because two callers must read the graph without altering it, and
/// [`open`] is unusable for both: it runs `ensure_wal` — `PRAGMA
/// journal_mode=WAL`, which takes a brief exclusive lock — then pins `PRAGMA
/// synchronous=FULL` and runs `migrate`. A reader that migrates is not a
/// reader.
///
/// The two are the `au eval` run and the turn-start recognition hook. For eval
/// the fixture is also the yardstick: a single write during a run changes the
/// input of the next one — recall bumps `access_count` on every node it shows,
/// and that counter is a ranking factor — so a run that needed to write is a
/// failed run, not a failed case. The hook fires on every single turn and must
/// never become a writer, let alone a migrator, of the database it consults.
///
/// `SQLITE_OPEN_READ_ONLY` is what enforces that: an attempted write fails on
/// the connection instead of being caught by review.
pub fn open_readonly(path: &Path) -> Result<Connection> {
    init_sqlite_extensions();
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_owned();
    raw.push(suffix);
    PathBuf::from(raw)
}

fn classify(err: rusqlite::Error, path: &Path) -> DbError {
    match &err {
        rusqlite::Error::SqliteFailure(failure, message)
            if matches!(
                failure.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
            ) =>
        {
            DbError::Corrupt {
                path: path.display().to_string(),
                detail: message.clone().unwrap_or_else(|| failure.to_string()),
            }
        }
        _ => DbError::Sqlite(err),
    }
}

/// Physical geometry of the file versus what its own header claims.
struct Geometry {
    page_size: i64,
    page_count: i64,
    file_bytes: u64,
    wal_bytes: u64,
    problems: Vec<String>,
}

/// Read the geometry straight out of the 100-byte database header.
///
/// Deliberately not via `PRAGMA page_size` / `page_count`: on a damaged image
/// the engine refuses to answer at all, which is precisely when this report
/// matters most. Reading the bytes needs no connection, takes no lock, and
/// cannot fail on a healthy database.
///
/// While the `-wal` holds frames, the header in the main file is not the
/// current one: the newest page 1 may live in the WAL, so the main file can
/// legitimately be shorter OR longer than its header says. Longer is what a
/// PASSIVE checkpoint leaves behind when a reader holds an older snapshot: it
/// copies every page whose newest frame predates that snapshot — extending the
/// file — but skips page 1 if a later transaction rewrote it, and truncates only
/// after backfilling everything. SQLite itself rejects only a header larger than
/// the file. Judging that state as damage locked every `au` call out on
/// 2026-09-13 and 2026-09-15 (nBackfill 981 of mxFrame 1001, page 1 newest in
/// frame 991, file 26 pages past a header one checkpoint behind), and the
/// lock-out sustained itself: no connection could write the transaction whose
/// checkpoint heals the header.
///
/// So the size comparison runs only against a WAL with no frames. A file longer
/// than its header with nothing left to checkpoint is the fingerprint of an
/// in-place overwrite by a shorter image (the 2026-07-27 incident file:
/// 7 294 976 bytes against 181 pages x 4096).
fn geometry(path: &Path) -> Geometry {
    let file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let wal_bytes = std::fs::metadata(sidecar(path, "-wal"))
        .map(|m| m.len())
        .unwrap_or(0);

    let mut header = [0u8; 100];
    let read = std::fs::File::open(path).and_then(|mut f| {
        use std::io::Read;
        f.read_exact(&mut header)
    });
    // A missing or empty file is a database about to be created, not a problem.
    if read.is_err() || file_bytes < 100 {
        return Geometry {
            page_size: 0,
            page_count: 0,
            file_bytes,
            wal_bytes,
            problems: Vec::new(),
        };
    }

    let mut problems = Vec::new();
    if &header[..16] != b"SQLite format 3\0" {
        problems.push(format!(
            "file does not start with the SQLite header magic — {} is not a database",
            path.display()
        ));
        return Geometry {
            page_size: 0,
            page_count: 0,
            file_bytes,
            wal_bytes,
            problems,
        };
    }

    let be16 = |o: usize| u32::from(u16::from_be_bytes([header[o], header[o + 1]]));
    let be32 =
        |o: usize| u32::from_be_bytes([header[o], header[o + 1], header[o + 2], header[o + 3]]);

    // Offset 16: page size. The value 1 encodes 65536.
    let page_size = match be16(16) {
        1 => 65_536i64,
        other => i64::from(other),
    };
    let page_count = i64::from(be32(28));
    // Offset 28's page count is authoritative only when the change counter
    // (24) equals the version-valid-for marker (92). Otherwise SQLite derives
    // the size from the file itself and the comparison below would be noise.
    let header_size_authoritative = be32(24) == be32(92);
    let wal_has_frames = wal_bytes > WAL_HEADER_BYTES;

    let logical = u64::try_from(page_size.saturating_mul(page_count)).unwrap_or(0);
    if header_size_authoritative && logical > 0 && !wal_has_frames {
        if file_bytes > logical {
            problems.push(format!(
                "file is {file_bytes} bytes but the header describes only {page_count} pages of \
                 {page_size} ({logical} bytes) — {} bytes lie past the end of the declared \
                 database with no -wal frames to account for them; this is the signature of \
                 the file being overwritten in place by a shorter image",
                file_bytes - logical
            ));
        } else if file_bytes < logical {
            problems.push(format!(
                "file is {file_bytes} bytes, the header describes {logical} bytes and there is \
                 no -wal to account for the difference — the file is truncated"
            ));
        }
    }

    Geometry {
        page_size,
        page_count,
        file_bytes,
        wal_bytes,
        problems,
    }
}

/// Health gate, run before every open.
///
/// Deliberately geometry-only. A full `PRAGMA quick_check` also validates the
/// FTS5 inverted indexes, which in SQLite 3.45 needs write access and a lock —
/// so under this project's own concurrency it reports "database is locked" on a
/// perfectly healthy database. A gate that refuses healthy databases is worse
/// than no gate. Structural damage past this point is still caught, because
/// every corrupt read is mapped through `classify` into an actionable error;
/// the exhaustive scan lives in `au db check`.
///
/// Run per open rather than once per process on purpose: the file can be
/// swapped in the middle of a long-lived process's life, which is exactly what
/// happened on 2026-07-27.
///
/// The header is evidence, SQLite is the verdict. A geometry finding alone
/// refuses nothing: the engine is asked (read-only, per-table `quick_check`)
/// and only a finding it confirms becomes a refusal. A refusal locks out every
/// CLI call, hook and MCP request at once, and on 2026-09-15 that lock-out is
/// what drove a session to swap the live file with `mv`/`cp` — the one action
/// that really can damage it. The scan runs only on this rare path, never on a
/// clean open.
fn verify(path: &Path) -> Result<()> {
    let geometry = geometry(path);
    if geometry.problems.is_empty() {
        return Ok(());
    }
    let detail = geometry.problems.join("\n  ");
    let confirmed = match open_readonly(path) {
        Ok(conn) => table_findings(&conn, false),
        Err(e) => vec![e.to_string()],
    };
    if confirmed.is_empty() {
        eprintln!(
            "aurelius: warning: {}\n  {detail}\n  SQLite reads the database as intact \
             (quick_check ok), continuing — run `au db check --full`",
            path.display()
        );
        return Ok(());
    }
    Err(DbError::Corrupt {
        path: path.display().to_string(),
        detail: format!("{detail}\n  SQLite confirms: {}", confirmed.join("; ")),
    })
}

/// Names of ordinary (non-virtual) tables. FTS5 virtual tables are excluded
/// because validating them requires write access; their shadow tables are
/// ordinary tables and are checked like any other.
fn checkable_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
           AND sql NOT LIKE 'CREATE VIRTUAL TABLE%'
         ORDER BY name",
    )?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(names)
}

/// SQLite's own page-level verdict, one ordinary table at a time — empty means
/// the engine found nothing.
///
/// Per table rather than whole-database: the whole-database form also validates
/// FTS5 inverted indexes, which needs write access, so on a read-only connection
/// it reports "attempt to write a readonly database" for every healthy database.
/// Checking ordinary tables individually covers the same b-tree integrity
/// without that false positive.
fn table_findings(conn: &Connection, full: bool) -> Vec<String> {
    let verb = if full {
        "integrity_check"
    } else {
        "quick_check"
    };
    let tables = match checkable_tables(conn) {
        Ok(tables) => tables,
        Err(e) => return vec![format!("cannot enumerate tables: {e}")],
    };
    let mut findings = Vec::new();
    for table in tables {
        let sql = format!("PRAGMA {verb}('{}')", table.replace('\'', "''"));
        match conn.query_row(&sql, [], |row| row.get::<_, String>(0)) {
            Ok(report) if report.eq_ignore_ascii_case("ok") => {}
            Ok(report) => findings.push(format!("{table}: {report}")),
            // The engine bails out mid-scan on a badly damaged file. That is a
            // finding to report, not a reason to crash.
            Err(e) => findings.push(format!("{table}: {e}")),
        }
        // Quick mode answers "is it damaged", not "how much" — stop at the
        // first table with a finding. `--full` reports everything.
        if !full && !findings.is_empty() {
            break;
        }
    }
    findings
}

/// Read-only integrity report. Never migrates, never writes a database page, and
/// therefore works on databases older than the current schema, newer than it, or
/// damaged.
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

pub fn check(path: &Path, full: bool) -> Result<CheckReport> {
    let geometry = geometry(path);
    let mut problems = geometry.problems;
    let conn = match open_readonly(path) {
        Ok(conn) => conn,
        // A file the engine will not even open is a finding, not a crash.
        Err(e) => {
            problems.push(e.to_string());
            return Ok(CheckReport {
                ok: false,
                problems,
                page_size: geometry.page_size,
                page_count: geometry.page_count,
                file_bytes: geometry.file_bytes,
                wal_bytes: geometry.wal_bytes,
                nodes: None,
                edges: None,
            });
        }
    };

    problems.extend(table_findings(&conn, full));

    Ok(CheckReport {
        ok: problems.is_empty(),
        problems,
        page_size: geometry.page_size,
        page_count: geometry.page_count,
        file_bytes: geometry.file_bytes,
        wal_bytes: geometry.wal_bytes,
        nodes: conn
            .query_row("SELECT count(*) FROM nodes", [], |row| row.get(0))
            .ok(),
        edges: conn
            .query_row("SELECT count(*) FROM edges", [], |row| row.get(0))
            .ok(),
    })
}

/// Snapshot the database with SQLite's own `VACUUM INTO` — the only safe way to
/// copy a live database. Includes everything still sitting in the `-wal`, and
/// fails on a damaged source rather than producing a plausible-looking bad
/// backup.
pub fn backup_into(src: &Path, dest: &Path) -> Result<u64> {
    let conn = open_readonly(src)?;
    let dest_str = dest.to_string_lossy().into_owned();
    conn.execute("VACUUM INTO ?1", params![dest_str])
        .map_err(|e| classify(e, src))?;
    // The snapshot holds everything the live file does; keep it owner-only
    // instead of leaving it to the umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600)) {
            eprintln!(
                "warning: could not restrict backup {} to mode 0600: {e}",
                dest.display()
            );
        }
    }
    Ok(std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0))
}

/// Zero is returned only when the `schema_version` table does not exist. Every
/// other failure — BUSY, LOCKED, CORRUPT — propagates. Collapsing them into
/// "brand-new database" is what re-ran the destructive migrations over live data
/// for a day after the 2026-07-27 corruption.
fn read_version(conn: &Connection) -> Result<i32> {
    if !object_exists(conn, "table", "schema_version")? {
        return Ok(0);
    }
    let version = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

fn object_exists(conn: &Connection, kind: &str, name: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2",
            params![kind, name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
            params![table, column],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn set_schema_version(conn: &Connection, version: i32) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
        params![version],
    )?;
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    // Fast path: the database is already current, so take no lock, run no DDL,
    // not even CREATE TABLE IF NOT EXISTS. This is what keeps a per-tool-call
    // and per-HTTP-request open cheap.
    let current = read_version(conn)?;
    if current == SCHEMA_VERSION {
        return Ok(());
    }
    if current > SCHEMA_VERSION {
        return Err(DbError::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }

    // Slow path: take the write lock up front and re-read the version inside
    // the transaction. A second process blocks on busy_timeout instead of
    // racing, and wakes to find the work already done. DDL is transactional in
    // SQLite, so a failure anywhere in the chain — including inside the
    // destructive migrate_v4 — rolls back whole.
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);")?;

    let current = read_version(&tx)?;
    if current > SCHEMA_VERSION {
        return Err(DbError::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }

    if current < 1 {
        migrate_v1(&tx)?;
        set_schema_version(&tx, 1)?;
    }

    if current < 2 {
        migrate_v2(&tx)?;
        set_schema_version(&tx, 2)?;
    }

    if current < 3 {
        migrate_v3(&tx)?;
        set_schema_version(&tx, 3)?;
    }

    if current < 4 {
        migrate_v4(&tx)?;
        set_schema_version(&tx, 4)?;
    }

    if current < 5 {
        migrate_v5(&tx)?;
        set_schema_version(&tx, 5)?;
    }

    if current < 6 {
        migrate_v6(&tx)?;
        set_schema_version(&tx, 6)?;
    }

    if current < 7 {
        migrate_v7(&tx)?;
        set_schema_version(&tx, 7)?;
    }

    if current < 8 {
        migrate_v8(&tx)?;
        set_schema_version(&tx, 8)?;
    }

    if current < 9 {
        migrate_v9(&tx)?;
        set_schema_version(&tx, 9)?;
    }

    if current < 10 {
        migrate_v10(&tx)?;
        set_schema_version(&tx, 10)?;
    }

    if current < 11 {
        migrate_v11(&tx)?;
        set_schema_version(&tx, 11)?;
    }

    if current < 12 {
        migrate_v12(&tx)?;
        set_schema_version(&tx, 12)?;
    }

    if current < 13 {
        migrate_v13(&tx)?;
        set_schema_version(&tx, 13)?;
    }

    if current < 14 {
        migrate_v14(&tx)?;
        set_schema_version(&tx, 14)?;
    }

    if current < 15 {
        migrate_v15(&tx)?;
        set_schema_version(&tx, 15)?;
    }

    if current < 16 {
        migrate_v16(&tx)?;
        set_schema_version(&tx, 16)?;
    }

    if current < 17 {
        migrate_v17(&tx)?;
        set_schema_version(&tx, 17)?;
    }

    if current < 18 {
        migrate_v18(&tx)?;
        set_schema_version(&tx, 18)?;
    }

    tx.commit()?;
    Ok(())
}

/// V18 — очередь эмбеддинга (спека `011-dense-retrieval`, фаза D,
/// `data-model.md` §1a). До этой версии `au db reindex-embeddings` сама
/// грузила bge-m3 и считала вектора на месте — вторая копия весов рядом с
/// демоном, который уже держит свою. Теперь демон (`au daemon`) — единственный
/// держатель модели во всей системе: запись узла (`add_node_full`,
/// `crates/aurelius-core/src/graph/crud.rs`) и разовая доиндексация
/// (`db_reindex_embeddings_cli`, `crates/au/src/commands.rs`) только кладут
/// сюда `node_id` — дёшево, без модели и без вычислений; демон разбирает
/// очередь на каждом такте уже загруженной моделью.
///
/// `node_id` — сам PRIMARY KEY, не отдельный автоинкремент: повторная
/// постановка того же узла (например, правка до того, как демон успел его
/// разобрать) обязана лечь на ту же строку через `INSERT OR REPLACE`, а не
/// завести дубль. `attempts` растёт на ошибке инференса, не на успехе;
/// строка покидает очередь только ПОСЛЕ успешной записи вектора в
/// `node_embeddings`, обе операции — одной транзакцией: падение демона
/// посреди пачки не теряет и не задваивает уже обработанные узлы, а
/// необработанные остаются в очереди и подхватываются следующим тактом.
///
/// `ON DELETE CASCADE`: `PRAGMA foreign_keys=ON` (`db::open`) делает эту
/// ссылку настоящей, а не декоративной — `memory_gc` (`crates/aurelius/src/
/// mcp/handlers/crud.rs`) жёстко удаляет узлы-дубли по `content_hash`
/// напрямую через `DELETE FROM nodes`, и без каскада такое удаление узла, у
/// которого демон ещё не успел разобрать очередь, падало бы нарушением
/// внешнего ключа вместо того, чтобы просто унести за собой уже бессмысленную
/// строку очереди.
fn migrate_v18(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE embedding_queue (
            node_id    TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
            queued_at  TEXT NOT NULL,
            attempts   INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(())
}

/// V17 — переразметка узлов прогона со старой формы `NodeType::Custom("run")`
/// на новый вариант перечисления `NodeType::Run` (граф `aurelius-core`, живая
/// база держала ~2992 таких строк на 16.09.2026). В одной транзакции с
/// подъёмом версии схемы — иначе есть окно, где версия уже говорит «17», а
/// строки ещё в старой форме.
///
/// Сравнение через `json_extract`, а не через точное совпадение строки:
/// `node_type` — это сериализованный JSON (`{"custom":"run"}` для старой
/// формы), и `$.custom` не находится в скалярных значениях вроде `"project"`
/// — им путь `json_extract` вернёт `NULL`, они не тронуты.
fn migrate_v17(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE nodes SET node_type = '\"run\"'
          WHERE json_valid(node_type) AND json_extract(node_type, '$.custom') = 'run'",
        [],
    )?;
    Ok(())
}

/// V14 — у факта появляется предмет, о котором он утверждает.
///
/// Противоречие раньше не обнаруживалось никак: «выключено» и «включено» могли
/// лежать рядом, и граф не возражал. Ребро `supersedes` ставилось руками —
/// то есть по памяти, а что зависит от памяти, то не происходит.
///
/// Индекс частичный и по выражению — той же формы, что `idx_nodes_agent_session`
/// из V13: поля происхождения живут в `data`, а не колонками.
fn migrate_v14(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_nodes_subject
            ON nodes(json_extract(data, '$.subject'))
            WHERE json_extract(data, '$.subject') IS NOT NULL;
        ",
    )?;
    Ok(())
}

/// V15 — напоминанию не хватало собственной таблицы: срок был либо мыслью в
/// голове, либо необязательным полем задачи, но ни то, ни другое никогда
/// никому не напоминало.
///
/// Напоминание — это ЯВНО поставленный момент со своим состоянием и
/// аудируемым журналом переносов, а не производная от чего-то ещё:
/// `pending` вооружено и ждёт, `delivered` показано, но исход ещё не
/// наступил, а `done`/`cancelled` — два РАЗНЫХ исхода, и не сливать их в
/// один nullable timestamp — половина смысла этой таблицы.
///
/// `original_due_at` и `snooze_count` рядом с `due_at` и `reminder_events`
/// не избыточны: `original_due_at` держит момент, на который напоминание
/// было поставлено ПЕРВЫЙ раз, поэтому строка, перенесённая пять раз, всё
/// равно показывает, что обещала вначале; `snooze_count` — это дешёвое
/// чтение той же правды для одной строки списка, а `reminder_events` —
/// полный журнал для того, кто захочет посмотреть, откуда и когда именно.
/// Точно так же `state` и три временны́х метки исхода не дублируют друг
/// друга: `state` — то, по чему фильтрует запрос, метки — когда это
/// случилось.
fn migrate_v16(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE node_embeddings USING vec0(
            rowid INTEGER PRIMARY KEY,
            embedding INT8[1024]
        );",
    )?;
    Ok(())
}

fn migrate_v15(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS reminders (
            id              TEXT PRIMARY KEY,
            task_id         TEXT,
            project         TEXT,
            text            TEXT NOT NULL,
            owner           TEXT NOT NULL DEFAULT 'both',
            state           TEXT NOT NULL DEFAULT 'pending',
            due_at          INTEGER NOT NULL,
            original_due_at INTEGER NOT NULL,
            repeat_spec     TEXT,
            created_at      INTEGER NOT NULL,
            delivered_at    INTEGER,
            delivered_via   TEXT,
            delivered_count INTEGER NOT NULL DEFAULT 0,
            snooze_count    INTEGER NOT NULL DEFAULT 0,
            done_at         INTEGER,
            cancelled_at    INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_reminders_state_due ON reminders(state, due_at);
        CREATE INDEX IF NOT EXISTS idx_reminders_task ON reminders(task_id);
        CREATE TABLE IF NOT EXISTS reminder_events (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            reminder_id TEXT NOT NULL,
            at          INTEGER NOT NULL,
            kind        TEXT NOT NULL,
            detail      TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_reminder_events_rem ON reminder_events(reminder_id, at);
        ",
    )?;
    Ok(())
}

/// V13 — запись помечается прогоном, который её сделал.
///
/// Журнал не различал сессии вообще: `session_id` жил только в лабильном окне
/// отзыва, а сами узлы его не несли. Хук конца сессии видел все записи проекта
/// и не мог отделить свои сегодняшние от вчерашних — «собрать всё, что я
/// написал за этот прогон» было невозможно механически, только на глаз по
/// времени.
///
/// Метка легла в `data.agent_session`, а не отдельной колонкой: колонка
/// потребовала бы вручную править десяток рукописных списков `SELECT`, и
/// пропущенный список упал бы не при компиляции, а в рантайме на `row.get`.
/// Скорость колонки при этом сохраняется — индекс по выражению даёт ровно тот
/// же поиск по равенству.
///
/// Индекс частичный: непомеченные записи в него не попадают, поэтому он не
/// растёт на всей истории, накопленной до этой версии.
fn migrate_v13(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_nodes_agent_session
            ON nodes(json_extract(data, '$.agent_session'))
            WHERE json_extract(data, '$.agent_session') IS NOT NULL;
        ",
    )?;
    Ok(())
}

/// V12 — у обязательства появляется ЧИТАЕМЫЙ объект рядом с отпечатком, и
/// разово вычищается то, что успел завести intake без заслонки на речь.
///
/// Отпечаток `object_fp` — отсортированный мешок токенов, он для дедупа и FTS.
/// Показывать его человеку было ошибкой: слой «Давление» выдавал строки вида
/// «aurelius blyss force foreach item local path remove silentlycontinue».
/// Хуже того, половина этих строк вообще не должна была родиться — их завели
/// шелл-команды, лишь УПОМИНАВШИЕ обещание в своих аргументах.
///
/// Чистка опирается на `act_trace`: журнал append-only, исходный текст жив, и
/// его можно перепроверить нынешним детектором речи. Намеренно зовём боевой
/// `obligations::is_utterance`, а не замороженную копию: смысл шага — «применить
/// заслонку задним числом», и он выполняется ровно один раз на базу.
fn migrate_v12(conn: &Connection) -> Result<()> {
    // ADD COLUMN не идемпотентен — проверяем, как и degrade_stage в V11.
    let has_text: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('obligations') WHERE name = 'object_text'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !has_text {
        conn.execute(
            "ALTER TABLE obligations ADD COLUMN object_text TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

    let rows: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT o.id, t.payload FROM obligations o
               JOIN act_trace t ON t.id = o.src_trace",
        )?;
        let it = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        it.filter_map(std::result::Result::ok).collect()
    };
    for (id, payload) in &rows {
        match crate::obligations::readable_object(payload) {
            Some(text) => {
                conn.execute(
                    "UPDATE obligations SET object_text = ?1 WHERE id = ?2",
                    rusqlite::params![text, id],
                )?;
            }
            // Источник не был речью — обязательства не существовало.
            None => {
                conn.execute("DELETE FROM obligations WHERE id = ?1", [id])?;
            }
        }
    }
    // Строкам без следа-источника читаемого текста взять неоткуда: оставляем им
    // отпечаток, чтобы снапшот не показывал пустоту.
    conn.execute(
        "UPDATE obligations SET object_text = object_fp
          WHERE object_text = '' AND src_trace IS NULL",
        [],
    )?;
    // V11 завела только AFTER INSERT-триггер, delete-триггера у FTS нет —
    // после удаления строк индекс пересобираем целиком.
    conn.execute_batch("INSERT INTO obligations_fts(obligations_fts) VALUES('rebuild');")?;
    Ok(())
}

/// V11 — «Бит-и-Дело», волна 4: клиринг гроссбуха (ledger, render_log, calib),
/// проспективный контур обязательств (obligations, ob_postings,
/// counterparty_profile) и банкротство-поглощение (receivership,
/// degrade_stage на узле).
fn migrate_v11(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        -- Ступень 5. Двойная запись битового гроссбуха: единственная валюта
        -- ранжирования. Timestamps в ценность не входят — только измерения.
        CREATE TABLE IF NOT EXISTS ledger (
            id         INTEGER PRIMARY KEY,
            node_id    TEXT NOT NULL,
            session_id TEXT NOT NULL,
            bits_delta INTEGER NOT NULL,
            kind       TEXT NOT NULL CHECK(kind IN
                       ('earn','render_miss','discovery','yield_bonus','inversion_debit')),
            at         INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ledger_node ON ledger(node_id);

        -- Что было показано в снапшоте: с этого момента у показа есть цена.
        CREATE TABLE IF NOT EXISTS render_log (
            session_id TEXT NOT NULL,
            node_id    TEXT NOT NULL,
            layer      TEXT NOT NULL,
            bytes      INTEGER NOT NULL,
            cited      INTEGER NOT NULL DEFAULT 0,
            at         INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_render_session ON render_log(session_id);

        -- Калибровка оценщика рюкзака: α (биты) и β (исход) правятся по факту.
        CREATE TABLE IF NOT EXISTS calib (
            id         INTEGER PRIMARY KEY CHECK(id = 1),
            alpha      REAL NOT NULL,
            beta       REAL NOT NULL,
            updated_at INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO calib (id, alpha, beta, updated_at) VALUES (1, 1.0, 1.0, 0);

        -- Ступень 6. Обязательства как двойная бухгалтерия: существует ⇔
        -- проводки не сбалансированы. Не булев флаг, а аудируемый журнал.
        CREATE TABLE IF NOT EXISTS obligations (
            id            INTEGER PRIMARY KEY,
            debtor        TEXT NOT NULL,
            creditor      TEXT NOT NULL,
            verb_class    TEXT NOT NULL,
            object_fp     TEXT NOT NULL,
            opened_at     INTEGER NOT NULL,
            deadline      INTEGER,
            closed_at     INTEGER,
            src_trace     INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_ob_dedup
            ON obligations(debtor, creditor, object_fp) WHERE closed_at IS NULL;
        CREATE VIRTUAL TABLE IF NOT EXISTS obligations_fts USING fts5(
            object_fp, content='obligations', content_rowid='id'
        );
        CREATE TRIGGER IF NOT EXISTS obligations_ai AFTER INSERT ON obligations BEGIN
            INSERT INTO obligations_fts(rowid, object_fp) VALUES (new.id, new.object_fp);
        END;
        CREATE TABLE IF NOT EXISTS counterparty_profile (
            node        TEXT PRIMARY KEY,
            opened      INTEGER NOT NULL DEFAULT 0,
            closed      INTEGER NOT NULL DEFAULT 0,
            breach      INTEGER NOT NULL DEFAULT 0
        );

        -- Ступень 7. Банкротство-поглощение: кто чьи требования унаследовал.
        CREATE TABLE IF NOT EXISTS receivership (
            node_id     TEXT PRIMARY KEY,
            absorbed_by TEXT NOT NULL,
            at          INTEGER NOT NULL
        );
    ",
    )?;
    // degrade_stage: 0 полный текст, 1 выжимка, 2 tombstone. ALTER отдельно —
    // ADD COLUMN не идемпотентен, а миграция и так под одной версией.
    let has_stage: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('nodes') WHERE name = 'degrade_stage'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !has_stage {
        conn.execute(
            "ALTER TABLE nodes ADD COLUMN degrade_stage INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

/// V10 — «Бит-и-Дело», волны 2-3: словари кодека и дельта-счета (шлюз
/// сюрприза, NCS), ревизии узлов (единственный писатель контента — судья
/// исхода, история правок аудируема).
fn migrate_v10(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        -- Шлюз сюрприза: обученные zstd-словари ожиданий по scope.
        CREATE TABLE IF NOT EXISTS codec (
            dict_id INTEGER PRIMARY KEY,
            scope   TEXT NOT NULL,
            epoch   INTEGER NOT NULL DEFAULT 1,
            blob    BLOB NOT NULL,
            trained_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_codec_scope_epoch ON codec(scope, epoch);

        -- Дельта-счёт записи: сколько информации она внесла против ожидания.
        CREATE TABLE IF NOT EXISTS delta (
            id             INTEGER PRIMARY KEY,
            node_id        TEXT NOT NULL,
            scope          TEXT NOT NULL,
            raw_len        INTEGER NOT NULL,
            resid_len      INTEGER NOT NULL,
            surprisal_bits INTEGER NOT NULL,
            ncs            REAL NOT NULL,
            epoch_born     INTEGER NOT NULL,
            status         TEXT NOT NULL DEFAULT 'active'
                           CHECK(status IN ('active','assimilating','folded','inverted'))
        );
        CREATE INDEX IF NOT EXISTS idx_delta_node ON delta(node_id);

        -- Ревизии контента: правки только append-ом с причиной-окном.
        CREATE TABLE IF NOT EXISTS node_version (
            node_id             TEXT NOT NULL,
            rev                 INTEGER NOT NULL,
            content             TEXT NOT NULL,
            consolidation_level INTEGER NOT NULL DEFAULT 0,
            cause_window_id     INTEGER,
            created_at          INTEGER NOT NULL,
            PRIMARY KEY (node_id, rev)
        );
    ",
    )?;
    Ok(())
}

/// V9 — «Бит-и-Дело», волны 1-2 (specs/003-bit-i-delo): журнал следов действий
/// (append-only WAL с FTS), пробы против ground truth, пути извлечения,
/// лабильные окна recall-а и коррекции-первыми. Гроссбух битов, обязательства
/// и словари кодека приедут отдельными миграциями своих волн.
fn migrate_v9(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        -- Ступень 1. Сырые следы действий агента. Только INSERT: память строится
        -- из «что сделал и чем кончилось», задним числом факты не редактируются.
        CREATE TABLE IF NOT EXISTS act_trace (
            id             INTEGER PRIMARY KEY,
            ts             INTEGER NOT NULL,
            session_id     TEXT NOT NULL,
            kind           TEXT NOT NULL,
            payload        TEXT NOT NULL,
            exit_code      INTEGER,
            state_hash_pre TEXT,
            state_hash_post TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_act_trace_session ON act_trace(session_id, ts);
        CREATE TRIGGER IF NOT EXISTS act_trace_ro BEFORE UPDATE ON act_trace BEGIN
            SELECT RAISE(ABORT, 'act_trace is append-only');
        END;
        CREATE TRIGGER IF NOT EXISTS act_trace_nodel BEFORE DELETE ON act_trace BEGIN
            SELECT RAISE(ABORT, 'act_trace is append-only');
        END;
        CREATE VIRTUAL TABLE IF NOT EXISTS act_trace_fts USING fts5(
            payload, content='act_trace', content_rowid='id'
        );
        CREATE TRIGGER IF NOT EXISTS act_trace_ai AFTER INSERT ON act_trace BEGIN
            INSERT INTO act_trace_fts(rowid, payload) VALUES (new.id, new.payload);
        END;

        -- Ступень 2. Машинно-проверяемые пробы: память, чьи утверждения можно
        -- исполнить против ground truth (файл существует, SHA есть в git).
        CREATE TABLE IF NOT EXISTS probes (
            id         INTEGER PRIMARY KEY,
            node_id    TEXT NOT NULL,
            kind       TEXT NOT NULL CHECK(kind IN
                       ('file_exists','git_sha','cmd_in_path','table_in_schema')),
            expr       TEXT NOT NULL,
            last_ok    INTEGER,
            checked_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_probes_node ON probes(node_id);

        -- Ступень 3. Пути извлечения: доверие и забывание живут на паре
        -- «сигнатура запроса → узел», а не на узле целиком.
        CREATE TABLE IF NOT EXISTS pathways (
            query_sig TEXT NOT NULL,
            node_id   TEXT NOT NULL,
            confirms  INTEGER NOT NULL DEFAULT 0,
            misfires  INTEGER NOT NULL DEFAULT 0,
            blocked   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (query_sig, node_id)
        );

        -- Ступень 3. Лабильное окно: recall открывает окно, следы сессии
        -- атрибутируются к нему, вердикт выносится при закрытии (ступень 4).
        CREATE TABLE IF NOT EXISTS labile_window (
            id            INTEGER PRIMARY KEY,
            node_id       TEXT NOT NULL,
            session_id    TEXT NOT NULL,
            snapshot_hash TEXT NOT NULL,
            opened_at     INTEGER NOT NULL,
            closed_at     INTEGER,
            verdict       TEXT CHECK(verdict IN ('reinforce','erode','fork','null'))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_lw_one_open
            ON labile_window(node_id, session_id) WHERE closed_at IS NULL;
        CREATE TABLE IF NOT EXISTS trace_attribution (
            window_id     INTEGER NOT NULL,
            trace_id      INTEGER NOT NULL,
            overlap_score REAL NOT NULL,
            PRIMARY KEY (window_id, trace_id)
        );

        -- Ступень 3. Коррекции: забывание — активная подача поправки ПЕРЕД
        -- результатами поиска, а не дыра в выдаче.
        CREATE TABLE IF NOT EXISTS corrections (
            id             INTEGER PRIMARY KEY,
            fts_pattern    TEXT NOT NULL,
            dead_node_id   TEXT NOT NULL,
            replacement_id TEXT,
            reason         TEXT NOT NULL,
            minted_at      INTEGER NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS corrections_fts USING fts5(
            fts_pattern, reason, content='corrections', content_rowid='id'
        );
        CREATE TRIGGER IF NOT EXISTS corrections_ai AFTER INSERT ON corrections BEGIN
            INSERT INTO corrections_fts(rowid, fts_pattern, reason)
            VALUES (new.id, new.fts_pattern, new.reason);
        END;
    ",
    )?;
    Ok(())
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS nodes (
            id          TEXT PRIMARY KEY,
            node_type   TEXT NOT NULL,
            label       TEXT NOT NULL,
            note        TEXT,
            source      TEXT NOT NULL DEFAULT 'manual',
            data        TEXT NOT NULL DEFAULT '{}',
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS edges (
            id          TEXT PRIMARY KEY,
            from_id     TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            to_id       TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            relation    TEXT NOT NULL,
            weight      REAL NOT NULL DEFAULT 1.0,
            created_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
        CREATE INDEX IF NOT EXISTS idx_edges_to   ON edges(to_id);

        CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
            label,
            note,
            data,
            content='nodes',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS nodes_ai AFTER INSERT ON nodes BEGIN
            INSERT INTO nodes_fts(rowid, label, note, data)
            VALUES (new.rowid, new.label, new.note, new.data);
        END;

        CREATE TRIGGER IF NOT EXISTS nodes_ad AFTER DELETE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note, data)
            VALUES ('delete', old.rowid, old.label, old.note, old.data);
        END;

        CREATE TRIGGER IF NOT EXISTS nodes_au AFTER UPDATE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note, data)
            VALUES ('delete', old.rowid, old.label, old.note, old.data);
            INSERT INTO nodes_fts(rowid, label, note, data)
            VALUES (new.rowid, new.label, new.note, new.data);
        END;
    ",
    )?;
    Ok(())
}

fn migrate_v2(conn: &Connection) -> Result<()> {
    // SQLite has no ALTER TABLE ... ADD COLUMN IF NOT EXISTS, so the column is
    // checked structurally. Matching the English text of an error message would
    // break silently the day the engine rewords it.
    let columns = [
        (
            "memory_kind",
            "ALTER TABLE nodes ADD COLUMN memory_kind TEXT NOT NULL DEFAULT 'semantic'",
        ),
        (
            "last_accessed_at",
            "ALTER TABLE nodes ADD COLUMN last_accessed_at TEXT",
        ),
        (
            "access_count",
            "ALTER TABLE nodes ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "content_hash",
            "ALTER TABLE nodes ADD COLUMN content_hash TEXT",
        ),
    ];
    for (name, sql) in columns {
        if !column_exists(conn, "nodes", name)? {
            conn.execute(sql, [])?;
        }
    }

    // Backfill last_accessed_at from updated_at where NULL
    conn.execute(
        "UPDATE nodes SET last_accessed_at = updated_at WHERE last_accessed_at IS NULL",
        [],
    )?;

    Ok(())
}

fn migrate_v4(conn: &Connection) -> Result<()> {
    // Rebuild FTS5 without the `data` column — raw JSON creates search noise
    conn.execute_batch(
        "
        DROP TRIGGER IF EXISTS nodes_ai;
        DROP TRIGGER IF EXISTS nodes_ad;
        DROP TRIGGER IF EXISTS nodes_au;
        DROP TABLE IF EXISTS nodes_fts;

        CREATE VIRTUAL TABLE nodes_fts USING fts5(
            label, note,
            content='nodes',
            content_rowid='rowid'
        );

        CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN
            INSERT INTO nodes_fts(rowid, label, note)
            VALUES (new.rowid, new.label, new.note);
        END;

        CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note)
            VALUES ('delete', old.rowid, old.label, old.note);
        END;

        CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note)
            VALUES ('delete', old.rowid, old.label, old.note);
            INSERT INTO nodes_fts(rowid, label, note)
            VALUES (new.rowid, new.label, new.note);
        END;

        INSERT INTO nodes_fts(rowid, label, note)
        SELECT rowid, label, note FROM nodes;
    ",
    )?;
    Ok(())
}

fn migrate_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS search_cache (
            id          TEXT PRIMARY KEY,
            query       TEXT NOT NULL,
            results     TEXT NOT NULL DEFAULT '[]',
            source      TEXT NOT NULL DEFAULT 'brave',
            created_at  TEXT NOT NULL,
            expires_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_search_cache_query
            ON search_cache(query);

        CREATE INDEX IF NOT EXISTS idx_search_cache_expires
            ON search_cache(expires_at);

        CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
            query, results,
            content='search_cache',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS search_cache_ai AFTER INSERT ON search_cache BEGIN
            INSERT INTO search_fts(rowid, query, results)
            VALUES (new.rowid, new.query, new.results);
        END;

        CREATE TRIGGER IF NOT EXISTS search_cache_ad AFTER DELETE ON search_cache BEGIN
            INSERT INTO search_fts(search_fts, rowid, query, results)
            VALUES ('delete', old.rowid, old.query, old.results);
        END;

        CREATE TRIGGER IF NOT EXISTS search_cache_au AFTER UPDATE ON search_cache BEGIN
            INSERT INTO search_fts(search_fts, rowid, query, results)
            VALUES ('delete', old.rowid, old.query, old.results);
            INSERT INTO search_fts(rowid, query, results)
            VALUES (new.rowid, new.query, new.results);
        END;
    ",
    )?;
    Ok(())
}

fn migrate_v6(conn: &Connection) -> Result<()> {
    // Sync attribution/tombstone/cursor columns on nodes and edges.
    let node_columns = [
        "ALTER TABLE nodes ADD COLUMN created_by TEXT",
        "ALTER TABLE nodes ADD COLUMN updated_by TEXT",
        "ALTER TABLE nodes ADD COLUMN deleted_at TEXT",
        "ALTER TABLE nodes ADD COLUMN sync_seq INTEGER",
    ];
    for sql in &node_columns {
        // ALTER TABLE ADD COLUMN IF NOT EXISTS not supported in SQLite,
        // so we silently ignore "duplicate column" errors
        match conn.execute(sql, []) {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => return Err(e.into()),
        }
    }

    let edge_columns = [
        "ALTER TABLE edges ADD COLUMN created_by TEXT",
        "ALTER TABLE edges ADD COLUMN deleted_at TEXT",
        "ALTER TABLE edges ADD COLUMN sync_seq INTEGER",
    ];
    for sql in &edge_columns {
        match conn.execute(sql, []) {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => return Err(e.into()),
        }
    }

    conn.execute_batch(
        "
        -- Client-side, one row per project: sync opt-in and cursor bookkeeping.
        CREATE TABLE IF NOT EXISTS sync_config (
            project_label   TEXT PRIMARY KEY,
            server_url      TEXT NOT NULL,
            token           TEXT NOT NULL,
            enabled         BOOLEAN NOT NULL DEFAULT 0,
            last_seq        INTEGER NOT NULL DEFAULT 0,
            updated_at      TEXT NOT NULL
        );

        -- Server-side, one row per issued collaborator token. Looked up by
        -- token_hash (sha256 of the plaintext token) -- the plaintext itself
        -- is never stored server-side, only shown once at issuance.
        CREATE TABLE IF NOT EXISTS collaborator_grants (
            token_hash      TEXT PRIMARY KEY,
            person_name     TEXT NOT NULL,
            person_email    TEXT NOT NULL,
            project_label   TEXT NOT NULL,
            granted_at      TEXT NOT NULL,
            revoked_at      TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_collaborator_grants_project
            ON collaborator_grants(project_label);

        CREATE INDEX IF NOT EXISTS idx_nodes_sync_seq ON nodes(sync_seq);
        CREATE INDEX IF NOT EXISTS idx_edges_sync_seq ON edges(sync_seq);
        CREATE INDEX IF NOT EXISTS idx_nodes_deleted_at ON nodes(deleted_at);
        ",
    )?;
    Ok(())
}

/// Client-side, one row per sync server this machine administers. Lets
/// `au share issue`/`au share revoke` resolve the admin token from a prior
/// `au share admin-set <server> <token>` instead of requiring
/// AURELIUS_SYNC_ADMIN_TOKEN to be re-exported every session.
fn migrate_v7(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS admin_tokens (
            server_url  TEXT PRIMARY KEY,
            token       TEXT NOT NULL,
            saved_at    TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

/// Converted-document cache. Keyed by the SHA-256 of the *file contents* rather
/// than its path, so a copied or renamed document is recognised as the one
/// already converted. The FTS mirror is what makes a document read months ago
/// findable without the original file.
fn migrate_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS doc_cache (
            sha256      TEXT PRIMARY KEY,
            source_path TEXT NOT NULL,
            file_name   TEXT NOT NULL,
            format      TEXT NOT NULL,
            markdown    TEXT NOT NULL,
            char_count  INTEGER NOT NULL,
            byte_size   INTEGER NOT NULL,
            spill_path  TEXT,
            created_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_doc_cache_path
            ON doc_cache(source_path);

        CREATE INDEX IF NOT EXISTS idx_doc_cache_created
            ON doc_cache(created_at);

        CREATE VIRTUAL TABLE IF NOT EXISTS doc_fts USING fts5(
            file_name, markdown,
            content='doc_cache',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS doc_cache_ai AFTER INSERT ON doc_cache BEGIN
            INSERT INTO doc_fts(rowid, file_name, markdown)
            VALUES (new.rowid, new.file_name, new.markdown);
        END;

        CREATE TRIGGER IF NOT EXISTS doc_cache_ad AFTER DELETE ON doc_cache BEGIN
            INSERT INTO doc_fts(doc_fts, rowid, file_name, markdown)
            VALUES ('delete', old.rowid, old.file_name, old.markdown);
        END;

        CREATE TRIGGER IF NOT EXISTS doc_cache_au AFTER UPDATE ON doc_cache BEGIN
            INSERT INTO doc_fts(doc_fts, rowid, file_name, markdown)
            VALUES ('delete', old.rowid, old.file_name, old.markdown);
            INSERT INTO doc_fts(rowid, file_name, markdown)
            VALUES (new.rowid, new.file_name, new.markdown);
        END;
    ",
    )?;
    Ok(())
}

fn migrate_v3(conn: &Connection) -> Result<()> {
    // Clean up duplicate edges BEFORE creating unique index
    conn.execute(
        "DELETE FROM edges WHERE id NOT IN (
            SELECT MIN(id) FROM edges GROUP BY from_id, to_id, relation
        )",
        [],
    )?;

    conn.execute_batch(
        "
        -- Edge dedup: prevent duplicate (from, to, relation) triples
        CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_unique
            ON edges(from_id, to_id, relation);

        -- Fast unsolved problems query
        CREATE INDEX IF NOT EXISTS idx_edges_to_relation
            ON edges(to_id, relation);

        -- Content hash lookup for dedup
        CREATE INDEX IF NOT EXISTS idx_nodes_content_hash
            ON nodes(content_hash) WHERE content_hash IS NOT NULL;

        -- Project-scoped queries by type
        CREATE INDEX IF NOT EXISTS idx_nodes_type_created
            ON nodes(node_type, created_at DESC);

        -- Source filtering (e.g. find all mcp-session nodes)
        CREATE INDEX IF NOT EXISTS idx_nodes_source
            ON nodes(source);
    ",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::sync::{Arc, Barrier};

    /// Temp database that cleans up its whole WAL triple on drop.
    struct TmpDb(PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("aurelius-test-{tag}-{}.db", uuid::Uuid::new_v4())),
            )
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(sidecar(&self.0, suffix));
            }
        }
    }

    fn insert_node(conn: &Connection, id: &str, label: &str) {
        conn.execute(
            "INSERT INTO nodes (id, node_type, label, note, source, data, created_at, updated_at)
             VALUES (?1,'concept',?2,'note','test','{}','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            params![id, label],
        )
        .expect("insert node");
    }

    fn stored_version(conn: &Connection) -> i32 {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .expect("read version")
    }

    fn fts_hits(conn: &Connection, term: &str) -> i64 {
        conn.query_row(
            "SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH ?1",
            params![term],
            |row| row.get(0),
        )
        .expect("fts match")
    }

    fn digest(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let mut file = std::fs::File::open(path).expect("open for hashing");
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).expect("read for hashing");
        format!("{:x}", Sha256::digest(&buf))
    }

    /// Regression guard: a fresh database reaches the current schema, and a
    /// second open is a no-op.
    #[test]
    fn fresh_open_migrates_and_is_idempotent() {
        let tmp = TmpDb::new("fresh");
        {
            let conn = open(tmp.path()).expect("initial open");
            assert_eq!(stored_version(&conn), SCHEMA_VERSION);
            assert!(
                object_exists(&conn, "table", "nodes_fts").expect("lookup"),
                "nodes_fts must exist after migration"
            );
        }
        let conn = open(tmp.path()).expect("second open");
        assert_eq!(stored_version(&conn), SCHEMA_VERSION);
    }

    /// A migration that fails partway through must leave nothing behind — not
    /// the destructive work of `migrate_v4`, not an advanced version marker.
    #[test]
    fn failed_migration_rolls_back_migrate_v4() {
        let tmp = TmpDb::new("rollback");
        {
            let conn = open(tmp.path()).expect("initial open");
            insert_node(&conn, "n1", "alpha");
            // Empty the FTS index so migrate_v4's full reindex is observable.
            conn.execute("INSERT INTO nodes_fts(nodes_fts) VALUES('delete-all')", [])
                .expect("clear fts");
            assert_eq!(fts_hits(&conn, "alpha"), 0);

            // Make the next open believe v4 and v5 are pending …
            conn.execute("DELETE FROM schema_version WHERE version >= 4", [])
                .expect("reset version");
            // … and poison migrate_v5, which runs after the destructive v4:
            // its `CREATE INDEX IF NOT EXISTS idx_search_cache_query` collides
            // with a table of that name (IF NOT EXISTS does not cover a
            // different object kind).
            conn.execute_batch(
                "DROP INDEX idx_search_cache_query;
                 CREATE TABLE idx_search_cache_query (x);",
            )
            .expect("poison v5");
        }

        open(tmp.path()).expect_err("migrate_v5 must fail");

        let conn = Connection::open(tmp.path()).expect("raw open");
        assert_eq!(
            stored_version(&conn),
            3,
            "version advanced even though the migration failed"
        );
        assert_eq!(
            fts_hits(&conn, "alpha"),
            0,
            "migrate_v4 committed its reindex despite the migration failing"
        );
    }

    /// A damaged image must be refused, the refusal must be actionable, and it
    /// must not write to the file.
    #[test]
    fn corrupt_header_is_detected_at_open() {
        let tmp = TmpDb::new("corrupt");
        {
            let conn = open(tmp.path()).expect("initial open");
            for i in 0..500 {
                insert_node(&conn, &format!("n{i}"), &format!("label {i}"));
            }
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .expect("checkpoint");
        }
        // Patch the header's page count (bytes 28..32, big endian) so the file
        // is larger than it declares — the signature of the 2026-07-27 incident.
        {
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(tmp.path())
                .expect("open file");
            f.seek(SeekFrom::Start(28)).expect("seek");
            f.write_all(&3u32.to_be_bytes()).expect("patch page_count");
            f.sync_all().expect("sync");
        }

        let before = digest(tmp.path());
        let err = open(tmp.path()).expect_err("a damaged image must be refused");
        assert!(
            matches!(err, DbError::Corrupt { .. }),
            "expected DbError::Corrupt, got: {err}"
        );
        let message = err.to_string();
        assert!(
            message.contains("au db backup"),
            "the corruption error must tell the user what to do next, got: {message}"
        );
        assert!(
            message.contains("SQLite confirms"),
            "the refusal must come from the health gate with SQLite's verdict, got: {message}"
        );
        assert_eq!(
            digest(tmp.path()),
            before,
            "refusing a damaged database must not modify it"
        );

        let report = check(tmp.path(), false).expect("check runs on a damaged file");
        assert!(!report.ok);
        assert!(
            report.problems.iter().any(|p| p.contains("past the end")),
            "check must name the file-larger-than-header signature: {:?}",
            report.problems
        );
    }

    fn header_u32(path: &Path, offset: u64) -> u32 {
        let mut f = std::fs::File::open(path).expect("open for header");
        f.seek(SeekFrom::Start(offset)).expect("seek header");
        let mut bytes = [0u8; 4];
        f.read_exact(&mut bytes).expect("read header");
        u32::from_be_bytes(bytes)
    }

    /// The 2026-09-13 / 2026-09-15 lock-out, produced by SQLite's own
    /// machinery: a PASSIVE checkpoint held back by a reader copies the pages
    /// that grew the file but not the newer page 1, so the file outgrows its
    /// authoritative-looking header while the newest header sits in the WAL.
    /// Healthy — must open, and `au db check` must agree.
    #[test]
    fn partial_checkpoint_longer_than_header_is_healthy() {
        let tmp = TmpDb::new("partial-ckpt");
        let writer = open(tmp.path()).expect("initial open");
        writer
            .execute_batch(
                "PRAGMA wal_autocheckpoint=0;
                 CREATE TABLE bulk (b BLOB);
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .expect("setup");
        // Transaction 1 grows the file: overflow pages at the end, and page 1
        // records the new page count.
        writer
            .execute("INSERT INTO bulk VALUES (zeroblob(262144))", [])
            .expect("grow");
        // A reader pins the snapshot that ends with transaction 1.
        let reader = open_readonly(tmp.path()).expect("reader");
        reader.execute_batch("BEGIN").expect("begin read");
        let rows: i64 = reader
            .query_row("SELECT count(*) FROM bulk", [], |row| row.get(0))
            .expect("pin snapshot");
        assert_eq!(rows, 1);
        // Transaction 2 rewrites page 1 past the reader's snapshot, then the
        // checkpoint can backfill only up to that snapshot.
        writer
            .execute_batch("CREATE TABLE later (x); PRAGMA wal_checkpoint(PASSIVE);")
            .expect("touch page 1 and checkpoint");

        let g = geometry(tmp.path());
        let declared =
            u64::from(header_u32(tmp.path(), 28)) * u64::try_from(g.page_size).expect("page size");
        assert_eq!(
            header_u32(tmp.path(), 24),
            header_u32(tmp.path(), 92),
            "precondition: the header page count must look authoritative"
        );
        assert!(
            g.file_bytes > declared && g.wal_bytes > WAL_HEADER_BYTES,
            "precondition: file must outgrow its header with a live WAL, got {} bytes vs {declared}, wal {}",
            g.file_bytes,
            g.wal_bytes
        );
        assert!(
            g.problems.is_empty(),
            "a partially checkpointed database is not damage: {:?}",
            g.problems
        );

        let conn = open(tmp.path()).expect("a partially checkpointed database must open");
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM bulk", [], |row| row.get(0))
            .expect("read through the WAL");
        assert_eq!(rows, 1);
        let report = check(tmp.path(), false).expect("check runs");
        assert!(report.ok, "check must agree: {:?}", report.problems);
        drop(reader);
    }

    /// A tail past the header with no WAL to explain it is still reported, but
    /// SQLite ignores bytes past the declared end, so the open degrades to a
    /// warning instead of locking every caller out.
    #[test]
    fn tail_without_wal_opens_when_sqlite_reads_it_intact() {
        let tmp = TmpDb::new("tail");
        {
            let conn = open(tmp.path()).expect("initial open");
            insert_node(&conn, "n1", "alpha");
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .expect("checkpoint");
        }
        {
            let page = usize::try_from(geometry(tmp.path()).page_size).expect("page size");
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(tmp.path())
                .expect("open file");
            f.write_all(&vec![0u8; 8 * page]).expect("append tail");
            f.sync_all().expect("sync");
        }
        assert!(
            geometry(tmp.path())
                .problems
                .iter()
                .any(|p| p.contains("past the end")),
            "precondition: geometry must flag the tail"
        );

        let conn = open(tmp.path()).expect("SQLite reads it intact, so open must not refuse");
        let found: i64 = conn
            .query_row("SELECT count(*) FROM nodes WHERE id = 'n1'", [], |row| {
                row.get(0)
            })
            .expect("read");
        assert_eq!(found, 1);
    }

    /// Contention must wait, not fail. Has a timing component by nature.
    #[test]
    fn concurrent_opens_all_succeed() {
        let tmp = TmpDb::new("race");
        let path = tmp.path().to_path_buf();
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (p, b) = (path.clone(), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    b.wait();
                    open(&p).map(|_| ())
                })
            })
            .collect();
        for handle in handles {
            let result = handle.join().expect("thread panicked");
            assert!(result.is_ok(), "concurrent open failed: {result:?}");
        }
        let conn = open(&path).expect("final open");
        assert_eq!(stored_version(&conn), SCHEMA_VERSION);
    }

    /// Never write to a database a newer binary produced.
    #[test]
    fn schema_newer_than_binary_is_rejected() {
        let tmp = TmpDb::new("newer");
        {
            let conn = open(tmp.path()).expect("initial open");
            conn.execute("INSERT INTO schema_version (version) VALUES (99)", [])
                .expect("write future version");
        }
        let err = open(tmp.path()).expect_err("a newer schema must be refused");
        assert!(
            matches!(
                err,
                DbError::SchemaTooNew {
                    found: 99,
                    supported: SCHEMA_VERSION
                }
            ),
            "expected SchemaTooNew, got: {err}"
        );
    }

    /// A snapshot must include rows that are still only in the -wal.
    #[test]
    fn backup_captures_uncheckpointed_wal() {
        let tmp = TmpDb::new("backup");
        let dest = TmpDb::new("backup-dest");
        let conn = open(tmp.path()).expect("initial open");
        for i in 0..200 {
            insert_node(&conn, &format!("n{i}"), &format!("label {i}"));
        }
        // Deliberately no checkpoint: the rows live in the -wal.
        let bytes = backup_into(tmp.path(), dest.path()).expect("backup");
        assert!(bytes > 0);

        let copy = Connection::open(dest.path()).expect("open snapshot");
        let count: i64 = copy
            .query_row("SELECT count(*) FROM nodes", [], |row| row.get(0))
            .expect("count in snapshot");
        assert_eq!(count, 200, "snapshot lost rows still in the -wal");
        let report = check(dest.path(), true).expect("check snapshot");
        assert!(report.ok, "snapshot is not clean: {:?}", report.problems);
    }

    /// A backup carries the same data as the live file, so it gets owner-only mode.
    #[cfg(unix)]
    #[test]
    fn backup_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TmpDb::new("backup-mode");
        let dest = TmpDb::new("backup-mode-dest");
        drop(open(tmp.path()).expect("initial open"));
        backup_into(tmp.path(), dest.path()).expect("backup");
        let mode = std::fs::metadata(dest.path())
            .expect("stat backup")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "backup mode is {:o}", mode & 0o777);
    }
}
