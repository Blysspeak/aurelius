//! `memory_path` — the step ladder query surfaced over MCP (spec 008, FR-011).
//!
//! Same question `au path` answers, same core functions
//! (`graph::resolve_selector`/`shortest_path`/`before`) — this is the second
//! door, not a second implementation.

use anyhow::Result;
use aurelius_core::graph;
use serde_json::json;

use super::open_db;

/// Same default as `au path --max-depth` — a caller that omits it should get
/// the same answer from either door.
const DEFAULT_MAX_DEPTH: u64 = 50;

pub fn memory_path(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    memory_path_with_conn(&conn, params)
}

/// Body with an explicit connection — same testability trick as
/// `memory_context_with_conn`: tests seed a graph and call this directly,
/// without going through the live database file.
fn memory_path_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let from = params.get("from").and_then(|v| v.as_str());
    let to = params.get("to").and_then(|v| v.as_str());
    let before = params.get("before").and_then(|v| v.as_str());
    let max_depth = params
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_DEPTH) as usize;

    match (from, to, before) {
        (Some(from), Some(to), None) => path_between(conn, from, to, max_depth),
        (None, None, Some(before)) => path_before(conn, before, max_depth),
        _ => anyhow::bail!(
            "memory_path: pass either (from and to) or before — not both, not neither"
        ),
    }
}

/// A miss is data, not a broken call: the caller asked a legitimate question
/// ("is there a path") and got a legitimate answer ("no"). `au path` reports
/// the same case on stderr with exit code 1 — MCP has no exit code, so the
/// negative result rides in the JSON body as `error` instead of failing the
/// whole tool call (isError). A caller that treats `error` as "there is no
/// such ladder" and a genuinely malformed call (unresolvable selector,
/// missing parameter) as a tool error can tell the two apart.
fn path_between(
    conn: &rusqlite::Connection,
    from: &str,
    to: &str,
    max_depth: usize,
) -> Result<serde_json::Value> {
    let from_node = graph::resolve_selector(conn, from)?;
    let to_node = graph::resolve_selector(conn, to)?;
    let Some(steps) = graph::shortest_path(conn, &from_node, &to_node, max_depth)? else {
        return Ok(json!({ "error": format!("no path from {from} to {to}") }));
    };

    Ok(json!({
        "steps": steps.iter().map(|s| json!({
            "index": s.index,
            "id": s.node.id.to_string(),
            "label": s.node.label,
            "subject": s.node.data.get("subject"),
            "relation": s.via.as_ref().map(ToString::to_string),
            "weight": s.weight,
        })).collect::<Vec<_>>(),
    }))
}

fn path_before(
    conn: &rusqlite::Connection,
    target: &str,
    max_depth: usize,
) -> Result<serde_json::Value> {
    let target_node = graph::resolve_selector(conn, target)?;
    let result = graph::before(conn, &target_node, max_depth)?;

    Ok(json!({
        "before": result.ordered.iter().map(|n| json!({
            "id": n.id.to_string(),
            "label": n.label,
            "subject": n.data.get("subject"),
        })).collect::<Vec<_>>(),
        "cycle": result.cycle,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelius_core::db;
    use aurelius_core::graph::{add_edge, add_node};
    use aurelius_core::models::{NodeType, Relation};
    use uuid::Uuid;

    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-mcp-path-test-{tag}-{}.db",
                Uuid::new_v4()
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

    fn setup(tag: &str) -> (TmpDb, rusqlite::Connection) {
        let tmp = TmpDb::new(tag);
        let conn = db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    #[test]
    fn returns_ordered_steps_for_a_next_step_chain() {
        let (_tmp, conn) = setup("chain");
        let a = add_node(&conn, NodeType::Doc, "A", None, "test", json!({})).expect("a");
        let b = add_node(&conn, NodeType::Doc, "B", None, "test", json!({})).expect("b");
        add_edge(&conn, a.id, b.id, Relation::NextStep, 1.0).expect("edge");

        let out =
            memory_path_with_conn(&conn, &json!({"from": "A", "to": "B"})).expect("memory_path");
        let steps = out["steps"].as_array().expect("steps array");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1]["relation"], "next_step");
        assert_eq!(steps[1]["label"], "B");
    }

    #[test]
    fn missing_path_is_a_normal_json_result_not_a_tool_error() {
        let (_tmp, conn) = setup("miss");
        add_node(&conn, NodeType::Doc, "A", None, "test", json!({})).expect("a");
        add_node(&conn, NodeType::Doc, "B", None, "test", json!({})).expect("b");

        let out =
            memory_path_with_conn(&conn, &json!({"from": "A", "to": "B"})).expect("memory_path");
        assert!(
            out["error"].as_str().is_some(),
            "expected error field: {out}"
        );
    }

    #[test]
    fn before_reports_ancestors_and_cycle_flag() {
        let (_tmp, conn) = setup("before");
        let a = add_node(&conn, NodeType::Doc, "A", None, "test", json!({})).expect("a");
        let b = add_node(&conn, NodeType::Doc, "B", None, "test", json!({})).expect("b");
        add_edge(&conn, a.id, b.id, Relation::Prerequisite, 1.0).expect("edge");

        let out = memory_path_with_conn(&conn, &json!({"before": "B"})).expect("memory_path");
        assert_eq!(out["cycle"], false);
        let before = out["before"].as_array().expect("before array");
        assert!(before.iter().any(|n| n["label"] == "A"));
    }

    #[test]
    fn neither_form_nor_both_is_refused() {
        let (_tmp, conn) = setup("bad-call");
        let err = memory_path_with_conn(&conn, &json!({})).expect_err("must fail");
        assert!(err.to_string().contains("memory_path"));
    }
}
