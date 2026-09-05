//! Ступень 1 «Бит-и-Дело»: журнал следов действий (specs/003-bit-i-delo).
//!
//! Единственная точка входа конвейера памяти v2. Каждый след — сырой факт
//! «что агент сделал и чем это кончилось», без интерпретации: вид, полезная
//! нагрузка, код возврата, хэш затронутого состояния до/после. Таблица
//! append-only на уровне триггеров БД — задним числом историю не правят.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
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
    let payload: String = t.payload.chars().take(PAYLOAD_CAP).collect();
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
}
