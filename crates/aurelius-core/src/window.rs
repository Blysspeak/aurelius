//! Ступень 3 «Бит-и-Дело»: recall как транзакция, а не SELECT.
//!
//! Каждое извлечение узла открывает лабильное окно (к нему судья атрибутирует
//! последующие следы действий), пишет путь извлечения query→node и подаёт
//! КОРРЕКЦИИ ПЕРВЫМИ: забывание — активная поправка в выдаче, а не дыра.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// Сигнатура запроса: нормализованные токены, отсортированные и захэшированные.
/// Одинаковые по смыслу формулировки складываются в один путь чаще, чем разные.
pub fn query_sig(query: &str) -> String {
    let mut tokens: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 2)
        .map(str::to_owned)
        .collect();
    tokens.sort();
    tokens.dedup();
    let mut h = Sha256::new();
    h.update(tokens.join(" ").as_bytes());
    format!("{:x}", h.finalize())[..16].to_owned()
}

/// Путь заблокирован? Такой узел не отдаётся ЭТОЙ формулировке запроса,
/// оставаясь достижимым другими.
pub fn pathway_blocked(conn: &Connection, sig: &str, node_id: &str) -> Result<bool> {
    let blocked: Option<i64> = conn
        .query_row(
            "SELECT blocked FROM pathways WHERE query_sig = ?1 AND node_id = ?2",
            [sig, node_id],
            |r| r.get(0),
        )
        .ok();
    Ok(blocked == Some(1))
}

/// Зафиксировать извлечение: путь + лабильное окно (одно открытое на узел
/// и сессию — повторный recall в той же сессии не плодит окон).
pub fn record_recall(
    conn: &Connection,
    sig: &str,
    node_id: &str,
    session_id: &str,
    content: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pathways (query_sig, node_id) VALUES (?1, ?2)
         ON CONFLICT(query_sig, node_id) DO NOTHING",
        [sig, node_id],
    )?;
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    let snapshot_hash = format!("{:x}", h.finalize());
    // Частичный UNIQUE (node, session, closed IS NULL) отсекает дубль — глотаем.
    let _ = conn.execute(
        "INSERT INTO labile_window (node_id, session_id, snapshot_hash, opened_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![node_id, session_id, snapshot_hash, Utc::now().timestamp()],
    );
    Ok(())
}

pub struct Correction {
    pub reason: String,
    pub replacement_id: Option<String>,
}

/// Коррекции, релевантные запросу, — подавать ПЕРЕД результатами поиска.
pub fn corrections_for(conn: &Connection, query: &str) -> Result<Vec<Correction>> {
    let fts: String = query
        .split_whitespace()
        .filter(|t| t.chars().count() > 2)
        .take(6)
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    if fts.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT c.reason, c.replacement_id FROM corrections_fts f
          JOIN corrections c ON c.id = f.rowid
         WHERE corrections_fts MATCH ?1 ORDER BY c.minted_at DESC LIMIT 5",
    )?;
    let rows = stmt.query_map([fts], |r| {
        Ok(Correction {
            reason: r.get(0)?,
            replacement_id: r.get::<_, Option<String>>(1)?,
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

/// Отчеканить коррекцию: «раньше считалось X — больше не считается, вот почему».
pub fn mint_correction(
    conn: &Connection,
    dead_node_id: &str,
    pattern: &str,
    reason: &str,
    replacement_id: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO corrections (fts_pattern, dead_node_id, replacement_id, reason, minted_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            pattern,
            dead_node_id,
            replacement_id,
            reason,
            Utc::now().timestamp()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn test_conn() -> Connection {
        let dir =
            std::env::temp_dir().join(format!("aurelius-window-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn query_sig_is_order_insensitive() {
        assert_eq!(
            query_sig("вебхуки мерчанта paysido"),
            query_sig("Paysido: мерчанта вебхуки!")
        );
        assert_ne!(query_sig("вебхуки мерчанта"), query_sig("курс обмена"));
    }

    #[test]
    fn one_open_window_per_node_and_session() {
        let conn = test_conn();
        record_recall(&conn, "sig", "n1", "s1", "text").expect("first");
        record_recall(&conn, "sig", "n1", "s1", "text").expect("second is swallowed");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM labile_window WHERE node_id='n1' AND closed_at IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 1);
    }

    #[test]
    fn corrections_surface_for_matching_query() {
        let conn = test_conn();
        mint_correction(
            &conn,
            "dead-1",
            "вебхуки paysido",
            "проба провалилась: файл удалён",
            None,
        )
        .expect("mint");
        let hits = corrections_for(&conn, "почему вебхуки paysido молчат").expect("query");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].reason.contains("проба"));
        assert!(corrections_for(&conn, "логотип бота")
            .expect("other")
            .is_empty());
    }
}
