//! Integration test for T024: a bug-report task filed by a "tester" client,
//! picked up and completed by an "owner" client, and pulled back by the
//! tester — asserting status and attribution at each hop, per
//! `specs/002-project-sync/spec.md` User Story 3 and
//! `specs/002-project-sync/contracts/sync-api.md`.
//!
//! Mirrors `tests/push_pull.rs`'s pattern: drives the real `axum::Router`
//! over real HTTP on an OS-assigned ephemeral port, against a temp SQLite
//! file — no `au`/`aurelius` CLI invocation and no `identity::current()` (so
//! no real `~/.config/aurelius` file is ever touched).

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
        std::env::temp_dir().join(format!("aurelius-sync-task-test-{}.db", Uuid::new_v4()));

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

/// Issues a collaborator token for `(project, person)` via `POST
/// /sync/grants`, using the admin credential — mirrors what `au share issue`
/// does. Both the tester and the owner hold their own token to the same
/// project.
async fn issue_grant(
    client: &reqwest::Client,
    base_url: &str,
    project: &str,
    person_name: &str,
    person_email: &str,
) -> String {
    let resp = client
        .post(format!("{base_url}/grants"))
        .bearer_auth(ADMIN_TOKEN)
        .json(&json!({
            "project": project,
            "person_name": person_name,
            "person_email": person_email,
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

/// Mirrors the shape `task_create` (MCP) / `TaskAction::New` (CLI) build,
/// per `crates/aurelius/src/mcp/handlers/task.rs`.
fn make_task_node(label: &str, note: &str, project: &str, author: &str) -> Node {
    let now = Utc::now();
    Node {
        id: Uuid::new_v4(),
        node_type: NodeType::Task,
        label: label.to_string(),
        note: Some(note.to_string()),
        source: "test".to_string(),
        data: json!({
            "status": "backlog",
            "priority": "high",
            "acceptance_criteria": [],
            "project": project,
            "started_at": null,
            "completed_at": null,
        }),
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

fn make_worklog_node(label: &str, note: &str, task_id: Uuid, author: &str) -> Node {
    let now = Utc::now();
    Node {
        id: Uuid::new_v4(),
        node_type: NodeType::WorkLog,
        label: label.to_string(),
        note: Some(note.to_string()),
        source: "test".to_string(),
        data: json!({"task_id": task_id.to_string()}),
        created_at: now,
        updated_at: now,
        memory_kind: MemoryKind::Episodic,
        last_accessed_at: now,
        access_count: 0,
        content_hash: None,
        created_by: Some(author.to_string()),
        updated_by: Some(author.to_string()),
        deleted_at: None,
        sync_seq: None,
    }
}

fn make_edge(from_id: Uuid, to_id: Uuid, relation: Relation, author: &str) -> Edge {
    Edge {
        id: Uuid::new_v4(),
        from_id,
        to_id,
        relation,
        weight: 1.0,
        created_at: Utc::now(),
        created_by: Some(author.to_string()),
        deleted_at: None,
        sync_seq: None,
    }
}

#[tokio::test]
async fn bug_report_task_roundtrips_with_attribution_at_each_hop() {
    let base_url = spawn_server().await;
    let client = reqwest::Client::new();

    let project_label = "widget-app";
    let tester_author = "Tester <tester@example.com>";
    let owner_author = "Owner <owner@example.com>";

    let tester_token = issue_grant(
        &client,
        &base_url,
        project_label,
        "Tester",
        "tester@example.com",
    )
    .await;
    let owner_token = issue_grant(
        &client,
        &base_url,
        project_label,
        "Owner",
        "owner@example.com",
    )
    .await;

    // --- Hop 1: tester files a bug report task and pushes it. ---
    let project_node = make_project_node(project_label, tester_author);
    let task_node = make_task_node(
        &format!("[{project_label}] login button does nothing"),
        "clicking login on the landing page does nothing in Safari",
        project_label,
        tester_author,
    );
    let task_belongs_to_project = make_edge(
        task_node.id,
        project_node.id,
        Relation::BelongsTo,
        tester_author,
    );

    let push1 = SyncPushRequest {
        nodes: vec![project_node.clone(), task_node.clone()],
        edges: vec![task_belongs_to_project.clone()],
    };
    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&tester_token)
        .json(&push1)
        .send()
        .await
        .expect("tester push request");
    assert_eq!(resp.status(), 200, "tester's push should succeed");
    let push1_resp: SyncPushResponse = resp.json().await.expect("push1 response json");
    assert_eq!(
        push1_resp.accepted, 3,
        "2 nodes + 1 edge should be accepted"
    );
    assert_eq!(push1_resp.conflicts, 0);

    // --- Hop 2: owner pulls and sees the new task, attributed to the tester. ---
    let resp = client
        .get(format!("{base_url}/pull?since=0"))
        .bearer_auth(&owner_token)
        .send()
        .await
        .expect("owner pull request");
    assert_eq!(resp.status(), 200, "owner's pull should succeed");
    let owner_pull1: SyncPullResponse = resp.json().await.expect("owner pull1 response json");

    assert_eq!(owner_pull1.project, project_label);
    let pulled_task = owner_pull1
        .nodes
        .iter()
        .find(|n| n.id == task_node.id)
        .expect("owner sees the tester's task")
        .clone();
    assert_eq!(
        pulled_task.data.get("status").and_then(|s| s.as_str()),
        Some("backlog"),
        "task should still be in backlog when the owner first sees it"
    );
    assert_eq!(pulled_task.created_by.as_deref(), Some(tester_author));
    assert_eq!(pulled_task.updated_by.as_deref(), Some(tester_author));

    // --- Hop 3: owner logs work and completes the task, then pushes back. ---
    let mut completed_task = pulled_task.clone();
    completed_task.updated_at = pulled_task.updated_at + chrono::Duration::seconds(60);
    completed_task.updated_by = Some(owner_author.to_string());
    completed_task.data["status"] = json!("done");
    completed_task.data["completed_at"] = json!(completed_task.updated_at.to_rfc3339());

    let worklog_node = make_worklog_node(
        &format!("[{project_label}] fixed missing onClick handler"),
        "root cause: the login button's onClick handler was never wired up; fixed in the Safari-only render path",
        task_node.id,
        owner_author,
    );
    let worklog_contains = make_edge(
        task_node.id,
        worklog_node.id,
        Relation::Contains,
        owner_author,
    );
    let worklog_belongs_to_project = make_edge(
        worklog_node.id,
        project_node.id,
        Relation::BelongsTo,
        owner_author,
    );

    let push2 = SyncPushRequest {
        nodes: vec![completed_task.clone(), worklog_node.clone()],
        edges: vec![worklog_contains.clone(), worklog_belongs_to_project.clone()],
    };
    let resp = client
        .post(format!("{base_url}/push"))
        .bearer_auth(&owner_token)
        .json(&push2)
        .send()
        .await
        .expect("owner push request");
    assert_eq!(resp.status(), 200, "owner's completion push should succeed");
    let push2_resp: SyncPushResponse = resp.json().await.expect("push2 response json");
    assert_eq!(
        push2_resp.accepted, 4,
        "task overwrite + worklog insert + 2 edges should be accepted"
    );
    assert_eq!(
        push2_resp.conflicts, 0,
        "owner's update is strictly newer, so it should not conflict"
    );

    // --- Hop 4: tester pulls back (incrementally, since their own push) and
    // sees the completed status, who completed it, and what was logged. ---
    let resp = client
        .get(format!("{base_url}/pull?since={}", push1_resp.server_seq))
        .bearer_auth(&tester_token)
        .send()
        .await
        .expect("tester pull-back request");
    assert_eq!(resp.status(), 200, "tester's pull-back should succeed");
    let tester_pull2: SyncPullResponse = resp.json().await.expect("tester pull2 response json");

    let pulled_completed_task = tester_pull2
        .nodes
        .iter()
        .find(|n| n.id == task_node.id)
        .expect("tester sees the completed task");
    assert_eq!(
        pulled_completed_task
            .data
            .get("status")
            .and_then(|s| s.as_str()),
        Some("done"),
        "tester should see the task marked done"
    );
    assert_eq!(
        pulled_completed_task.created_by.as_deref(),
        Some(tester_author),
        "creator attribution must be preserved across the owner's update"
    );
    assert_eq!(
        pulled_completed_task.updated_by.as_deref(),
        Some(owner_author),
        "tester should see who completed it"
    );

    let pulled_worklog = tester_pull2
        .nodes
        .iter()
        .find(|n| n.id == worklog_node.id)
        .expect("tester sees the owner's logged work");
    assert_eq!(
        pulled_worklog.note.as_deref(),
        Some(
            "root cause: the login button's onClick handler was never wired up; fixed in the Safari-only render path"
        )
    );
    assert_eq!(pulled_worklog.created_by.as_deref(), Some(owner_author));
}
