//! Integration test for T017: starts the real `aurelius-sync-server` router
//! against a temp SQLite file, bound to an OS-assigned ephemeral port, and
//! drives it over real HTTP with `reqwest`, per
//! `specs/002-project-sync/contracts/sync-api.md`.

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
