use anyhow::Result;
use aurelius_core::{
    db, graph, indexer,
    models::{MemoryKind, NodeType, Relation},
};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

fn db_path() -> PathBuf {
    let base = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("aurelius");
    std::fs::create_dir_all(&base).ok();
    base.join("aurelius.db")
}

fn open_db() -> Result<Connection> {
    db::open(&db_path())
}

pub fn memory_status() -> Result<serde_json::Value> {
    let conn = open_db()?;

    // Project nodes
    let projects = graph::get_nodes_by_type(&conn, &NodeType::Project)?;

    // Recent decisions
    let decisions = graph::get_nodes_by_type(&conn, &NodeType::Decision)?;
    let recent_decisions: Vec<_> = decisions.into_iter().take(10).collect();

    // Open problems (only those without a linked solution)
    let problems = graph::get_unsolved_problems(&conn)?;

    // Sessions (episodic)
    let sessions = graph::get_nodes_by_type(&conn, &NodeType::Session)?;
    let recent_sessions: Vec<_> = sessions.into_iter().take(5).collect();

    // Crates
    let crates = graph::get_nodes_by_type(&conn, &NodeType::Crate)?;

    // Solutions (to pair with problems)
    let solutions = graph::get_nodes_by_type(&conn, &NodeType::Solution)?;
    let recent_solutions: Vec<_> = solutions.into_iter().take(10).collect();

    // Stats
    let total_nodes = graph::count_nodes(&conn)?;
    let total_edges = graph::count_edges(&conn)?;

    Ok(json!({
        "summary": {
            "total_nodes": total_nodes,
            "total_edges": total_edges,
        },
        "projects": projects.iter().map(node_brief).collect::<Vec<_>>(),
        "crates": crates.iter().map(node_brief).collect::<Vec<_>>(),
        "recent_decisions": recent_decisions.iter().map(node_detail).collect::<Vec<_>>(),
        "open_problems": problems.iter().map(node_detail).collect::<Vec<_>>(),
        "recent_solutions": recent_solutions.iter().map(node_detail).collect::<Vec<_>>(),
        "recent_sessions": recent_sessions.iter().map(node_detail).collect::<Vec<_>>(),
    }))
}

pub fn memory_context(params: &serde_json::Value) -> Result<serde_json::Value> {
    let topic = params
        .get("topic")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'topic' parameter"))?;
    let depth = params.get("depth").and_then(|d| d.as_u64()).unwrap_or(2) as u32;

    let conn = open_db()?;
    let (nodes, edges) = graph::context(&conn, topic, depth)?;

    // Touch accessed nodes
    for node in &nodes {
        graph::touch_node(&conn, node.id).ok();
    }

    Ok(json!({
        "topic": topic,
        "depth": depth,
        "nodes": nodes.iter().map(node_detail).collect::<Vec<_>>(),
        "edges": edges.iter().map(edge_brief).collect::<Vec<_>>(),
    }))
}

pub fn memory_search(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
    let type_filter = params.get("type").and_then(|t| t.as_str());

    let conn = open_db()?;
    let nodes = if let Some(type_str) = type_filter {
        let node_type = parse_node_type(type_str);
        graph::search_typed(&conn, query, &node_type, limit)?
    } else {
        graph::search(&conn, query, limit)?
    };

    Ok(json!({
        "query": query,
        "type": type_filter,
        "count": nodes.len(),
        "results": nodes.iter().map(node_detail).collect::<Vec<_>>(),
    }))
}

