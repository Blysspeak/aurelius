use anyhow::Result;
use aurelius_core::{graph, indexer, models::MemoryKind};
use serde_json::json;
use uuid::Uuid;

use super::{edge_brief, node_detail, open_db, parse_node_type, parse_relation, parse_since, resolve_node};

pub fn memory_search(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
    let type_filter = params.get("type").and_then(|t| t.as_str());
    let since = params.get("since").and_then(|s| s.as_str());

    let conn = open_db()?;
    let mut nodes = if let Some(type_str) = type_filter {
        let node_type = parse_node_type(type_str);
        graph::search_typed(&conn, query, &node_type, limit)?
    } else {
        graph::search(&conn, query, limit)?
    };

    if let Some(since_str) = since {
        if let Some(cutoff_time) = parse_since(since_str) {
            nodes.retain(|n| n.created_at >= cutoff_time);
        }
    }

    Ok(json!({
        "query": query,
        "type": type_filter,
        "since": since,
        "count": nodes.len(),
        "results": nodes.iter().map(node_detail).collect::<Vec<_>>(),
    }))
}

pub fn memory_context(params: &serde_json::Value) -> Result<serde_json::Value> {
    let topic = params
        .get("topic")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'topic' parameter"))?;
    let depth = params.get("depth").and_then(|d| d.as_u64()).unwrap_or(2) as u32;
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;

    let conn = open_db()?;
    let (nodes, edges) = graph::context(&conn, topic, depth)?;

    let total = nodes.len();
    let capped_nodes: Vec<_> = nodes.iter().take(limit).collect();

    for node in &capped_nodes {
        graph::touch_node(&conn, node.id).ok();
    }

    let compact_nodes: Vec<serde_json::Value> = capped_nodes
        .iter()
        .map(|n| {
            json!({
                "id": n.id.to_string(),
                "type": n.node_type,
                "label": n.label,
                "note": n.note,
            })
        })
        .collect();

    // Only include edges between nodes in the capped set
    let node_ids: std::collections::HashSet<String> =
        capped_nodes.iter().map(|n| n.id.to_string()).collect();
    let relevant_edges: Vec<serde_json::Value> = edges
        .iter()
        .filter(|e| {
            node_ids.contains(&e.from_id.to_string())
                && node_ids.contains(&e.to_id.to_string())
        })
        .map(|e| edge_brief(e))
        .collect();

    Ok(json!({
        "topic": topic,
        "depth": depth,
        "nodes": compact_nodes,
        "edges": relevant_edges,
        "returned": capped_nodes.len(),
        "total": total,
    }))
}

pub fn memory_add(params: &serde_json::Value) -> Result<serde_json::Value> {
    let label = params
        .get("label")
        .and_then(|l| l.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'label' parameter"))?;
    let type_str = params.get("type").and_then(|t| t.as_str()).unwrap_or("concept");
    let note = params.get("note").and_then(|n| n.as_str());
    let source = params.get("source").and_then(|s| s.as_str()).unwrap_or("mcp");
    let data = params.get("data").cloned().unwrap_or(json!({}));
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
    let from_str = params.get("from").and_then(|f| f.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'from' parameter"))?;
    let to_str = params.get("to").and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'to' parameter"))?;
    let relation_str = params.get("relation").and_then(|r| r.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'relation' parameter"))?;
    let weight = params.get("weight").and_then(|w| w.as_f64()).unwrap_or(1.0) as f32;

    let conn = open_db()?;
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

pub fn memory_update(params: &serde_json::Value) -> Result<serde_json::Value> {
    let identifier = params.get("id").and_then(|i| i.as_str())
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

pub fn memory_index(params: &serde_json::Value) -> Result<serde_json::Value> {
    let path = params.get("path").and_then(|p| p.as_str())
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
    let id_str = params.get("id").and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;
    let id: Uuid = id_str.parse()
        .map_err(|_| anyhow::anyhow!("invalid UUID: {id_str}"))?;

    let conn = open_db()?;
    let deleted = graph::delete_node(&conn, id)?;

    Ok(json!({ "id": id_str, "deleted": deleted }))
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

pub fn memory_gc() -> Result<serde_json::Value> {
    let conn = open_db()?;

    let dup_edges = conn.execute(
        "DELETE FROM edges WHERE id NOT IN (
            SELECT MIN(id) FROM edges GROUP BY from_id, to_id, relation
        )", [],
    )?;

    let orphan_edges = conn.execute(
        "DELETE FROM edges WHERE
            from_id NOT IN (SELECT id FROM nodes) OR
            to_id NOT IN (SELECT id FROM nodes)", [],
    )?;

    let dup_nodes = conn.execute(
        "DELETE FROM nodes WHERE content_hash IS NOT NULL AND id NOT IN (
            SELECT MIN(id) FROM nodes WHERE content_hash IS NOT NULL GROUP BY content_hash
        )", [],
    )?;

    Ok(json!({
        "duplicate_edges_removed": dup_edges,
        "orphan_edges_removed": orphan_edges,
        "duplicate_nodes_removed": dup_nodes,
    }))
}
