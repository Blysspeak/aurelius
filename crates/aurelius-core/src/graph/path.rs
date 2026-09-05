//! Directed step-ladder queries over `next_step`/`prerequisite` edges (spec 008, FR-008..FR-011).

use crate::models::{Edge, Node, Relation};
use anyhow::{bail, Result};
use rusqlite::{params, Connection};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::{find_nodes_by_data_field, get_node, row_to_edge, row_to_node};

/// One hop of an ordered walk. `via`/`weight` describe the edge that led here;
/// both are meaningless for the start of a walk, where `via` is `None`.
#[derive(Debug, Clone)]
pub struct Step {
    pub index: usize,
    pub node: Node,
    pub via: Option<Relation>,
    pub weight: f32,
}

/// Result of [`before`]: ancestors of a target in topological order.
#[derive(Debug, Clone, Default)]
pub struct Before {
    pub ordered: Vec<Node>,
    pub cycle: bool,
}

/// FR-010: resolve a CLI/MCP argument to exactly one node — UUID, then exact
/// `data.subject`, then exact `label`, first stage with any hit wins. A stage
/// with more than one hit is an error naming the candidates: guessing which
/// one the caller meant would silently point a step ladder at the wrong node.
pub fn resolve_selector(conn: &Connection, selector: &str) -> Result<Node> {
    if let Ok(uuid) = Uuid::parse_str(selector) {
        if let Some(node) = get_node(conn, &uuid.to_string())? {
            return Ok(node);
        }
    }

    let by_subject = find_nodes_by_data_field(conn, "subject", selector, 50)?;
    match by_subject.as_slice() {
        [] => {}
        [only] => return Ok(only.clone()),
        many => bail!(
            "selector {selector:?} matches more than one node by subject: {}",
            describe_candidates(many)
        ),
    }

    let by_label = find_nodes_by_label_exact(conn, selector)?;
    match by_label.as_slice() {
        [] => bail!("no node matches {selector:?}"),
        [only] => Ok(only.clone()),
        many => bail!(
            "selector {selector:?} matches more than one node by label: {}",
            describe_candidates(many)
        ),
    }
}