pub fn memory_add(params: &serde_json::Value) -> Result<serde_json::Value> {
    let label = params
        .get("label")
        .and_then(|l| l.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'label' parameter"))?;
    let type_str = params
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("concept");
    let note = params.get("note").and_then(|n| n.as_str());
    let source = params
        .get("source")
        .and_then(|s| s.as_str())
        .unwrap_or("mcp");
    let data = params
        .get("data")
        .cloned()
        .unwrap_or(json!({}));
    let memory_kind = match params.get("memory_kind").and_then(|m| m.as_str()) {
        Some("episodic") => MemoryKind::Episodic,
        _ => MemoryKind::Semantic,
    };

    let node_type = parse_node_type(type_str);

    let conn = open_db()?;
    let node = graph::add_node_full(&conn, node_type, label, note, source, data, memory_kind, None)?;

    Ok(json!({
        "id": node.id.to_string(),
        "label": node.label,
        "type": type_str,
        "memory_kind": node.memory_kind,
        "created": true,
    }))
}

pub fn memory_relate(params: &serde_json::Value) -> Result<serde_json::Value> {
    let from_str = params
        .get("from")
        .and_then(|f| f.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'from' parameter"))?;
    let to_str = params
        .get("to")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'to' parameter"))?;
    let relation_str = params
        .get("relation")
        .and_then(|r| r.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'relation' parameter"))?;
    let weight = params.get("weight").and_then(|w| w.as_f64()).unwrap_or(1.0) as f32;

    let conn = open_db()?;

    // Resolve from/to — try as UUID first, then by label
    let from_node = resolve_node(&conn, from_str)?;
    let to_node = resolve_node(&conn, to_str)?;
    let relation = parse_relation(relation_str)?;

    let edge = graph::add_edge(&conn, from_node.id, to_node.id, relation, weight)?;

    Ok(json!({
        "id": edge.id.to_string(),
        "from": from_node.label,
        "to": to_node.label,
        "relation": relation_str,
        "created": true,
    }))
}

pub fn memory_index(params: &serde_json::Value) -> Result<serde_json::Value> {
    let path = params
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;

    let conn = open_db()?;
    let result = indexer::index_project(&conn, std::path::Path::new(path))?;

    Ok(json!({
        "project": result.project_name,
        "crates_found": result.crates_found,
        "files_indexed": result.files_indexed,
        "dependencies_found": result.dependencies_found,
        "nodes_created": result.nodes_created,
        "nodes_updated": result.nodes_updated,
        "nodes_removed": result.nodes_removed,
    }))
}

pub fn memory_forget(params: &serde_json::Value) -> Result<serde_json::Value> {
    let id_str = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

    let id: Uuid = id_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid UUID: {id_str}"))?;

    let conn = open_db()?;
    let deleted = graph::delete_node(&conn, id)?;

    Ok(json!({
        "id": id_str,
        "deleted": deleted,
    }))
}

pub fn memory_update(params: &serde_json::Value) -> Result<serde_json::Value> {
    let identifier = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter (UUID or label)"))?;
    let note = params.get("note").and_then(|n| n.as_str());
    let data = params.get("data").cloned();

    if note.is_none() && data.is_none() {
        anyhow::bail!("at least one of 'note' or 'data' must be provided");
    }

    let conn = open_db()?;
    let node = resolve_node(&conn, identifier)?;
    let updated = graph::update_node(&conn, node.id, note, data)?;

    Ok(json!({
        "id": node.id.to_string(),
        "label": node.label,
        "updated": updated,
    }))
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

    // Build structured session data
    let mut session_data = json!({
        "project": project,
    });
    if let Some(decisions) = params.get("decisions") {
        session_data["decisions"] = decisions.clone();
    }
    if let Some(problems) = params.get("problems_solved") {
        session_data["problems_solved"] = problems.clone();
    }
    if let Some(next) = params.get("next_steps") {
        session_data["next_steps"] = next.clone();
    }
    if let Some(files) = params.get("key_files") {
        session_data["key_files"] = files.clone();
    }

    let conn = open_db()?;

    // Create session node (episodic memory)
    let session_label = format!("[{}] {}", project, chrono::Utc::now().format("%Y-%m-%d %H:%M"));
    let session = graph::add_node_full(
        &conn,
        NodeType::Session,
        &session_label,
        Some(summary),
        "mcp",
        session_data,
        MemoryKind::Episodic,
        None,
    )?;

    // Link session to project node if it exists
    if let Ok(proj_node) = resolve_node(&conn, project) {
        graph::add_edge(&conn, session.id, proj_node.id, Relation::BelongsTo, 1.0).ok();
    }

    // Create decision nodes from the session and link them
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
            }
        }
    }

    Ok(json!({
        "id": session.id.to_string(),
        "label": session.label,
        "type": "session",
        "memory_kind": "episodic",
        "created": true,
    }))
}

