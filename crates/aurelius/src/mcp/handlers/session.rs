use anyhow::Result;
use aurelius_core::{
    graph,
    models::{MemoryKind, NodeType, Relation},
};
use serde_json::json;

use super::{node_compact, open_db, resolve_node, truncate};

pub fn memory_session(params: &serde_json::Value) -> Result<serde_json::Value> {
    let summary = params
        .get("summary")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'summary' parameter"))?;
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");

    // Dedup: hash summary+project to prevent duplicate sessions
    let content_hash = {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(project.as_bytes());
        hasher.update(b"|");
        hasher.update(summary.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    let conn = open_db()?;

    if let Some(existing) = graph::find_node_by_content_hash(&conn, &content_hash)? {
        return Ok(json!({
            "id": existing.id.to_string(),
            "label": existing.label,
            "type": "session",
            "memory_kind": "episodic",
            "duplicate": true,
        }));
    }

    // Session data: only metadata not stored in child nodes
    let mut session_data = json!({ "project": project });
    if let Some(next) = params.get("next_steps") {
        session_data["next_steps"] = next.clone();
    }
    if let Some(files) = params.get("key_files") {
        session_data["key_files"] = files.clone();
    }

    let session_label = format!("[{}] {}", project, chrono::Utc::now().format("%Y-%m-%d %H:%M"));
    let session = graph::add_node_full(
        &conn,
        NodeType::Session,
        &session_label,
        Some(summary),
        "mcp",
        session_data,
        MemoryKind::Episodic,
        Some(&content_hash),
    )?;

    // Link session to project node — create if it doesn't exist
    let proj_node = match graph::find_project_by_label(&conn, project) {
        Ok(Some(n)) => n,
        _ => graph::add_node(
            &conn,
            NodeType::Project,
            project,
            None,
            "mcp-session",
            json!({"auto_created": true}),
        )?,
    };
    graph::add_edge(&conn, session.id, proj_node.id, Relation::BelongsTo, 1.0).ok();

    // Create decision nodes
    if let Some(decisions) = params.get("decisions").and_then(|d| d.as_array()) {
        for decision in decisions {
            if let Some(text) = decision.as_str() {
                let dec_node = graph::add_node(
                    &conn,
                    NodeType::Decision,
                    &format!("[{}] {}", project, truncate(text, 60)),
                    Some(text),
                    "mcp-session",
                    json!({"session_id": session.id.to_string()}),
                )?;
                graph::add_edge(&conn, session.id, dec_node.id, Relation::Contains, 1.0).ok();
                graph::add_edge(&conn, dec_node.id, proj_node.id, Relation::BelongsTo, 1.0).ok();
            }
        }
    }

    // Create problem+solution pairs
    if let Some(problems) = params.get("problems_solved").and_then(|p| p.as_array()) {
        for problem in problems {
            let prob_text = problem.get("problem").and_then(|p| p.as_str());
            let sol_text = problem.get("solution").and_then(|s| s.as_str());
            if let (Some(prob), Some(sol)) = (prob_text, sol_text) {
                let prob_node = graph::add_node(
                    &conn,
                    NodeType::Problem,
                    &format!("[{}] {}", project, truncate(prob, 60)),
                    Some(prob),
                    "mcp-session",
                    json!({"session_id": session.id.to_string()}),
                )?;
                let sol_node = graph::add_node(
                    &conn,
                    NodeType::Solution,
                    &format!("[{}] {}", project, truncate(sol, 60)),
                    Some(sol),
                    "mcp-session",
                    json!({"session_id": session.id.to_string()}),
                )?;
                graph::add_edge(&conn, sol_node.id, prob_node.id, Relation::Solves, 1.0).ok();
                graph::add_edge(&conn, session.id, prob_node.id, Relation::Contains, 1.0).ok();
                graph::add_edge(&conn, session.id, sol_node.id, Relation::Contains, 1.0).ok();
                graph::add_edge(&conn, prob_node.id, proj_node.id, Relation::BelongsTo, 1.0).ok();
                graph::add_edge(&conn, sol_node.id, proj_node.id, Relation::BelongsTo, 1.0).ok();
            }
        }
    }

    // Link session to tasks if specified
    let mut linked_tasks = vec![];
    if let Some(tasks) = params.get("tasks").and_then(|t| t.as_array()) {
        for task_ref in tasks {
            if let Some(task_id) = task_ref.as_str() {
                if let Ok(task_node) = resolve_node(&conn, task_id) {
                    graph::add_edge(&conn, session.id, task_node.id, Relation::RelatedTo, 1.0).ok();
                    linked_tasks.push(json!({
                        "id": task_node.id.to_string(),
                        "label": task_node.label,
                        "status": task_node.data.get("status"),
                    }));
                }
            }
        }
    }

    // Always show active tasks for this project as a hint
    let active_tasks: Vec<serde_json::Value> = graph::get_tasks_filtered(
        &conn,
        Some(project),
        Some("active,blocked"),
        None,
        5,
    )?
    .iter()
    .map(|t| json!({
        "id": t.id.to_string(),
        "label": t.label,
        "status": t.data.get("status"),
        "priority": t.data.get("priority"),
    }))
    .collect();

    Ok(json!({
        "id": session.id.to_string(),
        "label": session.label,
        "type": "session",
        "memory_kind": "episodic",
        "created": true,
        "linked_tasks": linked_tasks,
        "active_tasks_hint": active_tasks,
    }))
}

pub fn memory_recall(params: &serde_json::Value) -> Result<serde_json::Value> {
    let topic = params
        .get("topic")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'topic' parameter"))?;
    let depth = params.get("depth").and_then(|d| d.as_u64()).unwrap_or(1) as u32;

    let conn = open_db()?;
    let (context_nodes, _) = graph::context(&conn, topic, depth)?;

    let mut decisions = vec![];
    let mut problems = vec![];
    let mut solutions = vec![];
    let mut sessions = vec![];
    let mut concepts = vec![];
    let mut tasks = vec![];

    for node in &context_nodes {
        match &node.node_type {
            NodeType::Decision => decisions.push(node_compact(node)),
            NodeType::Problem => problems.push(node_compact(node)),
            NodeType::Solution => solutions.push(node_compact(node)),
            NodeType::Session => sessions.push(node_compact(node)),
            NodeType::Task => tasks.push(node_compact(node)),
            NodeType::Concept | NodeType::Project => concepts.push(node_compact(node)),
            _ => {}
        }
    }

    for node in &context_nodes {
        graph::touch_node(&conn, node.id).ok();
    }

    let knowledge_count = decisions.len() + problems.len() + solutions.len() + sessions.len() + concepts.len() + tasks.len();

    Ok(json!({
        "topic": topic,
        "decisions": decisions,
        "problems": problems,
        "solutions": solutions,
        "sessions": sessions,
        "tasks": tasks,
        "concepts": concepts,
        "total_knowledge_nodes": knowledge_count,
        "total_graph_nodes": context_nodes.len(),
    }))
}
