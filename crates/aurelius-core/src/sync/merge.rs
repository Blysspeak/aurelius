//! Server-side push/pull merge logic: upsert-by-`id` with last-writer-wins
//! (by `updated_at`) conflict resolution, per `specs/002-project-sync/data-model.md`
//! and `contracts/sync-api.md`. Used only by `aurelius-sync-server`.

use super::{SyncPullResponse, SyncPushResponse};
use crate::graph::{row_to_edge, row_to_node};
use crate::models::{Edge, Node, NodeType, Relation};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;

const NODE_SELECT: &str = "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes";

const EDGE_SELECT: &str =
    "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq
         FROM edges";

enum PushOutcome {
    /// New row inserted, or an existing row overwritten by a newer edit.
    Accepted,
    /// The server's existing copy won; the client's losing edit was stashed.
    Conflict,
    /// Nothing changed (e.g. a tombstone re-pushed after it already landed).
    NoOp,
}

/// Applies a client's push to this project: upserts every node/edge by
/// `id`, using last-writer-wins by `updated_at` for nodes, with the losing
/// version retained under `data._sync_conflict` on the winning row. Soft
/// deletes are sticky — never revived by a later "live" push, matching the
/// one-way `live -> deleted` state transition in data-model.md.
pub fn apply_push(
    conn: &Connection,
    project: &str,
    nodes: &[Node],
    edges: &[Edge],
) -> Result<SyncPushResponse> {
    for node in nodes {
        if node.created_by.is_none() {
            anyhow::bail!("node {} missing required created_by attribution", node.id);
        }
    }
    for edge in edges {
        if edge.created_by.is_none() {
            anyhow::bail!("edge {} missing required created_by attribution", edge.id);
        }
    }

    validate_project_membership(conn, project, nodes, edges)?;

    let mut seq = current_max_seq(conn)?;
    let mut accepted = 0usize;
    let mut conflicts = 0usize;

    for node in nodes {
        match upsert_node(conn, node, &mut seq)? {
            PushOutcome::Accepted => accepted += 1,
            PushOutcome::Conflict => conflicts += 1,
            PushOutcome::NoOp => {}
        }
    }
    for edge in edges {
        match upsert_edge(conn, edge, &mut seq)? {
            PushOutcome::Accepted => accepted += 1,
            PushOutcome::Conflict => conflicts += 1,
            PushOutcome::NoOp => {}
        }
    }

    tracing::debug!(
        project,
        accepted,
        conflicts,
        server_seq = seq,
        "sync push applied"
    );

    Ok(SyncPushResponse {
        accepted,
        conflicts,
        server_seq: seq,
    })
}

/// Rejects a push that touches anything outside `project` — a collaborator
/// token is authenticated to exactly one project (see `auth.rs`), and
/// without this check that scoping would be advisory only: a valid
/// credential for project A could inject or overwrite nodes/edges tagged as
/// belonging to a completely different project B hosted on the same server.
///
/// A node counts as "in" the project if it already has a `belongs_to` edge
/// to the project node server-side, or if this exact push batch introduces
/// that edge (the normal case — a client pushes a new node together with
/// the edge that links it). The project node's very first push (bootstrap)
/// is the one case where the project node itself arrives in `nodes`.
fn validate_project_membership(
    conn: &Connection,
    project: &str,
    nodes: &[Node],
    edges: &[Edge],
) -> Result<()> {
    if nodes.is_empty() && edges.is_empty() {
        return Ok(());
    }

    let existing_project_id = crate::graph::find_project_by_label(conn, project)?.map(|n| n.id);
    let incoming_project_id = nodes
        .iter()
        .find(|n| matches!(n.node_type, NodeType::Project) && n.label == project)
        .map(|n| n.id);

    let project_id = match existing_project_id.or(incoming_project_id) {
        Some(id) => id,
        None => anyhow::bail!(
            "push rejected: project '{project}' does not exist on the server, and this push does not create it"
        ),
    };

    let mut allowed: HashSet<Uuid> = HashSet::from([project_id]);
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT from_id FROM edges WHERE to_id = ?1 AND relation = 'belongs_to'",
        )?;
        let rows = stmt.query_map(params![project_id.to_string()], |r| r.get::<_, String>(0))?;
        for id in rows {
            if let Ok(uuid) = Uuid::parse_str(&id?) {
                allowed.insert(uuid);
            }
        }
    }
    for edge in edges {
        if matches!(edge.relation, Relation::BelongsTo) && edge.to_id == project_id {
            allowed.insert(edge.from_id);
        }
    }

    for node in nodes {
        if node.id != project_id && !allowed.contains(&node.id) {
            anyhow::bail!(
                "push rejected: node {} is not linked to project '{project}' — a token can only push data for the project it was issued for",
                node.id
            );
        }
    }
    for edge in edges {
        let from_ok = edge.from_id == project_id || allowed.contains(&edge.from_id);
        let to_ok = edge.to_id == project_id || allowed.contains(&edge.to_id);
        if !from_ok || !to_ok {
            anyhow::bail!(
                "push rejected: edge {} references a node outside project '{project}'",
                edge.id
            );
        }
    }

    Ok(())
}

