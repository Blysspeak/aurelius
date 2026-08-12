use anyhow::Result;
use aurelius_core::graph;
use serde_json::json;

use super::open_db;

/// Семислойный снапшот памяти — компактный Markdown под прямую инъекцию в
/// контекст. Read-only и мгновенный: его дёргает SessionStart-хук.
pub fn memory_snapshot(params: &serde_json::Value) -> Result<serde_json::Value> {
    let project = params.get("project").and_then(|p| p.as_str());
    let consolidate_first = params
        .get("consolidate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let conn = open_db()?;
    if consolidate_first {
        if let Some(p) = project {
            // Дистилляция дешёвая (чистый SQL), но затирает узел — только по просьбе.
            graph::consolidate(&conn, p)?;
        }
    }
    let markdown = graph::build_snapshot(&conn, project)?;
    Ok(json!({ "project": project, "markdown": markdown }))
}

/// Пересобрать дистиллят проекта (слой 7): хвосты next_steps последних сессий
/// + нерешённые проблемы одним узлом Digest. Идемпотентно.
pub fn memory_consolidate(params: &serde_json::Value) -> Result<serde_json::Value> {
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'project' parameter"))?;
    let conn = open_db()?;
    let node = graph::consolidate(&conn, project)?;
    Ok(json!({
        "id": node.id.to_string(),
        "label": node.label,
        "note": node.note,
    }))
}
