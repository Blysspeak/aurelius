use anyhow::Result;
use rusqlite::{Connection, params};
use uuid::Uuid;
use chrono::Utc;
use crate::models::{Node, Edge, NodeType, Relation};

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
    };
    conn.execute(
        "INSERT INTO nodes (id, node_type, label, note, source, data, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            node.id.to_string(),
            serde_json::to_string(&node.node_type)?,
            node.label,
            node.note,
            node.source,
            serde_json::to_string(&node.data)?,
            node.created_at.to_rfc3339(),
            node.updated_at.to_rfc3339(),
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
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1
         LIMIT ?2",
    )?;
    let nodes = stmt.query_map(params![query, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn context(conn: &Connection, topic: &str, depth: u32) -> Result<(Vec<Node>, Vec<Edge>)> {
    // find seed nodes by label or FTS
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
        if queue.is_empty() { break; }
    }
    Ok((all_nodes, all_edges))
}

fn get_edges(conn: &Connection, node_id: &str) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_id, to_id, relation, weight, created_at FROM edges
         WHERE from_id = ?1 OR to_id = ?1",
    )?;
    let edges = stmt.query_map(params![node_id], row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

fn get_node(conn: &Connection, id: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at FROM nodes WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], row_to_node)?;
    Ok(rows.next().transpose()?)
}

fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
        id: row.get::<_, String>(0)?.parse().map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        node_type: serde_json::from_str(&row.get::<_, String>(1)?).map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        label: row.get(2)?,
        note: row.get(3)?,
        source: row.get(4)?,
        data: serde_json::from_str(&row.get::<_, String>(5)?).map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_at: row.get::<_, String>(6)?.parse().map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        updated_at: row.get::<_, String>(7)?.parse().map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
    })
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row.get::<_, String>(0)?.parse().map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        from_id: row.get::<_, String>(1)?.parse().map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        to_id: row.get::<_, String>(2)?.parse().map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        relation: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?)).map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        weight: row.get(4)?,
        created_at: row.get::<_, String>(5)?.parse().map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
    })
}
