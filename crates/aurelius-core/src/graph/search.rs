use crate::models::{Node, NodeType};
use anyhow::Result;
use rusqlite::{params, Connection};

use super::row_to_node;

pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Node>> {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return get_recent_nodes(conn, limit);
    }
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1
         ORDER BY rank + (n.access_count * 0.1) DESC
         LIMIT ?2",
    )?;
    let nodes = stmt
        .query_map(params![trimmed, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn search_typed(conn: &Connection, query: &str, node_type: &NodeType, limit: usize) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(node_type)?;
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == "*" {
        let mut stmt = conn.prepare(
            "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                    memory_kind, last_accessed_at, access_count, content_hash
             FROM nodes WHERE node_type = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let nodes = stmt
            .query_map(params![type_str, limit as i64], row_to_node)?
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(nodes);
    }
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1 AND n.node_type = ?2
         LIMIT ?3",
    )?;
    let nodes = stmt
        .query_map(params![trimmed, type_str, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn get_unsolved_problems(conn: &Connection, limit: usize) -> Result<Vec<Node>> {
    let problem_type = serde_json::to_string(&NodeType::Problem)?;
    let solution_type = serde_json::to_string(&NodeType::Solution)?;
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash
         FROM nodes n
         WHERE n.node_type = ?1
           AND NOT EXISTS (
             SELECT 1 FROM edges e
             JOIN nodes sol ON sol.id = e.from_id AND sol.node_type = ?2
             WHERE e.to_id = n.id AND e.relation = 'solves'
           )
         ORDER BY n.created_at DESC
         LIMIT ?3",
    )?;
    let nodes = stmt
        .query_map(params![problem_type, solution_type, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn get_recent_nodes(conn: &Connection, limit: usize) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes ORDER BY created_at DESC LIMIT ?1",
    )?;
    let nodes = stmt
        .query_map(params![limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}
