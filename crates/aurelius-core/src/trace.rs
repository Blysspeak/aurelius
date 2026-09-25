//! Ступень 1 «Бит-и-Дело»: журнал следов действий (specs/003-bit-i-delo).
//!
//! Единственная точка входа конвейера памяти v2. Каждый след — сырой факт
//! «что агент сделал и чем это кончилось», без интерпретации: вид, полезная
//! нагрузка, код возврата, хэш затронутого состояния до/после. Таблица
//! append-only на уровне триггеров БД — задним числом историю не правят.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Виды следов. Строгий список: неизвестный вид — ошибка вызывающего,
/// а не новая строка-опечатка в журнале.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceKind {
    ToolCall,
    FileEdit,
    Error,
    Commit,
    MsgSent,
    UserCorrection,
}

impl TraceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TraceKind::ToolCall => "tool_call",
            TraceKind::FileEdit => "file_edit",
            TraceKind::Error => "error",
            TraceKind::Commit => "commit",
            TraceKind::MsgSent => "msg_sent",
            TraceKind::UserCorrection => "user_correction",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tool_call" => Some(Self::ToolCall),
            "file_edit" => Some(Self::FileEdit),
            "error" => Some(Self::Error),
            "commit" => Some(Self::Commit),
            "msg_sent" => Some(Self::MsgSent),
            "user_correction" => Some(Self::UserCorrection),
            _ => None,
        }
    }
}

/// Payload обрезается до этого размера: журнал — сигнал для атрибуции и FTS,
/// а не архив содержимого (полные тексты живут в своих таблицах).
const PAYLOAD_CAP: usize = 2_000;

pub struct TraceInput<'a> {
    pub session_id: &'a str,
    pub kind: TraceKind,
    pub payload: &'a str,
    pub exit_code: Option<i64>,
    pub state_hash_pre: Option<String>,
    pub state_hash_post: Option<String>,
}

/// Записать след. Возвращает id строки журнала.
pub fn ingest(conn: &Connection, t: &TraceInput<'_>) -> Result<i64> {
    // Mask secrets before the insert: the AFTER INSERT trigger copies the
    // payload into act_trace_fts, so neither table ever sees the raw value.
    let capped: String = t.payload.chars().take(PAYLOAD_CAP).collect();
    let payload = crate::secret::mask_secrets(&capped);
    conn.execute(
        "INSERT INTO act_trace
             (ts, session_id, kind, payload, exit_code, state_hash_pre, state_hash_post)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            Utc::now().timestamp(),
            t.session_id,
            t.kind.as_str(),
            payload,
            t.exit_code,
            t.state_hash_pre,
            t.state_hash_post,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Rewrite already stored payloads through [`crate::secret::mask_secrets`] in
/// one IMMEDIATE transaction; returns how many rows changed. Never wired to the CLI yet.
///
/// `act_trace` is append-only via the `act_trace_ro` trigger, so the trigger is
/// dropped and recreated from its own `sqlite_master` text inside the same
/// transaction (a failure rolls the drop back). `act_trace_fts` is an
/// external-content FTS5 index fed only by an AFTER INSERT trigger, so each
/// changed row gets an explicit FTS 'delete' of the old text and an insert of
/// the new one; otherwise the old tokens would stay searchable.
pub fn scrub_existing(conn: &Connection) -> Result<usize> {
    scrub_existing_with(conn, || {})
}

/// Body of [`scrub_existing`]; `after_read` runs between the read and the
/// writes so tests can race a second connection against the scrub.
fn scrub_existing_with(conn: &Connection, after_read: impl FnOnce()) -> Result<usize> {
    // IMMEDIATE takes the write lock before the read: a deferred transaction
    // that reads first fails with SQLITE_BUSY_SNAPSHOT (517) once another
    // connection writes in between.
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let changed: Vec<(i64, String, String)> = {
        let mut stmt = tx.prepare("SELECT id, payload FROM act_trace")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, old) = row?;
            let new = crate::secret::mask_secrets(&old);
            if new != old {
                out.push((id, old, new));
            }
        }
        out
    };
    after_read();
    if changed.is_empty() {
        return Ok(0);
    }
    let trigger_sql = trigger_sql(&tx, "act_trace_ro");
    tx.execute_batch("DROP TRIGGER IF EXISTS act_trace_ro")?;
    for (id, old, new) in &changed {
        tx.execute(
            "INSERT INTO act_trace_fts(act_trace_fts, rowid, payload) VALUES ('delete', ?1, ?2)",
            rusqlite::params![id, old],
        )?;
        tx.execute(
            "UPDATE act_trace SET payload = ?2 WHERE id = ?1",
            rusqlite::params![id, new],
        )?;
        tx.execute(
            "INSERT INTO act_trace_fts(rowid, payload) VALUES (?1, ?2)",
            rusqlite::params![id, new],
        )?;
    }
    if let Some(sql) = trigger_sql {
        tx.execute_batch(&sql)?;
    }
    tx.commit()?;
    Ok(changed.len())
}

