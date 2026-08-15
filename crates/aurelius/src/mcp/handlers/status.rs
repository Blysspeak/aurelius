use anyhow::Result;
use aurelius_core::{graph, indexer, models::NodeType};
use serde_json::json;

use super::{node_brief, node_detail, open_db, sync_pull_if_enabled};

pub fn memory_status(params: &serde_json::Value) -> Result<serde_json::Value> {
    let project_filter = params.get("project").and_then(|p| p.as_str());
    let conn = open_db()?;

    // Auto-index current working directory if not yet indexed
    if let Ok(cwd) = std::env::current_dir() {
        // Opportunistic: a failed auto-index must not fail the status call.
        if let Err(e) = indexer::ensure_indexed(&conn, &cwd) {
            tracing::warn!("could not auto-index {}: {e}", cwd.display());
        }
    }

    // US2: pull any pending sync updates for a shared project before reading
    // the graph below, so the response reflects the peer's latest work.
    // Best-effort — never fails memory_status (T022).
    sync_pull_if_enabled(&conn, project_filter);

    let projects = graph::search_typed(&conn, "*", &NodeType::Project, 10)?;
    let crates = graph::search_typed(&conn, "*", &NodeType::Crate, 20)?;
    let mut skills = graph::get_nodes_by_type(&conn, &NodeType::Skill)?;
    skills.sort_by_key(|s| std::cmp::Reverse(s.access_count));
    let total_nodes = graph::count_nodes(&conn)?;
    let total_edges = graph::count_edges(&conn)?;

    // Проектная выборка идёт через общий предикат принадлежности, а не через
    // полнотекстовый поиск по литералу "[проект]": FTS видел только префикс
    // метки и молчал про узлы, связанные с проектом ребром memory_relate.
    let (recent_decisions, problems, recent_solutions, recent_sessions, active_tasks) = (
        graph::typed_in_project(&conn, &NodeType::Decision, project_filter, 10)?,
        graph::get_unsolved_problems(&conn, project_filter, 10)?,
        graph::typed_in_project(&conn, &NodeType::Solution, project_filter, 10)?,
        graph::typed_in_project(&conn, &NodeType::Session, project_filter, 5)?,
        graph::get_tasks_filtered(
            &conn,
            project_filter,
            Some(graph::OPEN_TASK_STATUSES),
            None,
            10,
        )?,
    );

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
