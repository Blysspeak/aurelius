use anyhow::Result;
use aurelius_core::{
    graph,
    models::{MemoryKind, NodeType, Relation},
};
use serde_json::json;

use super::{node_compact, node_detail, open_db, resolve_node, truncate};

pub fn task_create(params: &serde_json::Value) -> Result<serde_json::Value> {
    let title = params
        .get("title")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'title' parameter"))?;
    let description = params.get("description").and_then(|d| d.as_str());
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");
    let priority = params
        .get("priority")
        .and_then(|p| p.as_str())
        .unwrap_or("medium");
    let acceptance_criteria = params
        .get("acceptance_criteria")
        .cloned()
        .unwrap_or(json!([]));

    let conn = open_db()?;

    let task_data = json!({
        "status": "backlog",
        "priority": priority,
        "acceptance_criteria": acceptance_criteria,
        "project": project,
        "started_at": null,
        "completed_at": null,
    });

    let label = format!("[{}] {}", project, title);
    let task = graph::add_node_full(
        &conn,
        NodeType::Task,
        &label,
        description,
        "mcp",
        task_data,
        MemoryKind::Semantic,
        None,
    )?;

    // Link to project (auto-create if missing)
    let proj_node = match graph::find_project_by_label(&conn, project) {
        Ok(Some(n)) => n,
        _ => graph::add_node(
            &conn,
            NodeType::Project,
            project,
            None,
            "mcp-task",
            json!({"auto_created": true}),
        )?,
    };
    graph::add_edge(&conn, task.id, proj_node.id, Relation::BelongsTo, 1.0).ok();

    // Parent task (subtask_of)
    if let Some(parent_id) = params.get("parent").and_then(|p| p.as_str()) {
        if let Ok(parent) = resolve_node(&conn, parent_id) {
            graph::add_edge(&conn, task.id, parent.id, Relation::SubtaskOf, 1.0).ok();
        }
    }

    // Blocks edges
    if let Some(blocks) = params.get("blocks").and_then(|b| b.as_array()) {
        for blocked in blocks {
            if let Some(blocked_id) = blocked.as_str() {
                if let Ok(blocked_node) = resolve_node(&conn, blocked_id) {
                    graph::add_edge(&conn, task.id, blocked_node.id, Relation::Blocks, 1.0).ok();
                }
            }
        }
    }

    Ok(json!({
        "id": task.id.to_string(),
        "label": task.label,
        "type": "task",
        "status": "backlog",
        "priority": priority,
        "project": project,
        "created": true,
    }))
}

pub fn task_update(params: &serde_json::Value) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

    let conn = open_db()?;
    let node = resolve_node(&conn, id)?;

    // Merge data fields
    let mut data = node.data.clone();
    let now = chrono::Utc::now().to_rfc3339();

    if let Some(status) = params.get("status").and_then(|s| s.as_str()) {
        // Set started_at on first activation
        if status == "active" && data.get("started_at").and_then(|s| s.as_str()).is_none() {
            data["started_at"] = json!(now);
        }
        // Set completed_at on done
        if status == "done" {
            data["completed_at"] = json!(now);
        }
        data["status"] = json!(status);
    }

    if let Some(blocked_by) = params.get("blocked_by").and_then(|b| b.as_str()) {
        data["status"] = json!("blocked");
        data["blocked_by"] = json!(blocked_by);
    }

    if let Some(priority) = params.get("priority").and_then(|p| p.as_str()) {
        data["priority"] = json!(priority);
    }

    if let Some(criteria) = params.get("acceptance_criteria") {
        data["acceptance_criteria"] = criteria.clone();
    }

    let new_note = params.get("note").and_then(|n| n.as_str());

    graph::update_node(&conn, node.id, new_note, Some(data.clone()))?;

    Ok(json!({
        "id": node.id.to_string(),
        "label": node.label,
        "status": data["status"],
        "priority": data["priority"],
        "updated": true,
    }))
}

pub fn task_list(params: &serde_json::Value) -> Result<serde_json::Value> {
    let project = params.get("project").and_then(|p| p.as_str());
    let status = params.get("status").and_then(|s| s.as_str());
    let priority = params.get("priority").and_then(|p| p.as_str());
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;

    let conn = open_db()?;
    let tasks = graph::get_tasks_filtered(&conn, project, status, priority, limit)?;

    let task_type = serde_json::to_string(&NodeType::WorkLog)?;

    let items: Vec<serde_json::Value> = tasks
        .iter()
        .map(|t| {
            // Count work logs linked to this task
            let log_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM edges e JOIN nodes n ON n.id = e.to_id
                     WHERE e.from_id = ?1 AND e.relation = 'contains' AND n.node_type = ?2",
                    rusqlite::params![t.id.to_string(), &task_type],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            json!({
                "id": t.id.to_string(),
                "label": t.label,
                "status": t.data.get("status").and_then(|s| s.as_str()).unwrap_or("backlog"),
                "priority": t.data.get("priority").and_then(|p| p.as_str()).unwrap_or("medium"),
                "work_logs": log_count,
                "created_at": t.created_at.to_rfc3339(),
                "note": t.note,
            })
        })
        .collect();

    Ok(json!({
        "tasks": items,
        "total": items.len(),
        "filters": {
            "project": project,
            "status": status,
            "priority": priority,
        }
    }))
}

