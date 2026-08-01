//! Bearer-token authentication, per `specs/002-project-sync/contracts/sync-api.md`.
//!
//! Two schemes, both `Authorization: Bearer {token}`:
//! - [`CollaboratorAuth`]: push/pull. The token is hashed and matched against
//!   `collaborator_grants.token_hash`; the grant (if active) resolves the
//!   project — the client never supplies a project separately.
//! - [`AdminAuth`]: grants issuance/revocation, checked against the
//!   `AURELIUS_SYNC_ADMIN_TOKEN` the server was started with.

use crate::AppState;
use aurelius_core::db;
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

/// Errors surfaced to sync-server clients, mapped to the status codes in
/// `contracts/sync-api.md`.
pub enum ApiError {
    Unauthorized,
    Unprocessable(String),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
            ApiError::Unprocessable(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg).into_response(),
            ApiError::Internal(err) => {
                tracing::error!(error = %err, "sync-server internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Internal(err)
    }
}

pub(crate) fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn bearer_token(parts: &Parts) -> Result<String, ApiError> {
    parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
        .ok_or(ApiError::Unauthorized)
}

/// Resolves the authenticated collaborator's project from the bearer token.
/// Push/pull handlers extract this instead of taking a project parameter.
pub struct CollaboratorAuth {
    pub project_label: String,
}

impl FromRequestParts<AppState> for CollaboratorAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)?;
        let token_hash = hash_token(&token);
        let db_path = state.db_path.clone();

        let project_label =
            tokio::task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
                let conn = db::open(&db_path)?;
                let project_label = conn
                    .query_row(
                        "SELECT project_label FROM collaborator_grants
                         WHERE token_hash = ?1 AND revoked_at IS NULL",
                        rusqlite::params![token_hash],
                        |row| row.get::<_, String>(0),
                    )
                    .ok();
                Ok(project_label)
            })
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))??;

        project_label
            .map(|project_label| CollaboratorAuth { project_label })
            .ok_or(ApiError::Unauthorized)
    }
}

/// Verifies the bearer token against `AppState::admin_token`, for the
/// grants/grants-revoke endpoints.
pub struct AdminAuth;

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)?;
        if token == state.admin_token {
            Ok(AdminAuth)
        } else {
            Err(ApiError::Unauthorized)
        }
    }
}
