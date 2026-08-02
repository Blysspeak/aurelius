//! HTTP handlers for `/sync/push`, `/sync/pull`, `/sync/grants`, and
//! `/sync/grants/revoke`, per `specs/002-project-sync/contracts/sync-api.md`.
//! Push/pull delegate the actual merge logic to `aurelius_core::sync::merge`;
//! grants issuance/revocation talk to the `collaborator_grants` table
//! directly, since that table is server-only (see data-model.md).

use crate::auth::{hash_token, AdminAuth, ApiError, CollaboratorAuth};
use crate::AppState;
use aurelius_core::db;
use aurelius_core::models::{Edge, Node};
use aurelius_core::sync::{merge, SyncPullResponse, SyncPushRequest, SyncPushResponse};
use axum::{
    extract::{FromRequest, Query, Request, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/push", post(push))
        .route("/pull", get(pull))
        .route("/grants", post(grants))
        .route("/grants/revoke", post(grants_revoke))
}

/// A `Json<T>` extractor whose rejection maps to `422`, matching
/// `contracts/sync-api.md`'s "malformed payload" error rather than axum's
/// default `400`.
struct AppJson<T>(T);

impl<S, T> FromRequest<S> for AppJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|err| ApiError::Unprocessable(err.to_string()))?;
        Ok(AppJson(value))
    }
}

/// Rejects any node/edge missing `created_by`, per contracts/sync-api.md.
fn validate_attribution(nodes: &[Node], edges: &[Edge]) -> Result<(), ApiError> {
    for node in nodes {
        if node.created_by.is_none() {
            return Err(ApiError::Unprocessable(format!(
                "node {} missing required created_by attribution",
                node.id
            )));
        }
    }
    for edge in edges {
        if edge.created_by.is_none() {
            return Err(ApiError::Unprocessable(format!(
                "edge {} missing required created_by attribution",
                edge.id
            )));
        }
    }
    Ok(())
}

async fn push(
    State(state): State<AppState>,
    auth: CollaboratorAuth,
    AppJson(body): AppJson<SyncPushRequest>,
) -> Result<Json<SyncPushResponse>, ApiError> {
    validate_attribution(&body.nodes, &body.edges)?;

    let project = auth.project_label;
    let db_path = state.db_path.clone();
    let response = tokio::task::spawn_blocking(move || -> anyhow::Result<SyncPushResponse> {
        let conn = db::open(&db_path)?;
        merge::apply_push(&conn, &project, &body.nodes, &body.edges)
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?
    // `apply_push` bails with a "push rejected:"-prefixed message when a
    // node/edge falls outside the authenticated token's project — a client
    // mistake (or a probing attempt), not a server fault, so it maps to 422
    // rather than the generic 500 every other error here gets.
    .map_err(|e| {
        let msg = e.to_string();
        if msg.starts_with("push rejected:") {
            ApiError::Unprocessable(msg)
        } else {
            ApiError::Internal(e)
        }
    })?;

    Ok(Json(response))
}

#[derive(Deserialize)]
struct PullQuery {
    since: Option<i64>,
}

async fn pull(
    State(state): State<AppState>,
    auth: CollaboratorAuth,
    Query(query): Query<PullQuery>,
) -> Result<Json<SyncPullResponse>, ApiError> {
    let since_seq = query.since.unwrap_or(0);
    let project = auth.project_label;
    let db_path = state.db_path.clone();

    let response = tokio::task::spawn_blocking(move || -> anyhow::Result<SyncPullResponse> {
        let conn = db::open(&db_path)?;
        merge::pull_since(&conn, &project, since_seq)
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))??;

    Ok(Json(response))
}

#[derive(Deserialize)]
struct GrantsRequest {
    project: String,
    person_name: String,
    person_email: String,
}

#[derive(Serialize)]
struct GrantsResponse {
    token: String,
}

/// Mints a new collaborator token for `project`: stores only its sha256
/// hash, returns the plaintext exactly once.
async fn grants(
    State(state): State<AppState>,
    _admin: AdminAuth,
    AppJson(body): AppJson<GrantsRequest>,
) -> Result<Json<GrantsResponse>, ApiError> {
    if body.project.trim().is_empty()
        || body.person_name.trim().is_empty()
        || body.person_email.trim().is_empty()
    {
        return Err(ApiError::Unprocessable(
            "project, person_name, and person_email are required".to_string(),
        ));
    }

    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let token_hash = hash_token(&token);
    let db_path = state.db_path.clone();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = db::open(&db_path)?;
        conn.execute(
            "INSERT INTO collaborator_grants
                (token_hash, person_name, person_email, project_label, granted_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![
                token_hash,
                body.person_name,
                body.person_email,
                body.project,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))??;

    Ok(Json(GrantsResponse { token }))
}

#[derive(Deserialize)]
struct RevokeRequest {
    project: String,
    person_email: String,
}

#[derive(Serialize)]
struct RevokeResponse {
    revoked: usize,
}

/// Sets `revoked_at` on every matching, not-yet-revoked grant for
/// `(project, person_email)`. Zero matches is not an error.
async fn grants_revoke(
    State(state): State<AppState>,
    _admin: AdminAuth,
    AppJson(body): AppJson<RevokeRequest>,
) -> Result<Json<RevokeResponse>, ApiError> {
    if body.project.trim().is_empty() || body.person_email.trim().is_empty() {
        return Err(ApiError::Unprocessable(
            "project and person_email are required".to_string(),
        ));
    }

    let db_path = state.db_path.clone();
    let revoked = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
        let conn = db::open(&db_path)?;
        let count = conn.execute(
            "UPDATE collaborator_grants SET revoked_at = ?1
             WHERE project_label = ?2 AND person_email = ?3 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), body.project, body.person_email],
        )?;
        Ok(count)
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))??;

    Ok(Json(RevokeResponse { revoked }))
}
