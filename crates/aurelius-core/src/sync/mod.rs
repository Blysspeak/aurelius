//! Shared request/response types for the sync server HTTP API, per
//! `specs/002-project-sync/contracts/sync-api.md`. Used by both
//! `aurelius-sync-server` (server side) and `au`/`aurelius` (client side).

pub mod merge;

use crate::models::{Edge, Node};
use serde::{Deserialize, Serialize};

/// `POST /sync/push` request body. No `project` field — per
/// `contracts/sync-api.md`, the project is always resolved server-side from
/// the authenticated token, never supplied by the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushRequest {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// `POST /sync/push` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushResponse {
    /// Items newly inserted or that overwrote an older server copy.
    pub accepted: usize,
    /// Items where the server's existing copy won over what the client
    /// pushed — informational, not an error.
    pub conflicts: usize,
    pub server_seq: i64,
}

/// `GET /sync/pull` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullResponse {
    /// The token-resolved project's label, so a first-time client can
    /// create/attach its local project under the same name without already
    /// knowing it (see `contracts/sync-api.md`).
    pub project: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub server_seq: i64,
}