/// Stored `CREATE TRIGGER` text of `name`, if the trigger exists.
fn trigger_sql(conn: &Connection, name: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
        [name],
        |r| r.get(0),
    )
    .ok()
}

/// Retention capability: rows of `act_trace` with `ts` older than
/// `cutoff_unix_seconds`. With `apply` false only counts them. With `apply`
/// true removes them, their `act_trace_fts` entries and their
/// `trace_attribution` rows in one IMMEDIATE transaction, lifting the
/// `act_trace_nodel` guard and recreating it from its `sqlite_master` text.
/// Returns the number of trace rows matched. Not wired to the CLI yet.
pub fn prune_trace_before(
    conn: &Connection,
    cutoff_unix_seconds: i64,
    apply: bool,
) -> Result<usize> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM act_trace WHERE ts < ?1",
        [cutoff_unix_seconds],
        |r| r.get(0),
    )?;
    let count = usize::try_from(count)?;
    if !apply || count == 0 {
        return Ok(count);
    }
    let nodel_sql = trigger_sql(&tx, "act_trace_nodel");
    tx.execute_batch("DROP TRIGGER IF EXISTS act_trace_nodel")?;
    tx.execute(
        "DELETE FROM trace_attribution
         WHERE trace_id IN (SELECT id FROM act_trace WHERE ts < ?1)",
        [cutoff_unix_seconds],
    )?;
    // External-content FTS5 needs the old text to drop its tokens.
    tx.execute(
        "INSERT INTO act_trace_fts(act_trace_fts, rowid, payload)
         SELECT 'delete', id, payload FROM act_trace WHERE ts < ?1",
        [cutoff_unix_seconds],
    )?;
    tx.execute("DELETE FROM act_trace WHERE ts < ?1", [cutoff_unix_seconds])?;
    if let Some(sql) = nodel_sql {
        tx.execute_batch(&sql)?;
    }
    tx.commit()?;
    Ok(count)
}

/// Хэш состояния файла для пары pre/post. Отсутствующий файл — тоже состояние
/// (след «файл удалён» должен отличаться от «файл пуст»).
pub fn file_state_hash(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut h = Sha256::new();
            h.update(&bytes);
            format!("{:x}", h.finalize())
        }
        Err(_) => "absent".to_owned(),
    }
}

/// Сколько следов накопила сессия (для отчётов и порогов клиринга).
pub fn count_for_session(conn: &Connection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM act_trace WHERE session_id = ?1",
        [session_id],
        |r| r.get(0),
    )?)
}

/// Пути файлов, правки которых зафиксированы (`kind = 'file_edit'`) начиная с
/// момента `since_ts` (unix-секунды) — спека 007, FR-006: «способ решения
/// собирается из уже существующих следов работы», а не запрашивается у
/// человека отдельным вопросом. Используется и при закрытии задачи
/// (`resolution.files`), и при предъявлении созревшей (`au task ripe`,
/// `au judge --hook`) как перечень изменённого.
///
/// `project_root` — вторая граница отбора, помимо времени (находка 2,
/// адверсариальный разбор спеки 007): `act_trace` — одна таблица на все
/// проекты (миграция v9 не хранит `project` вовсе), и пока задача проекта A
/// в работе, хук `au trace --hook` в другом окне пишет туда же правки
/// проекта B. `Some(root)` оставляет только пути ПОД этим каталогом;
/// `None` — каталог задачи неизвестен графу, тогда фильтр по-прежнему
/// работает только по времени, как и до этой правки (осознанно оставлено:
/// не хуже прежнего поведения, но и не решает находку 2 для такой задачи).
pub fn files_edited_since(
    conn: &Connection,
    since_ts: i64,
    project_root: Option<&Path>,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT payload FROM act_trace
          WHERE kind = 'file_edit' AND ts >= ?1 AND payload != ''
          ORDER BY payload",
    )?;
    let rows = stmt.query_map([since_ts], |r| r.get::<_, String>(0))?;
    let paths = rows.filter_map(std::result::Result::ok);

    let Some(root) = project_root else {
        return Ok(paths.collect());
    };
    let root_prefix = normalized_prefix(root);
    Ok(paths
        .filter(|p| normalize_for_compare(p).starts_with(&root_prefix))
        .collect())
}

