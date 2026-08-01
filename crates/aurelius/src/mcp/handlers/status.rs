use anyhow::Result;
use aurelius_core::{graph, indexer, models::NodeType};
use serde_json::json;

use super::{node_brief, node_detail, open_db, sync_pull_if_enabled};

pub fn memory_status(params: &serde_json::Value) -> Result<serde_json::Value> {
    let project_filter = params.get("project").and_then(|p| p.as_str());
    let conn = open_db()?;

    // Auto-index current working directory if not yet indexed
    if let Ok(cwd) = std::env::current_dir() {
        indexer::ensure_indexed(&conn, &cwd).ok();
    }

    // US2: pull any pending sync updates for a shared project before reading
    // the graph below, so the response reflects the peer's latest work.
    // Best-effort — never fails memory_status (T022).
    sync_pull_if_enabled(&conn, project_filter);

    let projects = graph::search_typed(&conn, "*", &NodeType::Project, 10)?;
    let crates = graph::search_typed(&conn, "*", &NodeType::Crate, 20)?;
    let mut skills = graph::get_nodes_by_type(&conn, &NodeType::Skill)?;
    skills.sort_by(|a, b| b.access_count.cmp(&a.access_count));
    let total_nodes = graph::count_nodes(&conn)?;
    let total_edges = graph::count_edges(&conn)?;

    let (recent_decisions, problems, recent_solutions, recent_sessions, active_tasks) =
        if let Some(proj) = project_filter {
            let fts_query = format!("\"[{}]\"", proj);
            let prefix = format!("[{}]", proj);
            (
                graph::search_typed(&conn, &fts_query, &NodeType::Decision, 10)?,
                graph::get_unsolved_problems(&conn, 50)?
                    .into_iter()
                    .filter(|n| n.label.starts_with(&prefix))
                    .take(10)
                    .collect::<Vec<_>>(),
                graph::search_typed(&conn, &fts_query, &NodeType::Solution, 10)?,
                graph::search_typed(&conn, &fts_query, &NodeType::Session, 5)?,
                graph::get_tasks_filtered(&conn, Some(proj), Some("active,blocked"), None, 10)?,
            )
        } else {
            (
                graph::search_typed(&conn, "*", &NodeType::Decision, 10)?,
                graph::get_unsolved_problems(&conn, 10)?,
                graph::search_typed(&conn, "*", &NodeType::Solution, 10)?,
                graph::search_typed(&conn, "*", &NodeType::Session, 5)?,
                graph::get_tasks_filtered(&conn, None, Some("active,blocked"), None, 10)?,
            )
        };

    let active_tasks_json: Vec<serde_json::Value> = active_tasks
        .iter()
        .map(|t| {
            json!({
                "id": t.id.to_string(),
                "label": t.label,
                "status": t.data.get("status"),
                "priority": t.data.get("priority"),
                "note": t.note,
                "created_at": t.created_at.to_rfc3339(),
                "created_by": t.created_by,
                "updated_by": t.updated_by,
            })
        })
        .collect();

    Ok(json!({
        "summary": {
            "total_nodes": total_nodes,
            "total_edges": total_edges,
        },
        "project_filter": project_filter,
        "projects": projects.iter().map(node_brief).collect::<Vec<_>>(),
        "crates": crates.iter().map(node_brief).collect::<Vec<_>>(),
        "skills": skills.iter().take(30).map(|n| json!({
            "name": n.label,
            "trigger": n.note,
            "uses": n.access_count,
        })).collect::<Vec<_>>(),
        "active_tasks": active_tasks_json,
        "recent_decisions": recent_decisions.iter().map(node_detail).collect::<Vec<_>>(),
        "open_problems": problems.iter().map(node_detail).collect::<Vec<_>>(),
        "recent_solutions": recent_solutions.iter().map(node_detail).collect::<Vec<_>>(),
        "recent_sessions": recent_sessions.iter().map(node_detail).collect::<Vec<_>>(),
    }))
}
