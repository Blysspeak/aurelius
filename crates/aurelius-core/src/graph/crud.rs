use crate::models::{Edge, MemoryKind, Node, NodeType, Relation};
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use super::{row_to_edge, row_to_node};

pub fn add_node(
    conn: &Connection,
    node_type: NodeType,
    label: &str,
    note: Option<&str>,
    source: &str,
    data: serde_json::Value,
) -> Result<Node> {
    add_node_full(conn, node_type, label, note, source, data, MemoryKind::Semantic, None)
}

#[allow(clippy::too_many_arguments)]
pub fn add_node_full(
    conn: &Connection,
    node_type: NodeType,
    label: &str,
    note: Option<&str>,
    source: &str,
    data: serde_json::Value,
    memory_kind: MemoryKind,
    content_hash: Option<&str>,
) -> Result<Node> {
    let now = Utc::now();
    let node = Node {
        id: Uuid::new_v4(),
        node_type,
        label: label.to_owned(),
        note: note.map(str::to_owned),
        source: source.to_owned(),
        data,
        created_at: now,
        updated_at: now,
        memory_kind,
        last_accessed_at: now,
        access_count: 0,
        content_hash: content_hash.map(str::to_owned),
    };
    conn.execute(
        "INSERT INTO nodes (id, node_type, label, note, source, data, created_at, updated_at, memory_kind, last_accessed_at, access_count, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            node.id.to_string(),
            serde_json::to_string(&node.node_type)?,
            node.label,
            node.note,
            node.source,
            serde_json::to_string(&node.data)?,
            node.created_at.to_rfc3339(),
            node.updated_at.to_rfc3339(),
            node.memory_kind.to_string(),
            node.last_accessed_at.to_rfc3339(),
            node.access_count,
            node.content_hash,
        ],
    )?;
    Ok(node)
}

pub fn add_edge(
    conn: &Connection,
    from_id: Uuid,
    to_id: Uuid,
    relation: Relation,
    weight: f32,
) -> Result<Edge> {
    let edge = Edge {
        id: Uuid::new_v4(),
        from_id,
        to_id,
        relation,
        weight,
        created_at: Utc::now(),
    };
    conn.execute(
        "INSERT OR IGNORE INTO edges (id, from_id, to_id, relation, weight, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            edge.id.to_string(),
            edge.from_id.to_string(),
            edge.to_id.to_string(),
            edge.relation.to_string(),
            edge.weight,
            edge.created_at.to_rfc3339(),
        ],
    )?;
    Ok(edge)
}

pub fn update_node(
    conn: &Connection,
    id: Uuid,
    note: Option<&str>,
    data: Option<serde_json::Value>,
) -> Result<bool> {
    let now = Utc::now();
    let mut updates = vec!["updated_at = ?1".to_string()];
    let mut param_idx = 2;

    if note.is_some() {
        updates.push(format!("note = ?{param_idx}"));
        param_idx += 1;
    }
    if data.is_some() {
        updates.push(format!("data = ?{param_idx}"));
        param_idx += 1;
    }
    let _ = param_idx;

    let sql = format!(
        "UPDATE nodes SET {} WHERE id = ?{}",
        updates.join(", "),
        updates.len() + 1
    );

    let now_str = now.to_rfc3339();
    let id_str = id.to_string();
    let note_str = note.map(str::to_owned);
    let data_str = data.map(|d| serde_json::to_string(&d).unwrap_or_default());

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now_str)];
    if let Some(n) = note_str {
        param_values.push(Box::new(n));
    }
    if let Some(d) = data_str {
        param_values.push(Box::new(d));
    }
    param_values.push(Box::new(id_str));

    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let affected = conn.execute(&sql, params.as_slice())?;
    Ok(affected > 0)
}

pub fn delete_node(conn: &Connection, id: Uuid) -> Result<bool> {
    let affected = conn.execute("DELETE FROM nodes WHERE id = ?1", params![id.to_string()])?;
    Ok(affected > 0)
}

pub fn touch_node(conn: &Connection, id: Uuid) -> Result<()> {
    let now = Utc::now();
    conn.execute(
        "UPDATE nodes SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
        params![now.to_rfc3339(), id.to_string()],
    )?;
    Ok(())
}

pub fn get_node(conn: &Connection, id: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn find_node_by_label(conn: &Connection, label: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE label = ?1",
    )?;
    let mut rows = stmt.query_map(params![label], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn find_project_by_label(conn: &Connection, label: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE label = ?1 AND node_type = 'project'",
    )?;
    let mut rows = stmt.query_map(params![label], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn find_node_by_content_hash(conn: &Connection, hash: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE content_hash = ?1",
    )?;
    let mut rows = stmt.query_map(params![hash], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn find_node_by_data_field(conn: &Connection, key: &str, value: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE json_extract(data, ?1) = ?2",
    )?;
    let json_path = format!("$.{key}");
    let mut rows = stmt.query_map(params![json_path, value], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn get_all_nodes(conn: &Connection) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes ORDER BY created_at DESC",
    )?;
    let nodes = stmt
        .query_map([], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn get_all_edges(conn: &Connection) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_id, to_id, relation, weight, created_at FROM edges ORDER BY created_at DESC",
    )?;
    let edges = stmt
        .query_map([], row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

pub fn get_nodes_paginated(conn: &Connection, offset: usize, limit: usize) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
    )?;
    let nodes = stmt
        .query_map(params![limit as i64, offset as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn get_edges_paginated(conn: &Connection, offset: usize, limit: usize) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_id, to_id, relation, weight, created_at FROM edges ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
    )?;
    let edges = stmt
        .query_map(params![limit as i64, offset as i64], row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

pub fn count_nodes(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get::<_, usize>(0))?)
}

pub fn count_edges(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get::<_, usize>(0))?)
}

pub fn get_nodes_by_type(conn: &Connection, node_type: &NodeType) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(node_type)?;
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE node_type = ?1 ORDER BY created_at DESC",
    )?;
    let nodes = stmt
        .query_map(params![type_str], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}
