//! Traversal caps (task c79b86c3): a hub node with hundreds of neighbors must
//! not expand into an unbounded answer, and an explicitly huge depth must not
//! walk deeper than the clamp. The fixture lives in a uuid-named temp database
//! built through `db::open`, so the live database is never touched; the
//! `-wal`/`-shm` siblings are removed with it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use aurelius_core::db;
use aurelius_core::graph;
use aurelius_core::models::{NodeType, Relation};

/// Unique FTS token for the hub fixture: only the hub carries it, so FTS
/// seeding picks exactly one seed and the walk spreads from it. Neighbor
/// labels deliberately avoid the token — otherwise they would become seeds
/// themselves and the walk would start from everywhere at once.
const HUB_TOPIC: &str = "qxzephyr";

/// Same trick for the path fixture used by the depth-clamp test.
const PATH_TOPIC: &str = "qwmirage";

struct TmpDb(std::path::PathBuf);

impl TmpDb {
    fn new(tag: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "aurelius-traverse-cap-{tag}-{}.db",
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

fn open(tag: &str) -> (TmpDb, rusqlite::Connection) {
    let tmp = TmpDb::new(tag);
    let conn = db::open(&tmp.0).expect("open temp db");
    (tmp, conn)
}

fn concept(conn: &rusqlite::Connection, label: &str) -> aurelius_core::models::Node {
    graph::add_node(
        conn,
        NodeType::Concept,
        label,
        None,
        "traverse-cap-test",
        serde_json::json!({}),
    )
    .expect("add node")
}

/// The hub blow-up the task was filed on: one central node with 500 neighbors
/// is the project-node shape. Depth 2 from that seed used to return all 501
/// nodes; the cap must stop it at 200 and the truncation report must account
/// for every hidden node (501 reachable - 200 returned = 301).
#[test]
fn hub_with_500_neighbors_is_capped_at_200_with_honest_report() {
    let (_tmp, conn) = open("hub");

    let hub = concept(&conn, &format!("{HUB_TOPIC} gravity well"));
    for i in 0..500 {
        let satellite = concept(&conn, &format!("satellite {i}"));
        graph::add_edge(&conn, hub.id, satellite.id, Relation::RelatedTo, 1.0)
            .expect("edge hub -> satellite");
    }

    let report = graph::context_with_report(&conn, HUB_TOPIC, 2).expect("traverse hub");
    assert!(
        report.nodes.len() <= graph::MAX_TRAVERSAL_NODES,
        "traversal returned {} nodes, cap is {}",
        report.nodes.len(),
        graph::MAX_TRAVERSAL_NODES
    );
    assert_eq!(report.nodes.len(), 200, "seed + 199 neighbors fit the cap");
    assert_eq!(report.hidden_nodes, 301, "the rest must be reported hidden");
    assert_eq!(
        report.truncated_at_depth,
        Some(1),
        "the cap cuts during the first BFS expansion"
    );

    // The plain tuple API every caller uses must inherit the same cap.
    let (nodes, _) = graph::context(&conn, HUB_TOPIC, 2).expect("traverse hub tuple");
    assert!(
        nodes.len() <= graph::MAX_TRAVERSAL_NODES,
        "tuple API returned {} nodes, cap is {}",
        nodes.len(),
        graph::MAX_TRAVERSAL_NODES
    );
}

/// A path long enough that an unclamped depth 99 reaches all 6 nodes while
/// depth 3 reaches only 4. After the clamp both walks must return the same
/// node set — pre-fix this failed with 6 != 4.
#[test]
fn depth_above_clamp_behaves_like_depth_3() {
    let (_tmp, conn) = open("clamp");

    let first = concept(&conn, &format!("{PATH_TOPIC} origin"));
    let mut prev = first.id;
    for i in 2..=6 {
        let relay = concept(&conn, &format!("relay {i}"));
        graph::add_edge(&conn, prev, relay.id, Relation::RelatedTo, 1.0).expect("edge relay chain");
        prev = relay.id;
    }

    let deep = graph::context(&conn, PATH_TOPIC, 99).expect("deep walk");
    let clamped = graph::context(&conn, PATH_TOPIC, 3).expect("clamped walk");

    let deep_ids: std::collections::HashSet<_> = deep.0.iter().map(|n| n.id).collect();
    let clamped_ids: std::collections::HashSet<_> = clamped.0.iter().map(|n| n.id).collect();
    assert_eq!(
        deep_ids, clamped_ids,
        "depth 99 must return exactly what depth 3 returns"
    );
    assert_eq!(
        clamped.0.len(),
        4,
        "depth 3 from the origin reaches 4 nodes"
    );
}
