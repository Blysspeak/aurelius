use rusqlite::{
    params, Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Highest schema version this binary understands.
pub const SCHEMA_VERSION: i32 = 11;

/// How long a connection waits for a lock another process holds. Long enough to
/// absorb a checkpoint or a migration, short enough that a genuinely stuck lock
/// surfaces instead of hanging an editor hook.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(
        "database image is damaged: {path}\n  {detail}\n  \
         hint: `au db check --full` for the full report, `au db backup` to snapshot what is \
         still readable.\n  \
         never copy or restore aurelius.db with cp/mv/rsync while `au` or an MCP server is \
         running — use `au db backup`"
    )]
    Corrupt { path: String, detail: String },

    #[error("database schema is v{found}, this binary supports v{supported} — upgrade `au`")]
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

pub fn open(path: &Path) -> Result<Connection> {
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
fn open_readonly(path: &Path) -> Result<Connection> {
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
/// `page_size * page_count` is the LOGICAL size seen through the WAL, so while
/// a `-wal` is live it can legitimately exceed the main file. The reverse — a
/// file larger than its own header describes — never happens legitimately, and
/// is the fingerprint of a file-level copy over a live WAL database (the
/// 2026-07-27 incident file: 7 294 976 bytes against 181 pages x 4096).
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

    let logical = u64::try_from(page_size.saturating_mul(page_count)).unwrap_or(0);
    if header_size_authoritative && logical > 0 {
        if file_bytes > logical {
            problems.push(format!(
                "file is {file_bytes} bytes but the header describes only {page_count} pages of \
                 {page_size} ({logical} bytes) — {} bytes lie past the end of the declared \
                 database; this is the signature of a file-level copy over a live WAL database",
                file_bytes - logical
            ));
        } else if file_bytes < logical && wal_bytes == 0 {
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
fn verify(path: &Path) -> Result<()> {
    let geometry = geometry(path);
    if geometry.problems.is_empty() {
        return Ok(());
    }
    Err(DbError::Corrupt {
        path: path.display().to_string(),
        detail: geometry.problems.join("\n  "),
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

    // Per table rather than whole-database: the whole-database form also
    // validates FTS5 inverted indexes, which needs write access, so on a
    // read-only connection it reports "attempt to write a readonly database"
    // for every healthy database. Checking ordinary tables individually covers
    // the same page-level b-tree integrity without that false positive.
    let verb = if full {
        "integrity_check"
    } else {
        "quick_check"
    };
    match checkable_tables(&conn) {
        Ok(tables) => {
            for table in tables {
                let sql = format!("PRAGMA {verb}('{}')", table.replace('\'', "''"));
                match conn.query_row(&sql, [], |row| row.get::<_, String>(0)) {
                    Ok(report) if report.eq_ignore_ascii_case("ok") => {}
                    Ok(report) => problems.push(format!("{table}: {report}")),
                    // The engine bails out mid-scan on a badly damaged file.
                    // That is a finding to report, not a reason to crash.
                    Err(e) => problems.push(format!("{table}: {e}")),
                }
                // Quick mode answers "is it damaged", not "how much" — stop at
                // the first table with a finding. `--full` reports everything.
                if !full && !problems.is_empty() {
                    break;
                }
            }
        }
        Err(e) => problems.push(format!("cannot enumerate tables: {e}")),
    }

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

    tx.commit()?;
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
}
