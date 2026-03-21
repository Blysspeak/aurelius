use crate::models::{Edge, MemoryKind, Node, NodeType, Relation};
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

pub fn add_node(
    conn: &Connection,
    node_type: NodeType,
    label: &str,
    note: Option<&str>,
    source: &str,
    data: serde_json::Value,
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
        memory_kind: MemoryKind::Semantic,
        last_accessed_at: now,
        access_count: 0,
        content_hash: None,
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
        "INSERT INTO edges (id, from_id, to_id, relation, weight, created_at)
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

pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Node>> {
    let trimmed = query.trim();
    // Empty or wildcard query → return most recent nodes
    if trimmed.is_empty() || trimmed == "*" {
        return get_recent_nodes(conn, limit);
    }
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1
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

pub fn get_unsolved_problems(conn: &Connection) -> Result<Vec<Node>> {
    // Problems that have NO edge where a solution node "solves" them
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash
         FROM nodes n
         WHERE n.node_type = '\"problem\"'
           AND NOT EXISTS (
             SELECT 1 FROM edges e
             JOIN nodes sol ON sol.id = e.from_id AND sol.node_type = '\"solution\"'
             WHERE e.to_id = n.id AND e.relation = 'solves'
           )
         ORDER BY n.created_at DESC",
    )?;
    let nodes = stmt
        .query_map([], row_to_node)?
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

pub fn context(conn: &Connection, topic: &str, depth: u32) -> Result<(Vec<Node>, Vec<Edge>)> {
    let seeds = search(conn, topic, 5)?;
    if seeds.is_empty() {
        return Ok((vec![], vec![]));
    }
    let mut visited_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_nodes: Vec<Node> = vec![];
    let mut all_edges: Vec<Edge> = vec![];
    let mut queue: Vec<String> = seeds.iter().map(|n| n.id.to_string()).collect();
    for node in &seeds {
        visited_nodes.insert(node.id.to_string());
        all_nodes.push(node.clone());
    }
    for _ in 0..depth {
        let mut next_queue = vec![];
        for node_id in &queue {
            let edges = get_edges(conn, node_id)?;
            for edge in edges {
                let neighbor_id = if edge.from_id.to_string() == *node_id {
                    edge.to_id.to_string()
                } else {
                    edge.from_id.to_string()
                };
                if !visited_nodes.contains(&neighbor_id) {
                    visited_nodes.insert(neighbor_id.clone());
                    if let Some(n) = get_node(conn, &neighbor_id)? {
                        all_nodes.push(n);
                        next_queue.push(neighbor_id);
                    }
                }
                all_edges.push(edge);
            }
        }
        queue = next_queue;
        if queue.is_empty() {
            break;
        }
    }
    Ok((all_nodes, all_edges))
}

pub fn delete_node(conn: &Connection, id: Uuid) -> Result<bool> {
    // Edges are CASCADE deleted via foreign key
    let affected = conn.execute("DELETE FROM nodes WHERE id = ?1", params![id.to_string()])?;
    Ok(affected > 0)
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

pub fn touch_node(conn: &Connection, id: Uuid) -> Result<()> {
    let now = Utc::now();
    conn.execute(
        "UPDATE nodes SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
        params![now.to_rfc3339(), id.to_string()],
    )?;
    Ok(())
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

    // Build params dynamically
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

pub fn find_node_by_label(conn: &Connection, label: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE label = ?1",
    )?;
    let mut rows = stmt.query_map(params![label], row_to_node)?;
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

fn get_edges(conn: &Connection, node_id: &str) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_id, to_id, relation, weight, created_at FROM edges
         WHERE from_id = ?1 OR to_id = ?1",
    )?;
    let edges = stmt
        .query_map(params![node_id], row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
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

fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    let memory_kind_str: String = row
        .get::<_, String>(8)
        .unwrap_or_else(|_| "semantic".to_owned());
    let memory_kind = match memory_kind_str.as_str() {
        "episodic" => MemoryKind::Episodic,
        _ => MemoryKind::Semantic,
    };

    let last_accessed_str: Option<String> = row.get(9).ok();
    let last_accessed_at = last_accessed_str
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(Utc::now);

    Ok(Node {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        node_type: serde_json::from_str(&row.get::<_, String>(1)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        label: row.get(2)?,
        note: row.get(3)?,
        source: row.get(4)?,
        data: serde_json::from_str(&row.get::<_, String>(5)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_at: row
            .get::<_, String>(6)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        updated_at: row
            .get::<_, String>(7)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        memory_kind,
        last_accessed_at,
        access_count: row.get(10).unwrap_or(0),
        content_hash: row.get(11).ok().and_then(|v: Option<String>| v),
    })
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        from_id: row
            .get::<_, String>(1)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        to_id: row
            .get::<_, String>(2)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        relation: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        weight: row.get(4)?,
        created_at: row
            .get::<_, String>(5)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
    })
}
