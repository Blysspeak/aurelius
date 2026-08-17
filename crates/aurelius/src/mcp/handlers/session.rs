use anyhow::Result;
use aurelius_core::{
    graph::{self, ProblemSolved, SessionInput},
    models::{NodeType, Relation},
};
use serde_json::json;

use super::{node_compact, open_db, resolve_node, sync_push_if_enabled};

/// Строки массива параметра, пустой вектор при отсутствии или чужом типе.
fn string_list(params: &serde_json::Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub fn memory_session(params: &serde_json::Value) -> Result<serde_json::Value> {
    let summary = params
        .get("summary")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'summary' parameter"))?;
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");

    let decisions = string_list(params, "decisions");
    let next_steps = string_list(params, "next_steps");
    let key_files = string_list(params, "key_files");
    let problems_solved: Vec<ProblemSolved> = params
        .get("problems_solved")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    // Метка прогона. Не обязательна, но без неё запись невозможно отличить от
    // вчерашней: id прогона знает только вызывающий, граф его ниоткуда не
    // выведет.
    let agent_session = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let conn = open_db()?;

    // Сама запись — общий код с `au session` (graph::record_session). Здесь
    // остаётся только то, что есть у инструмента и нет у CLI: привязка к
    // задачам, подсказка об активных и авто-push синка.
    let written = graph::record_session(
        &conn,
        &SessionInput {
            decisions: &decisions,
            problems_solved: &problems_solved,
            next_steps: &next_steps,
            key_files: &key_files,
            agent_session,
            ..SessionInput::new(project, summary, "mcp")
        },
    )?;
    let session = written.session;

    if written.duplicate {
        return Ok(json!({
            "id": session.id.to_string(),
            "label": session.label,
            "type": "session",
            "memory_kind": "episodic",
            "duplicate": true,
        }));
    }

    // Link session to tasks if specified
    let mut linked_tasks = vec![];
    if let Some(tasks) = params.get("tasks").and_then(|t| t.as_array()) {
        for task_ref in tasks {
            if let Some(task_id) = task_ref.as_str() {
                if let Ok(task_node) = resolve_node(&conn, task_id) {
                    graph::add_edge(&conn, session.id, task_node.id, Relation::RelatedTo, 1.0)?;
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
        Some(graph::OPEN_TASK_STATUSES),
        None,
        5,
    )?
    .iter()
    .map(|t| {
        json!({
            "id": t.id.to_string(),
            "label": t.label,
            "status": t.data.get("status"),
            "priority": t.data.get("priority"),
        })
    })
    .collect();

    // US2: push everything new locally for a shared project right after this
    // session write. Best-effort — never fails memory_session (T022).
    sync_push_if_enabled(&conn, project);

    // Ровно та беда, ради которой это писалось: имена параметров теперь
    // проверены заслонкой, но правильно названный пустой список выглядел
    // переданным — и решения терялись при ответе "created": true.
    let (stored_fields, dropped_fields) = super::super::params::field_report(params);

    Ok(json!({
        "id": session.id.to_string(),
        "label": session.label,
        "type": "session",
        "memory_kind": "episodic",
        "created": true,
        "decisions_written": written.decisions,
        "problems_written": written.problems,
        "stored_fields": stored_fields,
        "dropped_fields": dropped_fields,
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
    let mut skills = vec![];

    for node in &context_nodes {
        match &node.node_type {
            NodeType::Decision => decisions.push(node_compact(node)),
            NodeType::Problem => problems.push(node_compact(node)),
            NodeType::Solution => solutions.push(node_compact(node)),
            NodeType::Session => sessions.push(node_compact(node)),
            NodeType::Task => tasks.push(node_compact(node)),
            NodeType::Concept | NodeType::Project => concepts.push(node_compact(node)),
            NodeType::Skill => skills.push(node_compact(node)),
            _ => {}
        }
    }

    for node in &context_nodes {
        // Best effort by design: an access counter must never fail a read.
        if let Err(e) = graph::touch_node(&conn, node.id) {
            tracing::warn!("could not record access for {}: {e}", node.id);
        }
    }

    let knowledge_count = decisions.len()
        + problems.len()
        + solutions.len()
        + sessions.len()
        + concepts.len()
        + tasks.len()
        + skills.len();

    Ok(json!({
        "topic": topic,
        "decisions": decisions,
        "problems": problems,
        "solutions": solutions,
        "sessions": sessions,
        "tasks": tasks,
        "concepts": concepts,
        "skills": skills,
        "skills_hint": if skills.is_empty() { serde_json::Value::Null } else { json!("Relevant skill cards found — call skill_get <name> for full instructions.") },
        "total_knowledge_nodes": knowledge_count,
        "total_graph_nodes": context_nodes.len(),
    }))
}