fn describe_candidates(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| format!("{} ({})", n.id, n.label))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `find_node_by_label` in crud.rs stops at the first row, which is exactly
/// the "first one wins" behavior FR-010 forbids here — this variant collects
/// every live match so ambiguity can be reported instead of hidden.
fn find_nodes_by_label_exact(conn: &Connection, label: &str) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE label = ?1 AND deleted_at IS NULL",
    )?;
    let nodes = stmt
        .query_map(params![label], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Column list copied verbatim from `traverse::get_edges_batch` so both
/// queries stay in lockstep with `row_to_edge`'s positional `row.get` calls.
const EDGE_COLUMNS: &str =
    "id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq";

fn step_edges(conn: &Connection, node_id: Uuid, outgoing: bool) -> Result<Vec<Edge>> {
    let column = if outgoing { "from_id" } else { "to_id" };
    let sql = format!(
        "SELECT {EDGE_COLUMNS} FROM edges
         WHERE {column} = ?1 AND deleted_at IS NULL AND (relation = ?2 OR relation = ?3)"
    );
    let mut stmt = conn.prepare(&sql)?;
    let edges = stmt
        .query_map(
            params![
                node_id.to_string(),
                Relation::NextStep.to_string(),
                Relation::Prerequisite.to_string(),
            ],
            row_to_edge,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

/// FR-008: shortest directed path from `from` to `to` over `next_step` and
/// `prerequisite` edges. BFS explores each node's out-edges sorted by
/// `(weight, destination label)`, so the first time a node is discovered is
/// always via the preferred predecessor — that turns "shortest path" into
/// "shortest path, ties broken by weight then label" without a second pass.
pub fn shortest_path(
    conn: &Connection,
    from: &Node,
    to: &Node,
    max_depth: usize,
) -> Result<Option<Vec<Step>>> {
    if from.id == to.id {
        return Ok(Some(vec![Step {
            index: 0,
            node: from.clone(),
            via: None,
            weight: 0.0,
        }]));
    }

    // parent[id] = (predecessor id, relation and weight of the edge that reached it)
    let mut parent: HashMap<Uuid, (Uuid, Relation, f32)> = HashMap::new();
    let mut nodes: HashMap<Uuid, Node> = HashMap::new();
    nodes.insert(from.id, from.clone());

    let mut frontier = vec![from.id];
    let mut depth = 0usize;
    let mut found = nodes.contains_key(&to.id);

    while !found && depth < max_depth && !frontier.is_empty() {
        let mut next_frontier = vec![];
        for node_id in &frontier {
            let mut candidates: Vec<(Edge, Node)> = vec![];
            for edge in step_edges(conn, *node_id, true)? {
                let Some(dest) = get_node(conn, &edge.to_id.to_string())? else {
                    continue; // soft-deleted or missing endpoint — not a valid step
                };
                candidates.push((edge, dest));
            }
            candidates.sort_by(|(ea, na), (eb, nb)| {
                ea.weight
                    .partial_cmp(&eb.weight)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| na.label.cmp(&nb.label))
            });
            for (edge, dest) in candidates {
                if nodes.contains_key(&dest.id) {
                    continue; // already reached at an earlier (shorter or equal-preferred) hop
                }
                parent.insert(dest.id, (*node_id, edge.relation, edge.weight));
                let reached_target = dest.id == to.id;
                let dest_id = dest.id;
                nodes.insert(dest.id, dest);
                next_frontier.push(dest_id);
                if reached_target {
                    found = true;
                }
            }
        }
        frontier = next_frontier;
        depth += 1;
    }

    if !nodes.contains_key(&to.id) {
        return Ok(None);
    }

    // Walk parent pointers back from `to` to `from`, then reverse into forward order.
    let mut chain: Vec<(Uuid, Option<(Relation, f32)>)> = vec![(to.id, None)];
    let mut cursor = to.id;
    while cursor != from.id {
        let (pred, relation, weight) = parent.get(&cursor).cloned().ok_or_else(|| {
            anyhow::anyhow!("path reconstruction: {cursor} has no recorded parent")
        })?;
        let last = chain
            .last_mut()
            .ok_or_else(|| anyhow::anyhow!("path reconstruction: chain became empty"))?;
        last.1 = Some((relation, weight));
        chain.push((pred, None));
        cursor = pred;
    }
    chain.reverse();

    let mut steps = Vec::with_capacity(chain.len());
    for (index, (id, via)) in chain.into_iter().enumerate() {
        let weight = via.as_ref().map_or(0.0, |(_, w)| *w);
        let node = nodes
            .remove(&id)
            .ok_or_else(|| anyhow::anyhow!("path reconstruction: node {id} was not discovered"))?;
        steps.push(Step {
            index,
            node,
            via: via.map(|(r, _)| r),
            weight,
        });
    }
    Ok(Some(steps))
}

/// FR-009: every node with a directed transitive path into `target` via
/// `next_step`/`prerequisite`, earliest first. Walks the *predecessor* graph
/// (edges reversed) from `target` with three-way node coloring — White
/// (unseen), Gray (on the current walk, not yet finished), Black (finished
/// and already placed in `ordered`) — so a back edge to a Gray node reports a
/// cycle without recursing into it again, while re-reaching a Black node
/// (a converging DAG, not a cycle) is just skipped.
pub fn before(conn: &Connection, target: &Node, max_depth: usize) -> Result<Before> {
    let mut gray: HashSet<Uuid> = HashSet::new();
    let mut black: HashSet<Uuid> = HashSet::new();
    let mut ordered: Vec<Node> = vec![];
    let mut cycle = false;

    gray.insert(target.id);
    visit_predecessors(
        conn,
        target.id,
        0,
        max_depth,
        &mut gray,
        &mut black,
        &mut ordered,
        &mut cycle,
    )?;

    Ok(Before { ordered, cycle })
}

#[allow(clippy::too_many_arguments)]
fn visit_predecessors(
    conn: &Connection,
    node_id: Uuid,
    depth: usize,
    max_depth: usize,
    gray: &mut HashSet<Uuid>,
    black: &mut HashSet<Uuid>,
    ordered: &mut Vec<Node>,
    cycle: &mut bool,
) -> Result<()> {
    if depth >= max_depth {
        return Ok(());
    }
    for edge in step_edges(conn, node_id, false)? {
        let pred_id = edge.from_id;
        if black.contains(&pred_id) {
            continue; // already fully explored via another branch — convergence, not a cycle
        }
        if gray.contains(&pred_id) {
            *cycle = true; // back edge into a node still on the current walk
            continue;
        }
        let Some(pred_node) = get_node(conn, &pred_id.to_string())? else {
            continue; // soft-deleted or missing endpoint
        };
        gray.insert(pred_id);
        visit_predecessors(
            conn,
            pred_id,
            depth + 1,
            max_depth,
            gray,
            black,
            ordered,
            cycle,
        )?;
        gray.remove(&pred_id);
        black.insert(pred_id);
        ordered.push(pred_node);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::graph::{add_edge, add_node};
    use crate::models::NodeType;

    /// Same trick as `graph::crud` and `graph::traverse` tests: a real temp
    /// file, not `:memory:` — `db::open` hard-requires WAL, which SQLite's
    /// in-memory mode can never report.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-path-test-{tag}-{}.db",
                uuid::Uuid::new_v4()
            )))
        }
    }

    impl Drop for TmpDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.as_os_str().to_owned();
                p.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(p));
            }
        }
    }

    fn setup(tag: &str) -> (TmpDb, Connection) {
        let tmp = TmpDb::new(tag);
        let conn = db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    fn node(conn: &Connection, label: &str) -> Node {
        add_node(
            conn,
            NodeType::Doc,
            label,
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node")
    }

    /// A ->next_step(1)-> B ->next_step(2)-> C, D ->prerequisite-> C.
    fn ladder(conn: &Connection) -> (Node, Node, Node, Node) {
        let a = node(conn, "A");
        let b = node(conn, "B");
        let c = node(conn, "C");
        let d = node(conn, "D");
        add_edge(conn, a.id, b.id, Relation::NextStep, 1.0).expect("edge a-b");
        add_edge(conn, b.id, c.id, Relation::NextStep, 2.0).expect("edge b-c");
        add_edge(conn, d.id, c.id, Relation::Prerequisite, 1.0).expect("edge d-c");
        (a, b, c, d)
    }

    #[test]
    fn shortest_path_walks_the_ladder_in_order() {
        let (_tmp, conn) = setup("ladder-shortest");
        let (a, b, c, _d) = ladder(&conn);

        let path = shortest_path(&conn, &a, &c, 10)
            .expect("query ok")
            .expect("path exists");
        let labels: Vec<&str> = path.iter().map(|s| s.node.label.as_str()).collect();
        assert_eq!(labels, vec!["A", "B", "C"]);
        assert_eq!(
            path.iter().map(|s| s.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(path[0].via.is_none());
        assert_eq!(path[1].node.id, b.id);
    }

    #[test]
    fn shortest_path_reports_none_when_direction_does_not_lead_there() {
        let (_tmp, conn) = setup("ladder-none");
        let (a, _b, _c, d) = ladder(&conn);

        // D only points into C; nothing points from A towards D.
        let path = shortest_path(&conn, &a, &d, 10).expect("query ok");
        assert!(path.is_none());
    }

    #[test]
    fn before_collects_ordered_ancestors() {
        let (_tmp, conn) = setup("ladder-before");
        let (a, b, _c, d) = ladder(&conn);

        let result = before(&conn, &_c, 10).expect("query ok");
        assert!(!result.cycle);
        let ids: Vec<Uuid> = result.ordered.iter().map(|n| n.id).collect();
        assert!(
            ids.contains(&a.id),
            "A must be reported as an ancestor of C"
        );
        assert!(
            ids.contains(&b.id),
            "B must be reported as an ancestor of C"
        );
        assert!(
            ids.contains(&d.id),
            "D must be reported as an ancestor of C"
        );

        let pos_a = ids.iter().position(|id| *id == a.id).unwrap();
        let pos_b = ids.iter().position(|id| *id == b.id).unwrap();
        assert!(pos_a < pos_b, "A must come before B in topological order");
    }

    #[test]
    fn before_terminates_and_flags_a_cycle() {
        let (_tmp, conn) = setup("cycle");
        let a = node(&conn, "A");
        let b = node(&conn, "B");
        add_edge(&conn, a.id, b.id, Relation::NextStep, 1.0).expect("edge a-b");
        add_edge(&conn, b.id, a.id, Relation::NextStep, 1.0).expect("edge b-a");

        let result = before(&conn, &b, 10).expect("query ok — must terminate, not loop forever");
        assert!(result.cycle, "a 2-cycle must be flagged");
        assert_eq!(
            result.ordered.iter().filter(|n| n.id == a.id).count(),
            1,
            "A must be printed exactly once even though the cycle revisits it"
        );
    }

    #[test]
    fn resolve_selector_finds_by_uuid_subject_and_label() {
        let (_tmp, conn) = setup("resolve-happy");
        let n = add_node(
            &conn,
            NodeType::Doc,
            "session/create",
            None,
            "test",
            serde_json::json!({"subject": "https://docs.example/session#create"}),
        )
        .expect("add node");

        let by_uuid = resolve_selector(&conn, &n.id.to_string()).expect("by uuid");
        assert_eq!(by_uuid.id, n.id);

        let by_subject =
            resolve_selector(&conn, "https://docs.example/session#create").expect("by subject");
        assert_eq!(by_subject.id, n.id);

        let by_label = resolve_selector(&conn, "session/create").expect("by label");
        assert_eq!(by_label.id, n.id);
    }

    #[test]
    fn resolve_selector_rejects_ambiguous_subject_and_label() {
        let (_tmp, conn) = setup("resolve-ambiguous");
        add_node(
            &conn,
            NodeType::Doc,
            "dup",
            None,
            "test",
            serde_json::json!({"subject": "same-subject"}),
        )
        .expect("add first");
        add_node(
            &conn,
            NodeType::Doc,
            "dup",
            None,
            "test",
            serde_json::json!({"subject": "same-subject"}),
        )
        .expect("add second");

        let err = resolve_selector(&conn, "same-subject").expect_err("ambiguous subject");
        assert!(err.to_string().contains("more than one node by subject"));

        let err = resolve_selector(&conn, "dup").expect_err("ambiguous label");
        assert!(err.to_string().contains("more than one node by label"));
    }

    #[test]
    fn resolve_selector_reports_no_match() {
        let (_tmp, conn) = setup("resolve-miss");
        let err = resolve_selector(&conn, "nothing-here").expect_err("no match");
        assert!(err.to_string().contains("no node matches"));
    }
}
