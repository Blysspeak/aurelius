//! Integration test for T017: starts the real `aurelius-sync-server` router
//! against a temp SQLite file, bound to an OS-assigned ephemeral port, and
//! drives it over real HTTP with `reqwest`, per
//! `specs/002-project-sync/contracts/sync-api.md`.

// Integration test — the whole file is test code, not a runtime path;
// unwrap/expect here are the assertion mechanism itself.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use aurelius_core::models::{Edge, MemoryKind, Node, NodeType, Relation};
use aurelius_core::sync::{SyncPullResponse, SyncPushRequest, SyncPushResponse};
use aurelius_sync_server::{build_router, AppState};
use chrono::Utc;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "test-admin-token";

/// Starts the server against a fresh temp SQLite file on `127.0.0.1:0`,
/// returning the base `http://…/sync` URL the test can hit.
async fn spawn_server() -> String {
    let db_path: PathBuf =
        std::env::temp_dir().join(format!("aurelius-sync-test-{}.db", Uuid::new_v4()));

    let state = AppState {
        db_path,
        admin_token: ADMIN_TOKEN.to_string(),
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("read back bound addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server crashed");
    });

    format!("http://{addr}/sync")
}

fn make_project_node(label: &str, author: &str) -> Node {
    let now = Utc::now();
    Node {
        id: Uuid::new_v4(),
        node_type: NodeType::Project,
        label: label.to_string(),
        note: None,
        source: "test".to_string(),
        data: json!({}),
        created_at: now,
        updated_at: now,
        memory_kind: MemoryKind::Semantic,
        last_accessed_at: now,
        access_count: 0,
        content_hash: None,
        created_by: Some(author.to_string()),
        updated_by: Some(author.to_string()),
        deleted_at: None,
        sync_seq: None,
    }
}

fn make_decision_node(label: &str, note: &str, author: &str) -> Node {
    let now = Utc::now();
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
        created_by: Some(author.to_string()),
        updated_by: Some(author.to_string()),
        deleted_at: None,
        sync_seq: None,
    }
}

fn make_belongs_to_edge(from_id: Uuid, to_id: Uuid, author: &str) -> Edge {
    Edge {
        id: Uuid::new_v4(),
        from_id,
        to_id,
        relation: Relation::BelongsTo,
        weight: 1.0,
        created_at: Utc::now(),
        created_by: Some(author.to_string()),
        deleted_at: None,
        sync_seq: None,
    }
}

/// Issues a collaborator token for `project` via `POST /sync/grants`, using
/// the admin credential — mirrors what `au share issue` does.
async fn issue_grant(client: &reqwest::Client, base_url: &str, project: &str) -> String {
    let resp = client
        .post(format!("{base_url}/grants"))
        .bearer_auth(ADMIN_TOKEN)
        .json(&json!({
            "project": project,
            "person_name": "Tester",
            "person_email": "tester@example.com",
        }))
        .send()
        .await
        .expect("grants request");
    assert_eq!(resp.status(), 200, "grants issuance should succeed");
    let body: serde_json::Value = resp.json().await.expect("grants response json");
    body.get("token")
        .and_then(|t| t.as_str())
        .expect("grants response has a token field")
        .to_string()
}

