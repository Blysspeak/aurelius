use crate::models::{Edge, Node};
use anyhow::Result;
use rusqlite::Connection;

use super::{row_to_edge, row_to_node, search};

/// Одно ребро видно с обоих концов, и на следующем шаге BFS оно приходит
/// вторично — уже со стороны соседа. Без этой отметки связь A→B попадала в
/// ответ дважды: счётчик «N edges» врал, а печать связей показывала близнеца.
type SeenEdges = std::collections::HashSet<uuid::Uuid>;

/// Hard ceiling on the total number of nodes one traversal may return,
/// seeds included. The project node is a hub everything hangs on, so an
/// uncapped BFS at depth 2 used to fan out across the whole database and
/// into other projects (measured 2026-08-30: 2809 nodes, 2.4 MB for one
/// memory_recall call). The cap lives here, not at the call sites, so
/// every caller inherits it and no per-caller default can reintroduce the
/// blow-up.
pub const MAX_TRAVERSAL_NODES: usize = 200;

/// Depth clamp for every traversal. Explicit depths above this are clamped
/// silently; smaller requested depths stay as they are. Three call sites
/// already had to lower their own defaults to 1 to survive hub nodes — the
/// clamp belongs to the walk itself, one defect, one fix.
pub const MAX_TRAVERSAL_DEPTH: u32 = 3;

/// Traversal outcome with the truncation report. Callers that only need
/// the graph use [`context`] / [`context_from_id`]; call sites that answer
/// to a model should surface `hidden_nodes`, so a cut answer can say so
/// instead of posing as the complete picture.
#[derive(Default)]
pub struct Traversal {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Nodes discovered by BFS but dropped because the node budget ran
    /// out. A lower bound: once the budget is gone the walk stops
    /// exploring, so anything further is unmeasured.
    pub hidden_nodes: usize,
    /// 1-based BFS depth at which the budget cut the walk, if it did.
    /// `Some(0)` would mean even the seed set did not fit.
    pub truncated_at_depth: Option<u32>,
}

pub fn context(conn: &Connection, topic: &str, depth: u32) -> Result<(Vec<Node>, Vec<Edge>)> {
    let traversal = context_with_report(conn, topic, depth)?;
    Ok((traversal.nodes, traversal.edges))
}

/// Same walk as [`context`], but keeps the truncation report: how many
/// nodes the cap hid and at which BFS depth the cut happened.
pub fn context_with_report(conn: &Connection, topic: &str, depth: u32) -> Result<Traversal> {
    let seeds = search(conn, topic, 5)?;
    if seeds.is_empty() {
        return Ok(Traversal::default());
    }
    walk(conn, seeds, depth)
}

/// BFS traversal from a specific node ID (no FTS search — starts from a known node).
pub fn context_from_id(
    conn: &Connection,
    node_id: &str,
    depth: u32,
) -> Result<(Vec<Node>, Vec<Edge>)> {
    let traversal = context_from_id_with_report(conn, node_id, depth)?;
    Ok((traversal.nodes, traversal.edges))
}

/// Same walk as [`context_from_id`] with the truncation report attached.
pub fn context_from_id_with_report(
    conn: &Connection,
    node_id: &str,
    depth: u32,
) -> Result<Traversal> {
    let seed = super::crud::get_node(conn, node_id)?;
    match seed {
        Some(n) => walk(conn, vec![n], depth),
        None => Ok(Traversal::default()),
    }
}

/// BFS shared by every entry point. Depth is clamped to
/// [`MAX_TRAVERSAL_DEPTH`]; the node budget stops the walk mid-level and
/// every discovered-but-dropped node lands in `hidden_nodes`.
fn walk(conn: &Connection, seeds: Vec<Node>, depth: u32) -> Result<Traversal> {
    let depth = depth.min(MAX_TRAVERSAL_DEPTH);
    let mut visited_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_edges = SeenEdges::new();
    let mut out = Traversal::default();
    let mut queue: Vec<String> = vec![];
    for node in seeds {
        if !visited_nodes.insert(node.id.to_string()) {
            continue;
        }
        if out.nodes.len() < MAX_TRAVERSAL_NODES {
            queue.push(node.id.to_string());
            out.nodes.push(node);
        } else {
            out.hidden_nodes += 1;
            out.truncated_at_depth.get_or_insert(0);
        }
    }
    for level in 0..depth {
        if queue.is_empty() {
            break;
        }
        if out.nodes.len() >= MAX_TRAVERSAL_NODES {
            // The budget ran out before this level: deeper neighborhoods
            // are not explored at all, so nothing can be counted there.
            out.truncated_at_depth.get_or_insert(level + 1);
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
            if seen_edges.insert(edge.id) {
                out.edges.push(edge);
            }
        }
        let neighbors = get_nodes_batch(conn, &neighbor_ids)?;
        let mut next_queue: Vec<String> = vec![];
        for node in neighbors {
            if out.nodes.len() < MAX_TRAVERSAL_NODES {
                next_queue.push(node.id.to_string());
                out.nodes.push(node);
            } else {
                out.hidden_nodes += 1;
                out.truncated_at_depth.get_or_insert(level + 1);
            }
        }
        queue = next_queue;
    }
    Ok(out)
}

fn get_edges_batch(conn: &Connection, node_ids: &[String]) -> Result<Vec<Edge>> {
    if node_ids.is_empty() {
        return Ok(vec![]);
    }
    let n = node_ids.len();
    let ph1: Vec<String> = (1..=n).map(|i| format!("?{i}")).collect();
    let ph2: Vec<String> = (n + 1..=2 * n).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq
         FROM edges
         WHERE (from_id IN ({}) OR to_id IN ({})) AND deleted_at IS NULL",
        ph1.join(","),
        ph2.join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for id in node_ids {
        param_values.push(Box::new(id.clone()));
    }
    for id in node_ids {
        param_values.push(Box::new(id.clone()));
    }
    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
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
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE id IN ({}) AND deleted_at IS NULL",
        placeholders.join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let nodes = stmt
        .query_map(params.as_slice(), row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NodeType, Relation};

    /// Ребро видно с обоих концов: на втором шаге BFS оно приходит со стороны
    /// соседа. Пока обход не помечал увиденное, `au context` печатал одну связь
    /// дважды, а MCP отдавал её дважды в JSON.
    #[test]
    fn traversal_returns_each_edge_once() {
        let path =
            std::env::temp_dir().join(format!("aurelius-traverse-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).expect("open temp db");

        let a = super::super::add_node(
            &conn,
            NodeType::Concept,
            "узел A",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("node a");
        let b = super::super::add_node(
            &conn,
            NodeType::Concept,
            "узел B",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("node b");
        super::super::add_edge(&conn, a.id, b.id, Relation::RelatedTo, 1.0).expect("edge");

        let (_, edges) = context_from_id(&conn, &a.id.to_string(), 3).expect("traverse");
        assert_eq!(edges.len(), 1, "одно ребро — одна запись в ответе");

        drop(conn);
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(p));
        }
    }
}
