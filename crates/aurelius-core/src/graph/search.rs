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

/// Get tasks filtered by project prefix, status, and priority (from JSON `data` column).
/// Results sorted by priority (critical > high > medium > low), then by created_at desc.
pub fn get_tasks_filtered(
    conn: &Connection,
    project: Option<&str>,
    status: Option<&str>,
    priority: Option<&str>,
    limit: usize,
) -> Result<Vec<Node>> {
    let task_type = serde_json::to_string(&NodeType::Task)?;
    let mut conditions = vec!["n.node_type = ?1".to_string()];
    let mut param_idx = 2u32;

    // We'll build dynamic SQL with positional params
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(task_type)];

    if let Some(proj) = project {
        let prefix = format!("[{}]%", proj);
        conditions.push(format!("n.label LIKE ?{param_idx}"));
        params_vec.push(Box::new(prefix));
        param_idx += 1;
    }

    if let Some(st) = status {
        // Support comma-separated statuses
        let statuses: Vec<&str> = st.split(',').map(|s| s.trim()).collect();
        let placeholders: Vec<String> = statuses
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", param_idx + i as u32))
            .collect();
        conditions.push(format!(
            "json_extract(n.data, '$.status') IN ({})",
            placeholders.join(", ")
        ));
        for s in statuses {
            params_vec.push(Box::new(s.to_string()));
            param_idx += 1;
        }
    }

    if let Some(pri) = priority {
        conditions.push(format!("json_extract(n.data, '$.priority') = ?{param_idx}"));
        params_vec.push(Box::new(pri.to_string()));
        param_idx += 1;
    }
    let _ = param_idx; // suppress unused warning

    let sql = format!(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash
         FROM nodes n
         WHERE {}
         ORDER BY
           CASE json_extract(n.data, '$.priority')
             WHEN 'critical' THEN 0
             WHEN 'high' THEN 1
             WHEN 'medium' THEN 2
             WHEN 'low' THEN 3
             ELSE 4
           END,
           n.created_at DESC
         LIMIT ?{}",
        conditions.join(" AND "),
        params_vec.len() + 1
    );

    params_vec.push(Box::new(limit as i64));

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let nodes = stmt
        .query_map(params_refs.as_slice(), row_to_node)?
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
