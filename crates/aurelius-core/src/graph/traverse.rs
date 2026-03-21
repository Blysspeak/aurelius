use crate::models::{Edge, Node};
use anyhow::Result;
use rusqlite::Connection;

use super::{row_to_edge, row_to_node, search};

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
        if queue.is_empty() {
            break;
        }
        let edges = get_edges_batch(conn, &queue)?;
        let mut neighbor_ids = vec![];
        for edge in edges {
            let neighbor_id = if queue.contains(&edge.from_id.to_string()) {
                edge.to_id.to_string()
            } else {
                edge.from_id.to_string()
            };
            if !visited_nodes.contains(&neighbor_id) {
                visited_nodes.insert(neighbor_id.clone());
                neighbor_ids.push(neighbor_id);
            }
            all_edges.push(edge);
        }
        let neighbors = get_nodes_batch(conn, &neighbor_ids)?;
        queue = neighbors.iter().map(|n| n.id.to_string()).collect();
        all_nodes.extend(neighbors);
    }
    Ok((all_nodes, all_edges))
}

fn get_edges_batch(conn: &Connection, node_ids: &[String]) -> Result<Vec<Edge>> {
    if node_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: Vec<String> = (1..=node_ids.len()).map(|i| format!("?{i}")).collect();
    let ph = placeholders.join(",");
    let sql = format!(
        "SELECT id, from_id, to_id, relation, weight, created_at FROM edges
         WHERE from_id IN ({ph}) OR to_id IN ({ph})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for id in node_ids {
        param_values.push(Box::new(id.clone()));
    }
    for id in node_ids {
        param_values.push(Box::new(id.clone()));
    }
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let edges = stmt
        .query_map(params.as_slice(), row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

fn get_nodes_batch(conn: &Connection, ids: &[String]) -> Result<Vec<Node>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash
         FROM nodes WHERE id IN ({})",
        placeholders.join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids.iter().map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>).collect();
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let nodes = stmt
        .query_map(params.as_slice(), row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}
