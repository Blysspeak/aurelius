mod crud;
mod lease;
mod search;
mod session;
mod snapshot;
mod traverse;

pub use crud::*;
pub use lease::*;
pub use search::*;
pub use session::*;
pub use snapshot::*;
pub use traverse::*;

use crate::models::{Edge, MemoryKind, Node, NodeType, Relation};
use chrono::Utc;
use uuid::Uuid;

/// Заводит узел прогона и связывает его с задачей ребром `verified_by`
/// (спека 007, T013/T014, data-model.md «Ребро»). Улика внутри `data.evidence`
/// задачи — для быстрого чтения без обхода графа; этот узел и ребро — для
/// обратного пути: от прогона к задаче, которую он подтвердил.
pub fn link_evidence_run(
    conn: &rusqlite::Connection,
    task_id: Uuid,
    command: &str,
    exit_code: i64,
    artifact: Option<&str>,
) -> anyhow::Result<Uuid> {
    let label = format!("прогон: {command}");
    let data = serde_json::json!({
        "command": command,
        "exit_code": exit_code,
        "artifact": artifact,
    });
    let run = crud::add_node(
        conn,
        NodeType::Custom("run".to_owned()),
        &label,
        None,
        "au-task-evidence",
        data,
    )?;
    crud::add_edge(conn, task_id, run.id, Relation::VerifiedBy, 1.0)?;
    Ok(run.id)
}

/// Заводит координату секрета — узел `Config` с признаком `kind: "secret_ref"`
/// (спека 007, US4, T039, data-model.md). Значения секрета здесь нет ни в
/// одном поле: вызывающий обязан прогнать `location` через
/// `secret::detect_lookalike` до вызова — эта функция только пишет.
///
/// Метка следует соглашению задач: `[project] name`, если проект назван, иначе
/// голое имя — так `typed_in_project` находит координату тем же механизмом
/// области видимости, что и прочие типы узлов.
pub fn add_secret_ref(
    conn: &rusqlite::Connection,
    project: Option<&str>,
    name: &str,
    purpose: Option<&str>,
    location: &str,
) -> anyhow::Result<Node> {
    let location_kind = crate::secret::infer_location_kind(location);
    let label = match project {
        Some(p) => format!("[{p}] {name}"),
        None => name.to_owned(),
    };
    let data = serde_json::json!({
        "kind": "secret_ref",
        "name": name,
        "purpose": purpose,
        "location": location,
        "location_kind": location_kind.as_str(),
    });
    crud::add_node(conn, NodeType::Config, &label, None, "au-secret", data)
}

/// Живые координаты секретов, свежие первыми. Область видимости — та же, что
/// у `typed_in_project`: без `project` отдаёт координаты всех проектов.
///
/// Тип `Config` уже занят прочими настройками, поэтому фильтр по
/// `data.kind == "secret_ref"` обязателен — иначе `au secret list` показал бы
/// чужие конфигурационные узлы.
pub fn list_secret_refs(
    conn: &rusqlite::Connection,
    project: Option<&str>,
) -> anyhow::Result<Vec<Node>> {
    let mut nodes = search::typed_in_project(conn, &NodeType::Config, project, 500)?;
    nodes.retain(|n| n.data.get("kind").and_then(|v| v.as_str()) == Some("secret_ref"));
    Ok(nodes)
}

pub(crate) fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    let memory_kind_str: String = row
        .get::<_, String>(8)
        .unwrap_or_else(|_| "semantic".to_owned());
    let memory_kind = match memory_kind_str.as_str() {
        "episodic" => MemoryKind::Episodic,
        _ => MemoryKind::Semantic,
    };

    let last_accessed_str: Option<String> = row.get(9).ok();
    let last_accessed_at = last_accessed_str
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(Utc::now);

    Ok(Node {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        node_type: serde_json::from_str(&row.get::<_, String>(1)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        label: row.get(2)?,
        note: row.get(3)?,
        source: row.get(4)?,
        data: serde_json::from_str(&row.get::<_, String>(5)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_at: row
            .get::<_, String>(6)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        updated_at: row
            .get::<_, String>(7)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        memory_kind,
        last_accessed_at,
        access_count: row.get(10).unwrap_or(0),
        content_hash: row.get(11).ok().and_then(|v: Option<String>| v),
        created_by: row.get(12).ok().and_then(|v: Option<String>| v),
        updated_by: row.get(13).ok().and_then(|v: Option<String>| v),
        deleted_at: row
            .get::<_, Option<String>>(14)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok()),
        sync_seq: row.get(15).ok().and_then(|v: Option<i64>| v),
    })
}

pub(crate) fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        from_id: row
            .get::<_, String>(1)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        to_id: row
            .get::<_, String>(2)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        relation: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        weight: row.get(4)?,
        created_at: row
            .get::<_, String>(5)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_by: row.get(6).ok().and_then(|v: Option<String>| v),
        deleted_at: row
            .get::<_, Option<String>>(7)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok()),
        sync_seq: row.get(8).ok().and_then(|v: Option<i64>| v),
    })
}