/// Returns every live and tombstoned node/edge belonging to `project` with
/// `sync_seq > since_seq` (a `since_seq` of `0` therefore returns the full
/// bootstrap history, since every server-side row carries `sync_seq >= 1`).
pub fn pull_since(conn: &Connection, project: &str, since_seq: i64) -> Result<SyncPullResponse> {
    let server_seq = current_max_seq(conn)?;

    let project_node = match crate::graph::find_project_by_label(conn, project)? {
        Some(p) => p,
        None => {
            return Ok(SyncPullResponse {
                project: project.to_string(),
                nodes: vec![],
                edges: vec![],
                server_seq,
            })
        }
    };
    let project_id = project_node.id.to_string();

    // Membership follows the existing `belongs_to` convention: every
    // decision/task/session/etc. gets a `belongs_to` edge into its project
    // node when created (see `au`/`aurelius`'s create paths).
    let mut member_ids: Vec<String> = vec![project_id.clone()];
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT from_id FROM edges WHERE to_id = ?1 AND relation = 'belongs_to'",
        )?;
        let rows = stmt.query_map(params![project_id], |r| r.get::<_, String>(0))?;
        for id in rows {
            member_ids.push(id?);
        }
    }

    let placeholders: Vec<String> = (1..=member_ids.len()).map(|i| format!("?{i}")).collect();
    let since_idx = member_ids.len() + 1;
    let in_list = placeholders.join(",");

    let mut node_params: Vec<Box<dyn rusqlite::types::ToSql>> = member_ids
        .iter()
        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    node_params.push(Box::new(since_seq));
    let node_param_refs: Vec<&dyn rusqlite::types::ToSql> =
        node_params.iter().map(|p| p.as_ref()).collect();

    let node_sql = format!("{NODE_SELECT} WHERE id IN ({in_list}) AND sync_seq > ?{since_idx}");
    let mut stmt = conn.prepare(&node_sql)?;
    let nodes = stmt
        .query_map(node_param_refs.as_slice(), row_to_node)?
        .collect::<rusqlite::Result<Vec<Node>>>()?;

    let edge_sql = format!("{EDGE_SELECT} WHERE from_id IN ({in_list}) AND to_id IN ({in_list}) AND sync_seq > ?{since_idx}");
    let mut stmt = conn.prepare(&edge_sql)?;
    let edges = stmt
        .query_map(node_param_refs.as_slice(), row_to_edge)?
        .collect::<rusqlite::Result<Vec<Edge>>>()?;

    Ok(SyncPullResponse {
        project: project.to_string(),
        nodes,
        edges,
        server_seq,
    })
}

fn current_max_seq(conn: &Connection) -> Result<i64> {
    let max_nodes: Option<i64> =
        conn.query_row("SELECT MAX(sync_seq) FROM nodes", [], |r| r.get(0))?;
    let max_edges: Option<i64> =
        conn.query_row("SELECT MAX(sync_seq) FROM edges", [], |r| r.get(0))?;
    Ok(max_nodes.unwrap_or(0).max(max_edges.unwrap_or(0)))
}

