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
pub fn files_edited_since(conn: &Connection, since_ts: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT payload FROM act_trace
          WHERE kind = 'file_edit' AND ts >= ?1 AND payload != ''
          ORDER BY payload",
    )?;
    let rows = stmt.query_map([since_ts], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
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
}