#[tokio::test]
async fn push_then_pull_round_trips_full_history() {
    let base_url = spawn_server().await;
    let client = reqwest::Client::new();

    let project_label = "demo";
    let token = issue_grant(&client, &base_url, project_label).await;

    let author = "Owner <owner@example.com>";
    let project_node = make_project_node(project_label, author);
    let decision_node = make_decision_node(
        "chose axum for sync",
        "because it's already a workspace dep",
        author,
    );
    let edge = make_belongs_to_edge(decision_node.id, project_node.id, author);

    let push_body = SyncPushRequest {
        nodes: vec![project_node.clone(), decision_node.clone()],
        edges: vec![edge.clone()],
    };

    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&token)
        .json(&push_body)
        .send()
        .await
        .expect("push request");
    assert_eq!(resp.status(), 200, "push should succeed");
    let push_resp: SyncPushResponse = resp.json().await.expect("push response json");
    assert_eq!(push_resp.accepted, 3, "2 nodes + 1 edge should be accepted");
    assert_eq!(push_resp.conflicts, 0);
    assert!(push_resp.server_seq >= 3);

    let resp = client
        .get(format!("{base_url}/pull?since=0"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("pull request");
    assert_eq!(resp.status(), 200, "pull should succeed");
    let pull_resp: SyncPullResponse = resp.json().await.expect("pull response json");

    assert_eq!(pull_resp.project, project_label);
    assert_eq!(pull_resp.server_seq, push_resp.server_seq);
    assert_eq!(pull_resp.nodes.len(), 2, "both nodes should round-trip");
    assert_eq!(
        pull_resp.edges.len(),
        1,
        "the belongs_to edge should round-trip"
    );

    let pulled_decision = pull_resp
        .nodes
        .iter()
        .find(|n| n.id == decision_node.id)
        .expect("decision node present in pull response");
    assert_eq!(pulled_decision.created_by.as_deref(), Some(author));
    assert_eq!(pulled_decision.updated_by.as_deref(), Some(author));
    assert_eq!(
        pulled_decision.note.as_deref(),
        Some("because it's already a workspace dep")
    );

    let pulled_edge = pull_resp
        .edges
        .iter()
        .find(|e| e.id == edge.id)
        .expect("belongs_to edge present in pull response");
    assert_eq!(pulled_edge.created_by.as_deref(), Some(author));
    assert_eq!(pulled_edge.from_id, decision_node.id);
    assert_eq!(pulled_edge.to_id, project_node.id);
}

/// T025: a deletion propagated through the real HTTP `push`/`pull` routes
/// (not just `sync::merge::apply_push` directly) — the tombstone mirrors
/// what `aurelius_core::graph::delete_node` now produces (`deleted_at` and
/// `updated_at` bumped to the same instant, see its doc comment) — must
/// stick on the peer and must not be resurrected by a later stale push of
/// the pre-delete "live" version (FR-010, spec.md User Story 4).
#[tokio::test]
async fn delete_propagates_as_tombstone_and_does_not_resurrect() {
    let base_url = spawn_server().await;
    let client = reqwest::Client::new();

    let project_label = "tombstone-demo";
    let token = issue_grant(&client, &base_url, project_label).await;

    let author = "Owner <owner@example.com>";
    let project_node = make_project_node(project_label, author);
    let decision_node = make_decision_node("obsolete decision", "no longer relevant", author);
    let edge = make_belongs_to_edge(decision_node.id, project_node.id, author);

    let push1 = SyncPushRequest {
        nodes: vec![project_node.clone(), decision_node.clone()],
        edges: vec![edge.clone()],
    };
    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&token)
        .json(&push1)
        .send()
        .await
        .expect("initial push request");
    assert_eq!(resp.status(), 200);
    let push1_resp: SyncPushResponse = resp.json().await.expect("push1 response json");
    assert_eq!(push1_resp.accepted, 3);

    // Locally soft-delete the decision: `deleted_at` and a freshly bumped
    // `updated_at` set to the same instant.
    let delete_time = Utc::now();
    let mut tombstoned = decision_node.clone();
    tombstoned.deleted_at = Some(delete_time);
    tombstoned.updated_at = delete_time;
    let mut tombstoned_edge = edge.clone();
    tombstoned_edge.deleted_at = Some(delete_time);

    let push2 = SyncPushRequest {
        nodes: vec![tombstoned.clone()],
        edges: vec![tombstoned_edge.clone()],
    };
    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&token)
        .json(&push2)
        .send()
        .await
        .expect("tombstone push request");
    assert_eq!(resp.status(), 200);
    let push2_resp: SyncPushResponse = resp.json().await.expect("push2 response json");
    assert_eq!(
        push2_resp.accepted, 2,
        "the tombstoned node and edge must be accepted, not lost to a conflict"
    );
    assert_eq!(push2_resp.conflicts, 0);

    // A fresh collaborator bootstrapping now must see the deletion, not a
    // live copy.
    let resp = client
        .get(format!("{base_url}/pull?since=0"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("bootstrap pull request");
    let pull_resp: SyncPullResponse = resp.json().await.expect("pull response json");
    let pulled_node = pull_resp
        .nodes
        .iter()
        .find(|n| n.id == decision_node.id)
        .expect("tombstoned node still present (as a tombstone) in the bootstrap pull");
    assert!(
        pulled_node.deleted_at.is_some(),
        "the deletion must propagate to the peer"
    );
    let pulled_edge = pull_resp
        .edges
        .iter()
        .find(|e| e.id == edge.id)
        .expect("tombstoned edge present in the bootstrap pull");
    assert!(pulled_edge.deleted_at.is_some());

    // A later push of the stale pre-delete "live" version must not
    // resurrect it.
    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&token)
        .json(&SyncPushRequest {
            nodes: vec![decision_node.clone()],
            edges: vec![],
        })
        .send()
        .await
        .expect("stale live push request");
    let resurrect_resp: SyncPushResponse = resp.json().await.expect("resurrect response json");
    assert_eq!(
        resurrect_resp.accepted, 0,
        "a stale live push must not resurrect a tombstoned node"
    );

    let resp = client
        .get(format!("{base_url}/pull?since=0"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("second pull request");
    let pull_resp2: SyncPullResponse = resp.json().await.expect("pull2 response json");
    let still_gone = pull_resp2
        .nodes
        .iter()
        .find(|n| n.id == decision_node.id)
        .expect("node still present as a tombstone");
    assert!(
        still_gone.deleted_at.is_some(),
        "the deletion must never reappear from a later sync"
    );
}

#[tokio::test]
async fn revoked_token_is_rejected_on_subsequent_push_and_pull() {
    let base_url = spawn_server().await;
    let client = reqwest::Client::new();

    let project_label = "revoke-demo";
    let token = issue_grant(&client, &base_url, project_label).await;

    let author = "Owner <owner@example.com>";
    let project_node = make_project_node(project_label, author);
    let push_body = SyncPushRequest {
        nodes: vec![project_node],
        edges: vec![],
    };

    // Token is active — push should succeed before revocation.
    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&token)
        .json(&push_body)
        .send()
        .await
        .expect("push request");
    assert_eq!(resp.status(), 200);

    // Revoke the grant, as `au share revoke` would.
    let resp = client
        .post(format!("{base_url}/grants/revoke"))
        .bearer_auth(ADMIN_TOKEN)
        .json(&json!({ "project": project_label, "person_email": "tester@example.com" }))
        .send()
        .await
        .expect("revoke request");
    assert_eq!(resp.status(), 200);
    let revoke_resp: serde_json::Value = resp.json().await.expect("revoke response json");
    assert_eq!(revoke_resp.get("revoked").and_then(|v| v.as_i64()), Some(1));

    // Revoking an already-revoked / non-matching grant is not an error.
    let resp = client
        .post(format!("{base_url}/grants/revoke"))
        .bearer_auth(ADMIN_TOKEN)
        .json(&json!({ "project": project_label, "person_email": "tester@example.com" }))
        .send()
        .await
        .expect("second revoke request");
    assert_eq!(resp.status(), 200);
    let revoke_resp: serde_json::Value = resp.json().await.expect("second revoke response json");
    assert_eq!(revoke_resp.get("revoked").and_then(|v| v.as_i64()), Some(0));

    // Now the same token is rejected on both push and pull.
    let resp = client
        .get(format!("{base_url}/pull?since=0"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("pull request after revoke");
    assert_eq!(
        resp.status(),
        401,
        "revoked token should be rejected on pull"
    );

    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&token)
        .json(&SyncPushRequest {
            nodes: vec![],
            edges: vec![],
        })
        .send()
        .await
        .expect("push request after revoke");
    assert_eq!(
        resp.status(),
        401,
        "revoked token should be rejected on push"
    );
}

#[tokio::test]
async fn bad_admin_token_is_rejected() {
    let base_url = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base_url}/grants"))
        .bearer_auth("not-the-admin-token")
        .json(&json!({
            "project": "demo",
            "person_name": "Tester",
            "person_email": "tester@example.com",
        }))
        .send()
        .await
        .expect("grants request");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn unknown_collaborator_token_is_rejected() {
    let base_url = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base_url}/pull?since=0"))
        .bearer_auth("not-a-real-token")
        .send()
        .await
        .expect("pull request");
    assert_eq!(resp.status(), 401);
}