fn fetch_node(conn: &Connection, id: &str) -> Result<Option<Node>> {
    let sql = format!("{NODE_SELECT} WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(params![id], row_to_node)?;
    Ok(rows.next().transpose()?)
}

fn fetch_edge(conn: &Connection, id: &str) -> Result<Option<Edge>> {
    let sql = format!("{EDGE_SELECT} WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(params![id], row_to_edge)?;
    Ok(rows.next().transpose()?)
}

fn upsert_node(conn: &Connection, node: &Node, seq: &mut i64) -> Result<PushOutcome> {
    let id_str = node.id.to_string();
    let existing = fetch_node(conn, &id_str)?;

    match existing {
        None => {
            *seq += 1;
            insert_node_row(conn, node, *seq)?;
            Ok(PushOutcome::Accepted)
        }
        Some(existing) if existing.deleted_at.is_some() => {
            // Tombstones are sticky: a soft-deleted row is never revived by
            // sync, even by a chronologically newer "live" push.
            if node.deleted_at.is_some() {
                Ok(PushOutcome::NoOp)
            } else {
                stash_conflict_onto_existing(conn, &id_str, &existing, node)?;
                Ok(PushOutcome::Conflict)
            }
        }
        Some(existing) if node.updated_at > existing.updated_at => {
            *seq += 1;
            let mut data = node.data.clone();
            stash_conflict(
                &mut data,
                existing.note.clone(),
                existing.data.clone(),
                existing.updated_by.clone(),
                existing.updated_at,
            );
            replace_node_row(conn, node, &data, *seq)?;
            Ok(PushOutcome::Accepted)
        }
        // Equal `updated_at` means the client is re-sending a version it
        // already has in sync (e.g. re-pushing its full project membership
        // set, which includes nodes it only ever pulled and never edited) —
        // not a genuine race. Treat as a true no-op: no data mutation, no
        // conflict recorded. A real conflict requires a strictly older,
        // divergent edit (handled below).
        Some(existing) if node.updated_at == existing.updated_at => Ok(PushOutcome::NoOp),
        Some(existing) => {
            stash_conflict_onto_existing(conn, &id_str, &existing, node)?;
            Ok(PushOutcome::Conflict)
        }
    }
}

/// Server's existing copy wins: leaves it in place but stashes the client's
/// losing edit under `data._sync_conflict` for recovery.
fn stash_conflict_onto_existing(
    conn: &Connection,
    id: &str,
    existing: &Node,
    losing: &Node,
) -> Result<()> {
    let mut data = existing.data.clone();
    stash_conflict(
        &mut data,
        losing.note.clone(),
        losing.data.clone(),
        losing.updated_by.clone(),
        losing.updated_at,
    );
    conn.execute(
        "UPDATE nodes SET data = ?1 WHERE id = ?2",
        params![serde_json::to_string(&data)?, id],
    )?;
    Ok(())
}

fn stash_conflict(
    data: &mut serde_json::Value,
    loser_note: Option<String>,
    loser_data: serde_json::Value,
    loser_updated_by: Option<String>,
    loser_updated_at: DateTime<Utc>,
) {
    if !data.is_object() {
        *data = json!({});
    }
    if let Some(obj) = data.as_object_mut() {
        obj.insert(
            "_sync_conflict".to_string(),
            json!({
                "note": loser_note,
                "data": loser_data,
                "updated_by": loser_updated_by,
                "updated_at": loser_updated_at.to_rfc3339(),
            }),
        );
    }
}

fn insert_node_row(conn: &Connection, node: &Node, seq: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO nodes (id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
            node.created_by,
            node.updated_by,
            node.deleted_at.map(|d| d.to_rfc3339()),
            seq,
        ],
    )?;
    Ok(())
}

fn replace_node_row(
    conn: &Connection,
    node: &Node,
    data: &serde_json::Value,
    seq: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE nodes SET node_type = ?1, label = ?2, note = ?3, source = ?4, data = ?5,
                created_at = ?6, updated_at = ?7, memory_kind = ?8, last_accessed_at = ?9,
                access_count = ?10, content_hash = ?11, created_by = ?12, updated_by = ?13,
                deleted_at = ?14, sync_seq = ?15
         WHERE id = ?16",
        params![
            serde_json::to_string(&node.node_type)?,
            node.label,
            node.note,
            node.source,
            serde_json::to_string(data)?,
            node.created_at.to_rfc3339(),
            node.updated_at.to_rfc3339(),
            node.memory_kind.to_string(),
            node.last_accessed_at.to_rfc3339(),
            node.access_count,
            node.content_hash,
            node.created_by,
            node.updated_by,
            node.deleted_at.map(|d| d.to_rfc3339()),
            seq,
            node.id.to_string(),
        ],
    )?;
    Ok(())
}

