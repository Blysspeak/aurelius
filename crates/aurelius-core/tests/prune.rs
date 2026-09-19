//! `au db prune`: три правила и ни одного больше. Узел с `claim` не снимается
//! никогда, пробный план ничего не трогает, `--apply` снимает ровно план.
//! База — временный файл через `db::open`, живая не задета.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use aurelius_core::db;
use aurelius_core::graph::{self, PruneRule};
use aurelius_core::models::{Node, NodeType, Relation};
use serde_json::json;

struct TmpDb(std::path::PathBuf);

impl Drop for TmpDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut p = self.0.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(p));
        }
    }
}

fn open() -> (TmpDb, rusqlite::Connection) {
    let tmp =
        TmpDb(std::env::temp_dir().join(format!("aurelius-prune-{}.db", uuid::Uuid::new_v4())));
    let conn = db::open(&tmp.0).expect("open temp db");
    (tmp, conn)
}

fn add(
    conn: &rusqlite::Connection,
    t: NodeType,
    label: &str,
    note: Option<&str>,
    data: serde_json::Value,
) -> Node {
    graph::add_node(conn, t, label, note, "test", data).expect("add node")
}

fn live(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )
    .expect("count")
}

#[test]
fn prune_takes_exactly_the_three_rules_and_never_a_claim() {
    let (_tmp, conn) = open();

    // Правило 1: технический узел без рёбер и без содержания.
    let orphan_run = add(&conn, NodeType::Run, "прогон: cargo test", None, json!({}));
    let orphan_dep = add(&conn, NodeType::Dependency, "rust-embed", None, json!({}));
    // Не правило 1: ребро, claim, тело.
    let task = add(&conn, NodeType::Task, "[demo] задача", None, json!({}));
    let linked_run = add(&conn, NodeType::Run, "прогон: au search", None, json!({}));
    graph::add_edge(&conn, task.id, linked_run.id, Relation::VerifiedBy, 1.0).expect("edge");
    let claimed_run = add(
        &conn,
        NodeType::Run,
        "прогон с выводом",
        None,
        json!({"claim": "exit 0"}),
    );
    let noted_dep = add(
        &conn,
        NodeType::Dependency,
        "fastembed",
        Some("закреплена на 6.1"),
        json!({}),
    );

    // Правило 2: не самый свежий дистиллят проекта.
    let old_digest = add(
        &conn,
        NodeType::Digest,
        "[demo] дистиллят",
        Some("старый"),
        json!({"project": "demo"}),
    );
    let new_digest = add(
        &conn,
        NodeType::Digest,
        "[demo] дистиллят",
        Some("новый"),
        json!({"project": "demo"}),
    );
    add(
        &conn,
        NodeType::Digest,
        "[solo] дистиллят",
        Some("один"),
        json!({"project": "solo"}),
    );

    // Правило 3: пустой узел проекта, которого никто не называет своим.
    let empty_project = add(&conn, NodeType::Project, "пустой", None, json!({}));
    let owned_project = add(&conn, NodeType::Project, "занятой", None, json!({}));
    let owned_decision = add(
        &conn,
        NodeType::Decision,
        "[занятой] решение",
        Some("тело"),
        json!({}),
    );
    let demo = add(&conn, NodeType::Project, "demo", None, json!({}));
    let linked_decision = add(
        &conn,
        NodeType::Decision,
        "решение",
        None,
        json!({"claim": "взяли sqlite"}),
    );
    graph::add_edge(&conn, linked_decision.id, demo.id, Relation::BelongsTo, 1.0).expect("edge");
    // Знание без связей: считается, не удаляется.
    let lonely = add(
        &conn,
        NodeType::Decision,
        "одинокое",
        None,
        json!({"claim": "без рёбер"}),
    );

    let before = live(&conn);
    let plan = graph::prune_plan(&conn).expect("plan");
    assert_eq!(live(&conn), before, "пробный план ничего не трогает");

    let ids = |rule: PruneRule| {
        let mut ids: Vec<_> = plan
            .candidates
            .iter()
            .filter(|c| c.rule == rule)
            .map(|c| c.id)
            .collect();
        ids.sort();
        ids
    };
    let mut technical = vec![orphan_run.id, orphan_dep.id];
    technical.sort();
    assert_eq!(ids(PruneRule::TechnicalOrphan), technical);
    assert_eq!(ids(PruneRule::StaleDigest), vec![old_digest.id]);
    assert_eq!(ids(PruneRule::EmptyProject), vec![empty_project.id]);

    let kept = [
        linked_run.id,
        claimed_run.id,
        noted_dep.id,
        new_digest.id,
        owned_project.id,
        owned_decision.id,
        lonely.id,
    ];
    assert!(plan.candidates.iter().all(|c| !kept.contains(&c.id)));
    assert_eq!(plan.unlinked_knowledge.get("decision"), Some(&2));
    assert_eq!(plan.unlinked_knowledge.get("dependency"), Some(&1));
    assert_eq!(plan.unlinked_knowledge.get("run"), Some(&1));

    let applied = graph::prune_apply(&conn).expect("apply");
    assert_eq!(applied.candidates.len(), 4);
    assert_eq!(live(&conn), before - 4);
    assert!(graph::prune_plan(&conn)
        .expect("replan")
        .candidates
        .is_empty());
}