pub fn memory_recall(params: &serde_json::Value) -> Result<serde_json::Value> {
    let topic = params
        .get("topic")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'topic' parameter"))?;
    let depth = params.get("depth").and_then(|d| d.as_u64()).unwrap_or(1) as u32;

    let conn = open_db()?;

    // BFS context traversal (depth 1 by default to avoid graph explosion)
    let (context_nodes, _context_edges) = graph::context(&conn, topic, depth)?;

    // Separate by type — only knowledge nodes, skip structural noise
    let mut decisions = vec![];
    let mut problems = vec![];
    let mut solutions = vec![];
    let mut sessions = vec![];
    let mut concepts = vec![];

    for node in &context_nodes {
        match &node.node_type {
            NodeType::Decision => decisions.push(node_detail(node)),
            NodeType::Problem => problems.push(node_detail(node)),
            NodeType::Solution => solutions.push(node_detail(node)),
            NodeType::Session => sessions.push(node_detail(node)),
            NodeType::Concept | NodeType::Project => concepts.push(node_detail(node)),
            // Skip structural nodes (files, deps, crates) — AI can derive these
            _ => {}
        }
    }

    // Touch accessed knowledge nodes
    for node in &context_nodes {
        graph::touch_node(&conn, node.id).ok();
    }

    let knowledge_count = decisions.len() + problems.len() + solutions.len() + sessions.len() + concepts.len();

    Ok(json!({
        "topic": topic,
        "decisions": decisions,
        "problems": problems,
        "solutions": solutions,
        "sessions": sessions,
        "concepts": concepts,
        "total_knowledge_nodes": knowledge_count,
        "total_graph_nodes": context_nodes.len(),
    }))
}

pub fn memory_dump(params: &serde_json::Value) -> Result<serde_json::Value> {
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

    let conn = open_db()?;
    let total_nodes = graph::count_nodes(&conn)?;
    let total_edges = graph::count_edges(&conn)?;
    let nodes = graph::get_nodes_paginated(&conn, offset, limit)?;
    let edges = graph::get_edges_paginated(&conn, offset, limit)?;

    Ok(json!({
        "nodes": nodes.iter().map(node_detail).collect::<Vec<_>>(),
        "edges": edges.iter().map(edge_brief).collect::<Vec<_>>(),
        "total_nodes": total_nodes,
        "total_edges": total_edges,
        "offset": offset,
        "limit": limit,
    }))
}

// --- helpers ---

fn node_brief(node: &aurelius_core::models::Node) -> serde_json::Value {
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
    })
}

fn node_detail(node: &aurelius_core::models::Node) -> serde_json::Value {
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "note": node.note,
        "source": node.source,
        "data": node.data,
        "created_at": node.created_at.to_rfc3339(),
        "memory_kind": node.memory_kind,
        "access_count": node.access_count,
    })
}

fn edge_brief(edge: &aurelius_core::models::Edge) -> serde_json::Value {
    json!({
        "from": edge.from_id.to_string(),
        "to": edge.to_id.to_string(),
        "relation": edge.relation.to_string(),
        "weight": edge.weight,
    })
}

fn resolve_node(conn: &Connection, identifier: &str) -> Result<aurelius_core::models::Node> {
    // Try UUID
    if let Ok(uuid) = identifier.parse::<Uuid>() {
        if let Some(node) = graph::get_node(conn, &uuid.to_string())? {
            return Ok(node);
        }
    }
    // Try label
    if let Some(node) = graph::find_node_by_label(conn, identifier)? {
        return Ok(node);
    }
    // Try FTS
    let results = graph::search(conn, identifier, 1)?;
    results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("node not found: {identifier}"))
}

fn parse_node_type(s: &str) -> NodeType {
    match s {
        "project" => NodeType::Project,
        "decision" => NodeType::Decision,
        "concept" => NodeType::Concept,
        "problem" => NodeType::Problem,
        "solution" => NodeType::Solution,
        "person" => NodeType::Person,
        "dependency" => NodeType::Dependency,
        "server" => NodeType::Server,
        "file" => NodeType::File,
        "module" => NodeType::Module,
        "crate" => NodeType::Crate,
        "config" => NodeType::Config,
        "session" => NodeType::Session,
        "language" => NodeType::Language,
        other => NodeType::Custom(other.to_owned()),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

fn parse_relation(s: &str) -> Result<Relation> {
    Ok(match s {
        "uses" => Relation::Uses,
        "depends_on" => Relation::DependsOn,
        "solves" => Relation::Solves,
        "caused_by" => Relation::CausedBy,
        "inspired_by" => Relation::InspiredBy,
        "conflicts_with" => Relation::ConflictsWith,
        "supersedes" => Relation::Supersedes,
        "belongs_to" => Relation::BelongsTo,
        "related_to" => Relation::RelatedTo,
        "learned_from" => Relation::LearnedFrom,
        "contains" => Relation::Contains,
        "imports" => Relation::Imports,
        "exports" => Relation::Exports,
        "implements" => Relation::Implements,
        "configures" => Relation::Configures,
        "tracked_by" => Relation::TrackedBy,
        _ => anyhow::bail!("unknown relation: {s}"),
    })
}
