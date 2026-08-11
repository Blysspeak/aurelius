//! Слой 7-уровневой памяти: замороженный снапшот и дистилляция.
//!
//! Снапшот — компактный Markdown (жёсткий бюджет символов на слой, ~4.5К всего),
//! который инжектится в контекст агента ОДИН раз при старте сессии. Урок
//! hermes-agent: маленький курируемый срез в системном промпте бьёт большой
//! JSON по запросу — и не ломает prefix-cache, потому что заморожен на сессию.
//!
//! Дистиллят (слой 7) — структурная выжимка без LLM: незакрытые next_steps
//! последних сессий + нерешённые проблемы, пересобирается consolidate().

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use super::row_to_node;
use crate::models::{MemoryKind, Node, NodeType};

/// Бюджеты слоёв в символах. Сумма ~4500 — порядка 1.5К токенов.
const B_IDENTITY: usize = 600;
const B_WORKING: usize = 1000;
const B_EPISODIC: usize = 800;
const B_SEMANTIC: usize = 900;
const B_PROCEDURAL: usize = 500;
const B_DIGEST: usize = 500;

fn clip(s: &str, budget: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= budget {
        return one;
    }
    let cut: String = one.chars().take(budget.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Строки слоя: по одной на узел, суммарно не больше budget.
fn layer(nodes: &[Node], per_line: usize, budget: usize) -> String {
    let mut out = String::new();
    for n in nodes {
        let text = n.note.as_deref().unwrap_or(&n.label);
        let line = format!("- {}\n", clip(text, per_line));
        if out.chars().count() + line.chars().count() > budget {
            break;
        }
        out.push_str(&line);
    }
    out
}

fn typed_recent(
    conn: &Connection,
    t: &NodeType,
    project: Option<&str>,
    limit: usize,
) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(t)?;
    let like = project.map(|p| format!("[{p}]%"));
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
           FROM nodes
          WHERE node_type = ?1 AND deleted_at IS NULL
            AND (?2 IS NULL OR label LIKE ?2 OR label = ?3)
          ORDER BY updated_at DESC LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![type_str, like, project.unwrap_or(""), limit as i64],
        row_to_node,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Собрать семислойный снапшот. Только чтение, без сети и индексации —
/// вызывается хуком на старте каждой сессии и обязан быть мгновенным.
pub fn build_snapshot(conn: &Connection, project: Option<&str>) -> Result<String> {
    let ts = Utc::now().format("%Y-%m-%d %H:%M");
    let scope = project.unwrap_or("глобально");

    let identity = typed_recent(conn, &NodeType::UserFact, None, 12)?;
    let tasks = super::get_tasks_filtered(conn, project, Some("active,blocked"), None, 8)?;
    let problems: Vec<Node> = super::get_unsolved_problems(conn, 30)?
        .into_iter()
        .filter(|n| project.is_none_or(|p| n.label.starts_with(&format!("[{p}]"))))
        .take(6)
        .collect();
    let sessions = typed_recent(conn, &NodeType::Session, project, 3)?;
    let decisions = typed_recent(conn, &NodeType::Decision, project, 8)?;
    let concepts = typed_recent(conn, &NodeType::Concept, project, 5)?;
    let skills = typed_recent(conn, &NodeType::Skill, None, 8)?;
    let digest = typed_recent(conn, &NodeType::Digest, project, 1)?;

    let nodes = super::count_nodes(conn)?;
    let edges = super::count_edges(conn)?;

    let mut working = layer(&tasks, 160, B_WORKING / 2);
    working.push_str(&layer(&problems, 160, B_WORKING / 2));

    let mut semantic = layer(&decisions, 150, B_SEMANTIC * 2 / 3);
    semantic.push_str(&layer(&concepts, 150, B_SEMANTIC / 3));

    let sections: [(&str, String); 7] = [
        ("1 · Владелец", layer(&identity, 120, B_IDENTITY)),
        ("2 · В работе (задачи и открытые проблемы)", working),
        ("3 · Последние сессии", layer(&sessions, 250, B_EPISODIC)),
        ("4 · Решения и знания", semantic),
        ("5 · Приёмы", layer(&skills, 100, B_PROCEDURAL)),
        (
            "6 · Архив",
            format!(
                "- {nodes} узлов, {edges} рёбер; глубже — memory_recall(topic) / memory_search(query)\n"
            ),
        ),
        ("7 · Дистиллят", layer(&digest, B_DIGEST, B_DIGEST)),
    ];

    let mut md = format!("# Память Aurelius · {scope} · {ts} UTC\n");
    for (title, body) in sections {
        if body.is_empty() {
            continue;
        }
        md.push_str(&format!("\n## {title}\n{body}"));
    }
    Ok(md)
}

/// Пересобрать дистиллят проекта: next_steps последних сессий + нерешённые
/// проблемы. Идемпотентно — один Digest-узел на проект, старый затирается.
pub fn consolidate(conn: &Connection, project: &str) -> Result<Node> {
    let sessions = typed_recent(conn, &NodeType::Session, Some(project), 5)?;
    let mut steps: Vec<String> = Vec::new();
    for s in &sessions {
        if let Some(arr) = s.data.get("next_steps").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(t) = v.as_str() {
                    let t = clip(t, 140);
                    if !steps.contains(&t) {
                        steps.push(t);
                    }
                }
            }
        }
    }
    let problems: Vec<Node> = super::get_unsolved_problems(conn, 30)?
        .into_iter()
        .filter(|n| n.label.starts_with(&format!("[{project}]")))
        .take(5)
        .collect();

    let mut note = String::new();
    if !steps.is_empty() {
        note.push_str("Хвосты из сессий: ");
        note.push_str(&steps.into_iter().take(8).collect::<Vec<_>>().join("; "));
        note.push('.');
    }
    if !problems.is_empty() {
        let p = problems
            .iter()
            .map(|n| clip(n.note.as_deref().unwrap_or(&n.label), 100))
            .collect::<Vec<_>>()
            .join("; ");
        note.push_str(&format!(" Нерешённое: {p}."));
    }
    if note.is_empty() {
        note = "Хвостов нет — чисто.".to_owned();
    }

    let label = format!("[{project}] дистиллят");
    let type_str = serde_json::to_string(&NodeType::Digest)?;
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM nodes WHERE node_type = ?1 AND label = ?2 AND deleted_at IS NULL",
            rusqlite::params![type_str, label],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = existing {
        conn.execute(
            "UPDATE nodes SET note = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![note, Utc::now().to_rfc3339(), id],
        )?;
        let found = typed_recent(conn, &NodeType::Digest, Some(project), 1)?;
        found
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("дистиллят обновлён, но не читается"))
    } else {
        super::add_node_full(
            conn,
            NodeType::Digest,
            &label,
            Some(&note),
            "consolidate",
            serde_json::json!({ "project": project }),
            MemoryKind::Semantic,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-snap-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn clip_respects_budget_and_collapses_whitespace() {
        assert_eq!(clip("a  b\n c", 100), "a b c");
        let clipped = clip(&"ы".repeat(50), 10);
        assert!(clipped.chars().count() <= 10);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn snapshot_has_header_and_fits_total_budget() {
        let conn = test_conn();
        super::super::add_node(
            &conn,
            NodeType::UserFact,
            "владелец",
            Some("факт о владельце"),
            "test",
            serde_json::json!({}),
        )
        .expect("add user fact");
        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");
        assert!(md.starts_with("# Память Aurelius · demo"));
        assert!(md.contains("1 · Владелец"));
        // Общий потолок: снапшот обязан оставаться маленьким при любом графе.
        assert!(md.chars().count() < 6_000, "снапшот распух: {}", md.len());
    }

    #[test]
    fn consolidate_is_idempotent_one_digest_per_project() {
        let conn = test_conn();
        let a = consolidate(&conn, "demo").expect("first");
        let b = consolidate(&conn, "demo").expect("second");
        assert_eq!(a.id, b.id, "должен обновляться тот же узел");
        let type_str = serde_json::to_string(&NodeType::Digest).expect("type");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE node_type = ?1 AND deleted_at IS NULL",
                [type_str],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 1);
    }
}