pub fn task_log(params: &serde_json::Value) -> Result<serde_json::Value> {
    let task_id = params
        .get("task")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'task' parameter"))?;
    let text = params
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'text' parameter"))?;

    let conn = open_db()?;
    let task = resolve_node(&conn, task_id)?;

    // Extract project from task data
    let project = task
        .data
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");

    // Auto-activate backlog tasks
    let mut task_data = task.data.clone();
    let status = task_data
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("backlog");
    if status == "backlog" {
        task_data["status"] = json!("active");
        task_data["started_at"] = json!(chrono::Utc::now().to_rfc3339());
        graph::update_node(&conn, task.id, None, Some(task_data))?;
    }

    // Create WorkLog node
    let log_label = format!(
        "[{}] {}",
        project,
        truncate(text, 60)
    );
    let log_node = graph::add_node_full(
        &conn,
        NodeType::WorkLog,
        &log_label,
        Some(text),
        "mcp-task",
        json!({"task_id": task.id.to_string()}),
        MemoryKind::Episodic,
        None,
    )?;

    // Link: task --contains--> worklog
    graph::add_edge(&conn, task.id, log_node.id, Relation::Contains, 1.0).ok();

    // Link worklog to project
    if let Ok(Some(proj_node)) = graph::find_project_by_label(&conn, project) {
        graph::add_edge(&conn, log_node.id, proj_node.id, Relation::BelongsTo, 1.0).ok();
    }

    let mut created_nodes = vec![node_compact(&log_node)];

    // Create decision nodes
    if let Some(decisions) = params.get("decisions").and_then(|d| d.as_array()) {
        for decision in decisions {
            if let Some(dec_text) = decision.as_str() {
                let dec_node = graph::add_node(
                    &conn,
                    NodeType::Decision,
                    &format!("[{}] {}", project, truncate(dec_text, 60)),
                    Some(dec_text),
                    "mcp-task",
                    json!({"task_id": task.id.to_string()}),
                )?;
                graph::add_edge(&conn, task.id, dec_node.id, Relation::Contains, 1.0).ok();
                graph::add_edge(&conn, log_node.id, dec_node.id, Relation::Contains, 1.0).ok();
                if let Ok(Some(proj_node)) = graph::find_project_by_label(&conn, project) {
                    graph::add_edge(&conn, dec_node.id, proj_node.id, Relation::BelongsTo, 1.0)
                        .ok();
                }
                created_nodes.push(node_compact(&dec_node));
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
                    "mcp-task",
                    json!({"task_id": task.id.to_string()}),
                )?;
                let sol_node = graph::add_node(
                    &conn,
                    NodeType::Solution,
                    &format!("[{}] {}", project, truncate(sol, 60)),
                    Some(sol),
                    "mcp-task",
                    json!({"task_id": task.id.to_string()}),
                )?;
                graph::add_edge(&conn, sol_node.id, prob_node.id, Relation::Solves, 1.0).ok();
                graph::add_edge(&conn, task.id, prob_node.id, Relation::Contains, 1.0).ok();
                graph::add_edge(&conn, task.id, sol_node.id, Relation::Contains, 1.0).ok();
                graph::add_edge(&conn, log_node.id, prob_node.id, Relation::Contains, 1.0).ok();
                if let Ok(Some(proj_node)) = graph::find_project_by_label(&conn, project) {
                    graph::add_edge(&conn, prob_node.id, proj_node.id, Relation::BelongsTo, 1.0)
                        .ok();
                    graph::add_edge(&conn, sol_node.id, proj_node.id, Relation::BelongsTo, 1.0)
                        .ok();
                }
                created_nodes.push(node_compact(&prob_node));
                created_nodes.push(node_compact(&sol_node));
            }
        }
    }

    Ok(json!({
        "task_id": task.id.to_string(),
        "task_label": task.label,
        "created_nodes": created_nodes,
        "total_created": created_nodes.len(),
    }))
}

pub fn task_view(params: &serde_json::Value) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

    let conn = open_db()?;
    let task = resolve_node(&conn, id)?;
    graph::touch_node(&conn, task.id).ok();

    // BFS from task node, depth 2
    let (nodes, edges) = graph::context_from_id(&conn, &task.id.to_string(), 2)?;

    let mut work_logs = vec![];
    let mut decisions = vec![];
    let mut problems = vec![];
    let mut solutions = vec![];
    let mut subtasks = vec![];

    for node in &nodes {
        if node.id == task.id {
            continue;
        }
        match &node.node_type {
            NodeType::WorkLog => work_logs.push(node_compact(node)),
            NodeType::Decision => decisions.push(node_compact(node)),
            NodeType::Problem => problems.push(node_compact(node)),
            NodeType::Solution => solutions.push(node_compact(node)),
            NodeType::Task => subtasks.push(node_compact(node)),
            _ => {}
        }
    }

    // Sort work_logs by created_at for timeline
    work_logs.sort_by(|a, b| {
        let a_date = a.get("created_at").and_then(|d| d.as_str()).unwrap_or("");
        let b_date = b.get("created_at").and_then(|d| d.as_str()).unwrap_or("");
        a_date.cmp(b_date)
    });

    Ok(json!({
        "task": node_detail(&task),
        "status": task.data.get("status"),
        "priority": task.data.get("priority"),
        "acceptance_criteria": task.data.get("acceptance_criteria"),
        "timeline": work_logs,
        "decisions": decisions,
        "problems": problems,
        "solutions": solutions,
        "subtasks": subtasks,
        "total_edges": edges.len(),
    }))
}
