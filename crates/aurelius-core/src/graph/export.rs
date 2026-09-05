//! Mermaid export of a (sub)graph (spec 008, FR-015).
//!
//! `--source-id` narrows the picture to one vendor-doc import instead of the
//! whole database: rendering everything past a handful of nodes stops being
//! a picture and starts being a wall.

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;
use uuid::Uuid;

use crate::models::Node;

use super::{get_all_edges, get_all_nodes};

/// `graph LR` body: one `id["label"]` line per node, one
/// `a -->|relation weight| b` per edge. Node ids are UUIDs with the dashes
/// stripped — mermaid's node-id grammar does not accept them.
///
/// Without `source_id` this is the whole graph; with it, only nodes whose
/// `data.source_id` matches and the edges that run between them (FR-006 keeps
/// doc nodes edge-free from the project/source hub, so no edge is ever lost
/// by this filter, only ones that would have crossed into an unrelated import).
pub fn mermaid(conn: &Connection, source_id: Option<&str>) -> Result<String> {
    let nodes = get_all_nodes(conn)?;
    let edges = get_all_edges(conn)?;

    let selected: Vec<&Node> = match source_id {
        Some(id) => nodes
            .iter()
            .filter(|n| n.data.get("source_id").and_then(|v| v.as_str()) == Some(id))
            .collect(),
        None => nodes.iter().collect(),
    };
    let selected_ids: HashSet<Uuid> = selected.iter().map(|n| n.id).collect();

    let mut out = String::from("graph LR\n");
    for node in &selected {
        out.push_str(&format!(
            "  {}[\"{}\"]\n",
            simple_id(node.id),
            escape_label(&node.label)
        ));
    }
    for edge in &edges {
        if !selected_ids.contains(&edge.from_id) || !selected_ids.contains(&edge.to_id) {
            continue;
        }
        out.push_str(&format!(
            "  {} -->|{} {}| {}\n",
            simple_id(edge.from_id),
            edge.relation,
            format_weight(edge.weight),
            simple_id(edge.to_id)
        ));
    }
    Ok(out)
}

fn simple_id(id: Uuid) -> String {
    id.simple().to_string()
}

fn escape_label(label: &str) -> String {
    label.replace('"', "\\\"")
}

/// Mermaid edge labels read as `next_step 2`, not `next_step 2.0` — a step
/// number is always a whole number, and the fractional form would be the
/// odd one out on every real import.
fn format_weight(weight: f32) -> String {
    if weight.fract() == 0.0 {
        format!("{}", weight as i64)
    } else {
        format!("{weight}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::graph::{add_edge, add_node};
    use crate::models::{NodeType, Relation};
    use serde_json::json;

    /// Same trick as the rest of `graph::*` tests: a real temp file, because
    /// `db::open` hard-requires WAL, which SQLite's `:memory:` mode never reports.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("aurelius-export-test-{tag}-{}.db", Uuid::new_v4())),
            )
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

    #[test]
    fn mermaid_renders_nodes_and_edges_of_the_whole_graph() {
        let (_tmp, conn) = setup("whole");
        let a = add_node(&conn, NodeType::Doc, "A", None, "test", json!({})).expect("node a");
        let b = add_node(&conn, NodeType::Doc, "B", None, "test", json!({})).expect("node b");
        add_edge(&conn, a.id, b.id, Relation::NextStep, 2.0).expect("edge a-b");

        let out = mermaid(&conn, None).expect("mermaid");

        assert!(out.starts_with("graph LR\n"));
        assert!(out.contains(&format!("{}[\"A\"]", a.id.simple())));
        assert!(out.contains(&format!("{}[\"B\"]", b.id.simple())));
        assert!(out.contains(&format!(
            "{} -->|next_step 2| {}",
            a.id.simple(),
            b.id.simple()
        )));
    }

    #[test]
    fn mermaid_source_id_filter_drops_other_sources_and_their_edges() {
        let (_tmp, conn) = setup("filter");
        let a = add_node(
            &conn,
            NodeType::Doc,
            "A",
            None,
            "test",
            json!({"source_id": "bank131-docs"}),
        )
        .expect("node a");
        let b = add_node(
            &conn,
            NodeType::Doc,
            "B",
            None,
            "test",
            json!({"source_id": "bank131-docs"}),
        )
        .expect("node b");
        let z = add_node(
            &conn,
            NodeType::Doc,
            "Z",
            None,
            "test",
            json!({"source_id": "other-docs"}),
        )
        .expect("node z");
        add_edge(&conn, a.id, b.id, Relation::NextStep, 1.0).expect("edge a-b");
        add_edge(&conn, b.id, z.id, Relation::References, 1.0).expect("edge b-z, wrong source");

        let out = mermaid(&conn, Some("bank131-docs")).expect("mermaid");

        assert!(out.contains(&format!("{}[\"A\"]", a.id.simple())));
        assert!(out.contains(&format!("{}[\"B\"]", b.id.simple())));
        assert!(
            !out.contains(&format!("{}[\"Z\"]", z.id.simple())),
            "node from another source leaked:\n{out}"
        );
        assert!(
            !out.contains("references"),
            "edge crossing into another source leaked:\n{out}"
        );
    }

    #[test]
    fn mermaid_escapes_quotes_in_labels() {
        let (_tmp, conn) = setup("escape");
        let a =
            add_node(&conn, NodeType::Doc, "say \"hi\"", None, "test", json!({})).expect("node a");

        let out = mermaid(&conn, None).expect("mermaid");

        assert!(out.contains(&format!("{}[\"say \\\"hi\\\"\"]", a.id.simple())));
    }
}