fn upsert_edge(conn: &Connection, edge: &Edge, seq: &mut i64) -> Result<PushOutcome> {
    let id_str = edge.id.to_string();
    let existing = fetch_edge(conn, &id_str)?;

    match existing {
        None => {
            *seq += 1;
            insert_edge_row(conn, edge, *seq)?;
            Ok(PushOutcome::Accepted)
        }
        Some(existing) if edge.deleted_at.is_some() && existing.deleted_at.is_none() => {
            *seq += 1;
            conn.execute(
                "UPDATE edges SET deleted_at = ?1, sync_seq = ?2 WHERE id = ?3",
                params![edge.deleted_at.map(|d| d.to_rfc3339()), *seq, id_str],
            )?;
            Ok(PushOutcome::Accepted)
        }
        Some(_) => Ok(PushOutcome::NoOp),
    }
}

fn insert_edge_row(conn: &Connection, edge: &Edge, seq: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO edges (id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            edge.id.to_string(),
            edge.from_id.to_string(),
            edge.to_id.to_string(),
            edge.relation.to_string(),
            edge.weight,
            edge.created_at.to_rfc3339(),
            edge.created_by,
            edge.deleted_at.map(|d| d.to_rfc3339()),
            seq,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::{MemoryKind, NodeType};
    use serde_json::json;
    use uuid::Uuid;

    /// A real temp-file database, not `:memory:` — SQLite's in-memory mode
    /// can never report journal_mode=WAL, and `db::open`'s `ensure_wal` gate
    /// now hard-rejects anything else. Kept alive for the test's duration so
    /// the file isn't removed out from under the open `Connection`; cleans
    /// up its `-wal`/`-shm` siblings on drop.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("aurelius-merge-test-{tag}-{}.db", Uuid::new_v4())),
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

    /// Seeds a "proj" project node directly (bypassing push, which now
    /// requires project membership) so tests can push nodes scoped to it.
    fn setup() -> (TmpDb, Connection, Uuid) {
        let tmp = TmpDb::new("setup");
        let conn = db::open(&tmp.0).expect("open temp db");
        let project =
            crate::graph::add_node(&conn, NodeType::Project, "proj", None, "test", json!({}))
                .expect("seed project");
        (tmp, conn, project.id)
    }

    /// A `belongs_to` edge, as a real client would push alongside a new
    /// node's very first push — this is what makes the node "in" the
    /// project for `validate_project_membership`.
    fn belongs_to(node_id: Uuid, project_id: Uuid) -> Edge {
        Edge {
            id: Uuid::new_v4(),
            from_id: node_id,
            to_id: project_id,
            relation: Relation::BelongsTo,
            weight: 1.0,
            created_at: Utc::now(),
            created_by: Some("Alice <alice@example.com>".to_string()),
            deleted_at: None,
            sync_seq: None,
        }
    }

    fn make_node(label: &str, note: &str, updated_at: DateTime<Utc>) -> Node {
        let now = updated_at;
        Node {
            id: Uuid::new_v4(),
            node_type: NodeType::Decision,
            label: label.to_string(),
            note: Some(note.to_string()),
            source: "test".to_string(),
            data: json!({}),
            created_at: now,
            updated_at: now,
            memory_kind: MemoryKind::Semantic,
            last_accessed_at: now,
            access_count: 0,
            content_hash: None,
            created_by: Some("Alice <alice@example.com>".to_string()),
            updated_by: Some("Alice <alice@example.com>".to_string()),
            deleted_at: None,
            sync_seq: None,
        }
    }

    #[test]
    fn new_id_inserts_and_gets_sync_seq() {
        let (_tmp, conn, project_id) = setup();
        let node = make_node("decision-1", "first note", Utc::now());
        let edge = belongs_to(node.id, project_id);

        let resp = apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&node),
            std::slice::from_ref(&edge),
        )
        .expect("push ok");

        assert_eq!(resp.accepted, 2);
        assert_eq!(resp.conflicts, 0);
        assert_eq!(resp.server_seq, 2);

        let stored = fetch_node(&conn, &node.id.to_string()).unwrap().unwrap();
        assert_eq!(stored.note.as_deref(), Some("first note"));
        assert_eq!(stored.sync_seq, Some(1));
    }

    #[test]
    fn newer_write_overwrites_older() {
        let (_tmp, conn, project_id) = setup();
        let t0 = Utc::now() - chrono::Duration::seconds(10);
        let original = make_node("decision-1", "v1", t0);
        let edge = belongs_to(original.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&original),
            std::slice::from_ref(&edge),
        )
        .expect("initial push ok");

        let mut updated = original.clone();
        updated.note = Some("v2".to_string());
        updated.updated_at = Utc::now();
        updated.updated_by = Some("Bob <bob@example.com>".to_string());

        let resp =
            apply_push(&conn, "proj", std::slice::from_ref(&updated), &[]).expect("update push ok");

        assert_eq!(resp.accepted, 1);
        assert_eq!(resp.conflicts, 0);

        let stored = fetch_node(&conn, &original.id.to_string())
            .unwrap()
            .unwrap();
        assert_eq!(stored.note.as_deref(), Some("v2"));
        assert_eq!(stored.updated_by.as_deref(), Some("Bob <bob@example.com>"));
        assert_eq!(stored.sync_seq, Some(3));
    }

    #[test]
    fn older_write_is_a_noop() {
        let (_tmp, conn, project_id) = setup();
        let t0 = Utc::now() - chrono::Duration::seconds(10);
        let mut current = make_node("decision-1", "current", Utc::now());
        current.updated_at = Utc::now();
        let edge = belongs_to(current.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&current),
            std::slice::from_ref(&edge),
        )
        .expect("initial push ok");

        let mut stale = current.clone();
        stale.note = Some("stale".to_string());
        stale.updated_at = t0; // older than `current`'s updated_at

        let resp =
            apply_push(&conn, "proj", std::slice::from_ref(&stale), &[]).expect("stale push ok");

        assert_eq!(resp.accepted, 0);
        assert_eq!(resp.conflicts, 1);

        let stored = fetch_node(&conn, &current.id.to_string()).unwrap().unwrap();
        // server's copy is still current — the stale push did not overwrite it.
        assert_eq!(stored.note.as_deref(), Some("current"));
        assert_eq!(stored.sync_seq, Some(1)); // no new seq assigned for a no-op
    }

    #[test]
    fn conflict_retention_field_populated_on_real_conflict() {
        let (_tmp, conn, project_id) = setup();
        let t0 = Utc::now() - chrono::Duration::seconds(10);
        let mut current = make_node("decision-1", "current", Utc::now());
        current.updated_at = Utc::now();
        let edge = belongs_to(current.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&current),
            std::slice::from_ref(&edge),
        )
        .expect("initial push ok");

        let mut stale = current.clone();
        stale.note = Some("lost edit".to_string());
        stale.data = json!({"detail": "from the losing side"});
        stale.updated_by = Some("Carol <carol@example.com>".to_string());
        stale.updated_at = t0;

        apply_push(&conn, "proj", std::slice::from_ref(&stale), &[]).expect("stale push ok");

        let stored = fetch_node(&conn, &current.id.to_string()).unwrap().unwrap();
        let conflict = stored.data.get("_sync_conflict").expect("conflict stashed");
        assert_eq!(
            conflict.get("note").and_then(|v| v.as_str()),
            Some("lost edit")
        );
        assert_eq!(
            conflict.get("updated_by").and_then(|v| v.as_str()),
            Some("Carol <carol@example.com>")
        );
        assert_eq!(
            conflict
                .get("data")
                .and_then(|v| v.get("detail"))
                .and_then(|v| v.as_str()),
            Some("from the losing side")
        );
    }

    /// A client re-pushing its full project membership set (unconditionally,
    /// not just deltas) will resend nodes it only ever pulled and never
    /// edited — same `updated_at` it already has in sync. That must be a
    /// true no-op, not a spurious conflict: no `_sync_conflict` stashed, no
    /// data mutation, `conflicts` stays 0.
    #[test]
    fn identical_repush_with_equal_updated_at_is_a_true_noop() {
        let (_tmp, conn, project_id) = setup();
        let node = make_node("decision-1", "unchanged", Utc::now());
        let edge = belongs_to(node.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&node),
            std::slice::from_ref(&edge),
        )
        .expect("initial push ok");

        // Same node, same updated_at, as if it were pulled and pushed back
        // unmodified.
        let resp = apply_push(&conn, "proj", std::slice::from_ref(&node), &[]).expect("repush ok");

        assert_eq!(resp.accepted, 0);
        assert_eq!(resp.conflicts, 0);

        let stored = fetch_node(&conn, &node.id.to_string()).unwrap().unwrap();
        assert_eq!(stored.note.as_deref(), Some("unchanged"));
        assert!(
            stored.data.get("_sync_conflict").is_none(),
            "an identical re-push must not stash a bogus conflict"
        );
        assert_eq!(stored.sync_seq, Some(1), "no new seq assigned for a no-op");
    }

    #[test]
    fn missing_attribution_is_rejected() {
        let (_tmp, conn, _project_id) = setup();
        let mut node = make_node("decision-1", "note", Utc::now());
        node.created_by = None;

        let err = apply_push(&conn, "proj", std::slice::from_ref(&node), &[]).unwrap_err();
        assert!(err.to_string().contains("created_by"));
    }

    #[test]
    fn tombstone_is_sticky_against_later_live_push() {
        let (_tmp, conn, project_id) = setup();
        let t0 = Utc::now() - chrono::Duration::seconds(10);
        let node = make_node("decision-1", "v1", t0);
        let edge = belongs_to(node.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&node),
            std::slice::from_ref(&edge),
        )
        .expect("initial push ok");

        let mut tombstone = node.clone();
        tombstone.deleted_at = Some(Utc::now());
        tombstone.updated_at = Utc::now();
        apply_push(&conn, "proj", std::slice::from_ref(&tombstone), &[])
            .expect("tombstone push ok");

        // A later push with an even newer `updated_at` but no deleted_at
        // must not revive the node.
        let mut revive_attempt = node.clone();
        revive_attempt.note = Some("please come back".to_string());
        revive_attempt.updated_at = Utc::now() + chrono::Duration::seconds(10);
        revive_attempt.deleted_at = None;
        let resp = apply_push(&conn, "proj", std::slice::from_ref(&revive_attempt), &[])
            .expect("revive push ok");

        assert_eq!(resp.accepted, 0);
        let stored = fetch_node(&conn, &node.id.to_string()).unwrap().unwrap();
        assert!(stored.deleted_at.is_some());
    }

    // --- T026: races called out in spec.md's Edge Cases / User Story 4 ---

    /// Simultaneous update-vs-update: the owner and a collaborator each edit
    /// their own local copy of the same node before either has synced with
    /// the other (spec.md: "both the owner and the collaborator modify the
    /// same record before either has synced the other's change"). Whichever
    /// edit carries the later `updated_at` wins deterministically once both
    /// have been pushed — regardless of push order — and the losing edit is
    /// retained under `data._sync_conflict` rather than discarded (FR-012).
    #[test]
    fn simultaneous_update_vs_update_retains_losing_edit_regardless_of_push_order() {
        let (_tmp, conn, project_id) = setup();
        let t_create = Utc::now() - chrono::Duration::seconds(30);
        let base = make_node("shared-decision", "original", t_create);
        let edge = belongs_to(base.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&base),
            std::slice::from_ref(&edge),
        )
        .expect("base push ok");

        // Owner and collaborator both branch off `base` while offline, and
        // owner happens to sync first — but the collaborator's edit is the
        // later one in wall-clock time.
        let t_owner = Utc::now() - chrono::Duration::seconds(10);
        let t_collab = Utc::now() - chrono::Duration::seconds(5);
        assert!(t_collab > t_owner);

        let mut owner_edit = base.clone();
        owner_edit.note = Some("owner's edit".to_string());
        owner_edit.data = json!({"from": "owner"});
        owner_edit.updated_by = Some("Owner <owner@example.com>".to_string());
        owner_edit.updated_at = t_owner;

        let mut collab_edit = base.clone();
        collab_edit.note = Some("collaborator's edit".to_string());
        collab_edit.data = json!({"from": "collaborator"});
        collab_edit.updated_by = Some("Collab <collab@example.com>".to_string());
        collab_edit.updated_at = t_collab;

        let owner_resp = apply_push(&conn, "proj", std::slice::from_ref(&owner_edit), &[])
            .expect("owner push ok");
        assert_eq!(owner_resp.accepted, 1, "owner syncs first, no conflict yet");
        assert_eq!(owner_resp.conflicts, 0);

        let collab_resp = apply_push(&conn, "proj", std::slice::from_ref(&collab_edit), &[])
            .expect("collaborator push ok");
        assert_eq!(
            collab_resp.accepted, 1,
            "collaborator's edit is chronologically newer, so it wins even though it pushed second"
        );
        assert_eq!(collab_resp.conflicts, 0);

        let stored = fetch_node(&conn, &base.id.to_string()).unwrap().unwrap();
        assert_eq!(
            stored.note.as_deref(),
            Some("collaborator's edit"),
            "the newer edit is current"
        );

        let conflict = stored
            .data
            .get("_sync_conflict")
            .expect("owner's overwritten edit must be retained, not discarded");
        assert_eq!(
            conflict.get("note").and_then(|v| v.as_str()),
            Some("owner's edit"),
            "the losing (owner's) edit is recoverable from the winning row"
        );
        assert_eq!(
            conflict.get("updated_by").and_then(|v| v.as_str()),
            Some("Owner <owner@example.com>")
        );
    }

    /// Update-vs-tombstone: the owner edits the node while, offline, a
    /// collaborator deletes their own (older) local copy of it. The owner's
    /// edit is chronologically newer, so it wins deterministically — the
    /// node stays live — and the collaborator's delete attempt is retained
    /// as a recoverable conflict rather than silently winning or vanishing.
    #[test]
    fn newer_update_beats_older_concurrent_delete() {
        let (_tmp, conn, project_id) = setup();
        let t_create = Utc::now() - chrono::Duration::seconds(30);
        let base = make_node("shared-decision", "original", t_create);
        let edge = belongs_to(base.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&base),
            std::slice::from_ref(&edge),
        )
        .expect("base push ok");

        let t_delete = Utc::now() - chrono::Duration::seconds(10);
        let t_update = Utc::now() - chrono::Duration::seconds(5);
        assert!(t_update > t_delete);

        let mut owner_update = base.clone();
        owner_update.note = Some("owner kept working on this".to_string());
        owner_update.updated_by = Some("Owner <owner@example.com>".to_string());
        owner_update.updated_at = t_update;
        let update_resp = apply_push(&conn, "proj", std::slice::from_ref(&owner_update), &[])
            .expect("owner update push ok");
        assert_eq!(update_resp.accepted, 1);

        let mut collab_delete = base.clone();
        collab_delete.deleted_at = Some(t_delete);
        collab_delete.updated_by = Some("Collab <collab@example.com>".to_string());
        collab_delete.updated_at = t_delete;
        let delete_resp = apply_push(&conn, "proj", std::slice::from_ref(&collab_delete), &[])
            .expect("collaborator delete push ok");
        assert_eq!(
            delete_resp.accepted, 0,
            "the delete is older than the update already on the server"
        );
        assert_eq!(delete_resp.conflicts, 1);

        let stored = fetch_node(&conn, &base.id.to_string()).unwrap().unwrap();
        assert!(
            stored.deleted_at.is_none(),
            "the newer update wins — the node must stay live"
        );
        assert_eq!(stored.note.as_deref(), Some("owner kept working on this"));
        assert!(
            stored.data.get("_sync_conflict").is_some(),
            "the losing delete attempt must be recoverable, not silently dropped"
        );
    }

    /// Update-vs-tombstone, reversed: the collaborator's delete is
    /// chronologically newer than the owner's concurrent update, so the
    /// delete wins deterministically and the node tombstones — with the
    /// owner's overwritten edit retained as a recoverable conflict.
    #[test]
    fn newer_concurrent_delete_beats_older_update_and_tombstones_it() {
        let (_tmp, conn, project_id) = setup();
        let t_create = Utc::now() - chrono::Duration::seconds(30);
        let base = make_node("shared-decision", "original", t_create);
        let edge = belongs_to(base.id, project_id);
        apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&base),
            std::slice::from_ref(&edge),
        )
        .expect("base push ok");

        let t_update = Utc::now() - chrono::Duration::seconds(10);
        let t_delete = Utc::now() - chrono::Duration::seconds(5);
        assert!(t_delete > t_update);

        let mut owner_update = base.clone();
        owner_update.note = Some("owner kept working on this".to_string());
        owner_update.updated_by = Some("Owner <owner@example.com>".to_string());
        owner_update.updated_at = t_update;
        let update_resp = apply_push(&conn, "proj", std::slice::from_ref(&owner_update), &[])
            .expect("owner update push ok");
        assert_eq!(update_resp.accepted, 1);

        let mut collab_delete = base.clone();
        collab_delete.deleted_at = Some(t_delete);
        collab_delete.updated_by = Some("Collab <collab@example.com>".to_string());
        collab_delete.updated_at = t_delete;
        let delete_resp = apply_push(&conn, "proj", std::slice::from_ref(&collab_delete), &[])
            .expect("collaborator delete push ok");
        assert_eq!(
            delete_resp.accepted, 1,
            "the delete is newer than the update already on the server"
        );
        assert_eq!(delete_resp.conflicts, 0);

        let stored = fetch_node(&conn, &base.id.to_string()).unwrap().unwrap();
        assert!(
            stored.deleted_at.is_some(),
            "the newer delete wins — the node must be tombstoned"
        );
        let conflict = stored
            .data
            .get("_sync_conflict")
            .expect("owner's overwritten update must be retained, not discarded");
        assert_eq!(
            conflict.get("note").and_then(|v| v.as_str()),
            Some("owner kept working on this")
        );
    }

    // --- project-scope enforcement: a token authenticated for one project
    // must never be able to write data into a different project. ---

    #[test]
    fn push_rejects_node_not_linked_to_authorized_project() {
        let (_tmp, conn, _project_id) = setup();
        // No belongs_to edge at all — this node claims no project membership.
        let orphan = make_node("orphan", "not linked to anything", Utc::now());

        let err = apply_push(&conn, "proj", std::slice::from_ref(&orphan), &[]).unwrap_err();
        assert!(
            err.to_string().contains("push rejected"),
            "expected a project-scope rejection, got: {err}"
        );
        assert!(
            fetch_node(&conn, &orphan.id.to_string()).unwrap().is_none(),
            "a rejected push must not partially write anything"
        );
    }

    #[test]
    fn push_rejects_node_belonging_to_a_different_project_on_the_same_server() {
        let (_tmp, conn, _proj_id) = setup();
        // A second, unrelated project hosted on the same server.
        let other_project = crate::graph::add_node(
            &conn,
            NodeType::Project,
            "other-proj",
            None,
            "test",
            json!({}),
        )
        .expect("seed other project");

        let leaked = make_node("belongs-elsewhere", "not yours", Utc::now());
        let edge = belongs_to(leaked.id, other_project.id);

        // Authenticated for "proj", but this node/edge only ever link to
        // "other-proj" — a collaborator token for one project must never be
        // able to write into a different project hosted on the same server.
        let err = apply_push(
            &conn,
            "proj",
            std::slice::from_ref(&leaked),
            std::slice::from_ref(&edge),
        )
        .unwrap_err();
        assert!(err.to_string().contains("push rejected"), "got: {err}");
        assert!(
            fetch_node(&conn, &leaked.id.to_string()).unwrap().is_none(),
            "a rejected push must not partially write anything"
        );
    }

    #[test]
    fn push_bootstraps_a_brand_new_project_via_its_own_first_push() {
        let tmp = TmpDb::new("bootstrap");
        let conn = db::open(&tmp.0).expect("open temp db");
        // Deliberately no pre-seeded project — the very first push for a
        // brand-new project includes the project node itself.
        let project_node = Node {
            id: Uuid::new_v4(),
            node_type: NodeType::Project,
            label: "brand-new".to_string(),
            note: None,
            source: "test".to_string(),
            data: json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            memory_kind: MemoryKind::Semantic,
            last_accessed_at: Utc::now(),
            access_count: 0,
            content_hash: None,
            created_by: Some("Alice <alice@example.com>".to_string()),
            updated_by: Some("Alice <alice@example.com>".to_string()),
            deleted_at: None,
            sync_seq: None,
        };
        let decision = make_node("first-decision", "kickoff", Utc::now());
        let edge = belongs_to(decision.id, project_node.id);

        let resp = apply_push(
            &conn,
            "brand-new",
            &[project_node.clone(), decision.clone()],
            std::slice::from_ref(&edge),
        )
        .expect("bootstrap push ok");

        assert_eq!(resp.accepted, 3);
        assert!(fetch_node(&conn, &project_node.id.to_string())
            .unwrap()
            .is_some());
        assert!(fetch_node(&conn, &decision.id.to_string())
            .unwrap()
            .is_some());
    }
}