/// Путь к устойчивому виду для сравнения по префиксу каталога проекта:
/// разделители приведены к `/`, регистр — к нижнему, снят расширенный
/// префикс Windows.
///
/// Все три приведения обязательны, и третье выяснилось живым прогоном.
/// `data.path` узла проекта приходит от `Path::to_string_lossy` после
/// `canonicalize`, а `canonicalize` на Windows возвращает путь в расширенной
/// форме — `\\?\A:\workSpace\aurelius`. Payload в `act_trace` приходит от
/// `tool_input.file_path`, как его прислал Claude Code, то есть обычным
/// `A:\workSpace\aurelius\...`. Без снятия префикса сравнение отсекало ВСЕ
/// файлы до единого даже внутри того самого каталога — и молча: пустой
/// список файлов выглядит как «правок не было», а не как «фильтр сломан».
fn normalize_for_compare(p: &str) -> String {
    let s = p.replace('\\', "/").to_lowercase();
    // `\\?\UNC\server\share` → `//server/share`, `\\?\A:\...` → `a:/...`.
    if let Some(rest) = s.strip_prefix("//?/unc/") {
        return format!("//{rest}");
    }
    s.strip_prefix("//?/").map_or(s.clone(), str::to_owned)
}

/// `root`, нормализованный и с гарантированным хвостовым `/` — без хвоста
/// `"/repo"` совпал бы префиксом и с `"/repo2/..."`, что был бы уже другой
/// проект.
fn normalized_prefix(root: &Path) -> String {
    let mut s = normalize_for_compare(&root.to_string_lossy());
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn test_conn() -> Connection {
        let dir =
            std::env::temp_dir().join(format!("aurelius-trace-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn ingest_writes_and_counts() {
        let conn = test_conn();
        let id = ingest(
            &conn,
            &TraceInput {
                session_id: "s1",
                kind: TraceKind::ToolCall,
                payload: "cargo build",
                exit_code: Some(0),
                state_hash_pre: None,
                state_hash_post: None,
            },
        )
        .expect("ingest");
        assert!(id > 0);
        assert_eq!(count_for_session(&conn, "s1").expect("count"), 1);
    }

    #[test]
    fn act_trace_is_append_only() {
        let conn = test_conn();
        ingest(
            &conn,
            &TraceInput {
                session_id: "s1",
                kind: TraceKind::Error,
                payload: "boom",
                exit_code: Some(1),
                state_hash_pre: None,
                state_hash_post: None,
            },
        )
        .expect("ingest");
        let upd = conn.execute("UPDATE act_trace SET payload = 'edited'", []);
        assert!(upd.is_err(), "UPDATE обязан упираться в триггер");
        let del = conn.execute("DELETE FROM act_trace", []);
        assert!(del.is_err(), "DELETE обязан упираться в триггер");
    }

    #[test]
    fn payload_is_capped_and_searchable() {
        let conn = test_conn();
        let long = "х".repeat(10_000);
        ingest(
            &conn,
            &TraceInput {
                session_id: "s1",
                kind: TraceKind::FileEdit,
                payload: &long,
                exit_code: None,
                state_hash_pre: Some("a".into()),
                state_hash_post: Some("b".into()),
            },
        )
        .expect("ingest");
        let stored: i64 = conn
            .query_row("SELECT LENGTH(payload) FROM act_trace", [], |r| r.get(0))
            .expect("len");
        // LENGTH в SQLite — символы для TEXT; потолок соблюдён.
        assert!(stored <= 2_000);
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM act_trace_fts WHERE act_trace_fts MATCH 'ххх*'",
                [],
                |r| r.get(0),
            )
            .expect("fts");
        assert_eq!(hits, 1);
    }

    fn ingest_file_edit(conn: &Connection, path: &str) {
        ingest(
            conn,
            &TraceInput {
                session_id: "s1",
                kind: TraceKind::FileEdit,
                payload: path,
                exit_code: None,
                state_hash_pre: None,
                state_hash_post: None,
            },
        )
        .expect("ingest file_edit");
    }

    /// Без `project_root` фильтр остаётся прежним — только по времени,
    /// поведение до находки 2 (осознанно сохранено как явный выбор — см.
    /// доккомментарий `files_edited_since`).
    #[test]
    fn files_edited_since_without_root_returns_everything_by_time() {
        let conn = test_conn();
        ingest_file_edit(&conn, "/repo-a/src/main.rs");
        ingest_file_edit(&conn, "/repo-b/src/lib.rs");

        let files = files_edited_since(&conn, 0, None).expect("query");

        assert_eq!(
            files,
            vec![
                "/repo-a/src/main.rs".to_owned(),
                "/repo-b/src/lib.rs".to_owned(),
            ]
        );
    }

    /// Находка 2 (адверсариальный разбор спеки 007): `act_trace` — одна
    /// таблица на все проекты; с границей каталога правки чужого проекта не
    /// обязаны попадать в список. Тест падал на прежней реализации
    /// (`files_edited_since` без параметра каталога вовсе) и проходит на
    /// новой.
    #[test]
    fn files_edited_since_with_root_excludes_other_projects() {
        let conn = test_conn();
        ingest_file_edit(&conn, "/repo-a/src/main.rs");
        ingest_file_edit(&conn, "/repo-b/src/lib.rs");

        let files = files_edited_since(&conn, 0, Some(Path::new("/repo-a"))).expect("query");

        assert_eq!(files, vec!["/repo-a/src/main.rs".to_owned()]);
    }

    /// Устойчивость к регистру и разделителю Windows: `data.path` узла
    /// проекта — от `canonicalize` (`C:\...`), payload — как его прислал
    /// Claude Code. Без нормализации сравнение молча отсекло бы всё до
    /// единого файла даже на том же самом каталоге.
    #[test]
    fn files_edited_since_root_prefix_is_case_and_separator_insensitive() {
        let conn = test_conn();
        ingest_file_edit(&conn, r"C:\Repo\src\Main.rs");

        let files = files_edited_since(&conn, 0, Some(Path::new("c:/repo"))).expect("query");

        assert_eq!(files, vec![r"C:\Repo\src\Main.rs".to_owned()]);
    }

    /// Каталог проекта приходит из `data.path`, а тот пишется после
    /// `canonicalize` — на Windows это расширенная форма `\\?\C:\...`, тогда
    /// как следы правок хранят обычный путь. Живой прогон показал, что без
    /// снятия префикса фильтр отсекает все файлы до единого, и происходит это
    /// молча: пустой список читается как «правок не было».
    #[test]
    fn files_edited_since_matches_verbatim_windows_root() {
        let conn = test_conn();
        ingest_file_edit(&conn, r"A:\workSpace\aurelius\crates\au\src\commands.rs");
        ingest_file_edit(&conn, r"A:\workSpace\boostix\src\index.ts");

        let files = files_edited_since(&conn, 0, Some(Path::new(r"\\?\A:\workSpace\aurelius")))
            .expect("query");

        assert_eq!(
            files,
            vec![r"A:\workSpace\aurelius\crates\au\src\commands.rs".to_owned()],
            "файл своего проекта обязан пройти фильтр, чужого — нет"
        );
    }

    fn ingest_cmd(conn: &Connection, cmd: &str) -> i64 {
        ingest(
            conn,
            &TraceInput {
                session_id: "s1",
                kind: TraceKind::ToolCall,
                payload: cmd,
                exit_code: Some(0),
                state_hash_pre: None,
                state_hash_post: None,
            },
        )
        .expect("ingest")
    }

    fn fts_hits(conn: &Connection, q: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM act_trace_fts WHERE act_trace_fts MATCH ?1",
            [q],
            |r| r.get(0),
        )
        .expect("fts")
    }

    #[test]
    fn ingest_stores_masked_payload() {
        let conn = test_conn();
        let id = ingest_cmd(&conn, "psql --password=hunter2 postgres://u:pw9@h/db");
        let stored: String = conn
            .query_row("SELECT payload FROM act_trace WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .expect("payload");
        assert_eq!(stored, "psql --password=*** postgres://u:***@h/db");
        assert_eq!(fts_hits(&conn, "hunter2"), 0);
        assert_eq!(fts_hits(&conn, "psql"), 1);
    }

    #[test]
    fn scrub_existing_masks_rows_and_index_then_is_idempotent() {
        let conn = test_conn();
        // Simulate rows written before masking existed: bypass ingest.
        conn.execute(
            "INSERT INTO act_trace (ts, session_id, kind, payload) VALUES
                 (1, 's', 'tool_call', 'export PGPASSWORD=hunter2'),
                 (2, 's', 'tool_call', 'git log 6784399')",
            [],
        )
        .expect("raw insert");
        assert_eq!(fts_hits(&conn, "hunter2"), 1);

        assert_eq!(scrub_existing(&conn).expect("scrub"), 1);
        let stored: String = conn
            .query_row("SELECT payload FROM act_trace WHERE ts = 1", [], |r| {
                r.get(0)
            })
            .expect("payload");
        assert_eq!(stored, "export PGPASSWORD=***");
        assert_eq!(fts_hits(&conn, "hunter2"), 0);
        assert_eq!(fts_hits(&conn, "PGPASSWORD"), 1);
        assert_eq!(fts_hits(&conn, "6784399"), 1);
        let integrity = conn.execute(
            "INSERT INTO act_trace_fts(act_trace_fts) VALUES ('integrity-check')",
            [],
        );
        assert!(integrity.is_ok(), "fts index out of sync: {integrity:?}");

        assert_eq!(scrub_existing(&conn).expect("scrub again"), 0);
        // The append-only guard is back in place.
        assert!(conn
            .execute("UPDATE act_trace SET payload = 'x'", [])
            .is_err());
    }

    fn db_path() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aurelius-trace-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        dir.join("test.db")
    }

    #[test]
    fn scrub_survives_concurrent_insert_between_read_and_write() {
        let path = db_path();
        let conn = db::open(&path).expect("open");
        conn.execute(
            "INSERT INTO act_trace (ts, session_id, kind, payload)
             VALUES (1, 's', 'tool_call', 'export PGPASSWORD=hunter2')",
            [],
        )
        .expect("raw insert");
        let mut writer = None;
        let changed = scrub_existing_with(&conn, || {
            let p = path.clone();
            writer = Some(std::thread::spawn(move || {
                let other = db::open(&p).expect("open second");
                other.execute(
                    "INSERT INTO act_trace (ts, session_id, kind, payload)
                     VALUES (2, 's', 'tool_call', 'ls')",
                    [],
                )
            }));
            // Give the second connection time to try its write mid-scrub.
            std::thread::sleep(std::time::Duration::from_millis(300));
        })
        .expect("scrub must not fail with 517");
        assert_eq!(changed, 1);
        let res = writer.expect("spawned").join().expect("join");
        assert!(res.is_ok(), "concurrent insert: {res:?}");
        assert_eq!(count_rows(&conn), 2);
        std::fs::remove_dir_all(path.parent().expect("dir")).ok();
    }

    fn count_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM act_trace", [], |r| r.get(0))
            .expect("count")
    }

    #[test]
    fn prune_counts_then_removes_old_rows_index_and_attributions() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO act_trace (id, ts, session_id, kind, payload) VALUES
                 (1, 100, 's', 'tool_call', 'oldcmd alpha'),
                 (2, 200, 's', 'tool_call', 'oldcmd beta'),
                 (3, 900, 's', 'tool_call', 'newcmd gamma')",
            [],
        )
        .expect("raw insert");
        conn.execute(
            "INSERT INTO trace_attribution (window_id, trace_id, overlap_score)
             VALUES (7, 1, 0.5), (7, 3, 0.5)",
            [],
        )
        .expect("attribution");

        assert_eq!(prune_trace_before(&conn, 500, false).expect("dry"), 2);
        assert_eq!(count_rows(&conn), 3);
        assert_eq!(fts_hits(&conn, "oldcmd"), 2);

        assert_eq!(prune_trace_before(&conn, 500, true).expect("apply"), 2);
        assert_eq!(count_rows(&conn), 1);
        assert_eq!(fts_hits(&conn, "oldcmd"), 0);
        assert_eq!(fts_hits(&conn, "newcmd"), 1);
        let attr: Vec<i64> = conn
            .prepare("SELECT trace_id FROM trace_attribution")
            .expect("prep")
            .query_map([], |r| r.get(0))
            .expect("q")
            .collect::<rusqlite::Result<_>>()
            .expect("rows");
        assert_eq!(attr, vec![3]);
        let integrity = conn.execute(
            "INSERT INTO act_trace_fts(act_trace_fts) VALUES ('integrity-check')",
            [],
        );
        assert!(integrity.is_ok(), "fts index out of sync: {integrity:?}");
        for name in ["act_trace_ro", "act_trace_nodel", "act_trace_ai"] {
            assert!(trigger_sql(&conn, name).is_some(), "{name} missing");
        }
        assert!(conn.execute("DELETE FROM act_trace", []).is_err());
        assert!(conn
            .execute("UPDATE act_trace SET payload = 'x'", [])
            .is_err());
    }
}
