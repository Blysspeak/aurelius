//! Client-side sync operations shared between `au` (CLI, `au share
//! push/pull`) and `aurelius` (MCP server, automatic sync at session
//! boundaries per US2): `sync_config` table access plus the push/pull HTTP
//! round trip against `aurelius-sync-server`, per `contracts/sync-api.md`.
//!
//! Callers at automatic sync points (MCP `memory_status`/`memory_session`,
//! the Stop hook) MUST treat `Err` from `push_project`/`pull_project` as
//! non-blocking per FR-006/FR-011 — log a warning and continue, never fail
//! the surrounding operation.

use super::{SyncPullResponse, SyncPushRequest, SyncPushResponse};
use crate::graph::{row_to_edge, row_to_node};
use crate::models::{Edge, Node};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};

/// One project's sync connection state — mirrors the `sync_config` table
/// (data-model.md `SyncConfig`).
#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub project_label: String,
    pub server_url: String,
    pub token: String,
    pub enabled: bool,
    pub last_seq: i64,
    pub updated_at: String,
}

fn map_row(row: &rusqlite::Row) -> rusqlite::Result<SyncConfig> {
    Ok(SyncConfig {
        project_label: row.get(0)?,
        server_url: row.get(1)?,
        token: row.get(2)?,
        enabled: row.get(3)?,
        last_seq: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

const SYNC_CONFIG_SELECT: &str =
    "SELECT project_label, server_url, token, enabled, last_seq, updated_at FROM sync_config";

pub fn get_sync_config(conn: &Connection, project: &str) -> Result<Option<SyncConfig>> {
    let mut stmt = conn.prepare(&format!("{SYNC_CONFIG_SELECT} WHERE project_label = ?1"))?;
    let mut rows = stmt.query_map(params![project], map_row)?;
    Ok(rows.next().transpose()?)
}

pub fn list_sync_configs(conn: &Connection) -> Result<Vec<SyncConfig>> {
    let mut stmt = conn.prepare(&format!("{SYNC_CONFIG_SELECT} ORDER BY project_label"))?;
    let rows = stmt
        .query_map([], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn upsert_sync_config(
    conn: &Connection,
    project: &str,
    server_url: &str,
    token: &str,
    enabled: bool,
    last_seq: i64,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sync_config (project_label, server_url, token, enabled, last_seq, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(project_label) DO UPDATE SET
             server_url = excluded.server_url,
             token = excluded.token,
             enabled = excluded.enabled,
             last_seq = excluded.last_seq,
             updated_at = excluded.updated_at",
        params![project, server_url, token, enabled, last_seq, now],
    )?;
    Ok(())
}

pub fn set_sync_last_seq(conn: &Connection, project: &str, last_seq: i64) -> Result<()> {
    conn.execute(
        "UPDATE sync_config SET last_seq = ?1, updated_at = ?2 WHERE project_label = ?3",
        params![last_seq, chrono::Utc::now().to_rfc3339(), project],
    )?;
    Ok(())
}

pub fn set_sync_enabled(conn: &Connection, project: &str, enabled: bool) -> Result<bool> {
    let affected = conn.execute(
        "UPDATE sync_config SET enabled = ?1, updated_at = ?2 WHERE project_label = ?3",
        params![enabled, chrono::Utc::now().to_rfc3339(), project],
    )?;
    Ok(affected > 0)
}

/// Looks up a previously stored admin token for `server_url` (from
/// `au share admin-set`) — lets `au share issue`/`revoke` skip
/// AURELIUS_SYNC_ADMIN_TOKEN entirely after the first setup.
pub fn get_admin_token(conn: &Connection, server_url: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT token FROM admin_tokens WHERE server_url = ?1")?;
    let mut rows = stmt.query_map(params![server_url], |r| r.get::<_, String>(0))?;
    Ok(rows.next().transpose()?)
}

pub fn upsert_admin_token(conn: &Connection, server_url: &str, token: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO admin_tokens (server_url, token, saved_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(server_url) DO UPDATE SET token = excluded.token, saved_at = excluded.saved_at",
        params![server_url, token, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Resolves which sync configs a push/pull should target: a specific
/// project (must be connected and enabled) or every enabled project.
pub fn resolve_sync_targets(conn: &Connection, project: Option<&str>) -> Result<Vec<SyncConfig>> {
    match project {
        Some(p) => {
            let cfg = get_sync_config(conn, p)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "project '{p}' is not connected to sync — run `au share <server> <token>` first"
                )
            })?;
            if !cfg.enabled {
                anyhow::bail!("project '{p}' has sync disabled — see `au share list`");
            }
            Ok(vec![cfg])
        }
        None => Ok(list_sync_configs(conn)?
            .into_iter()
            .filter(|c| c.enabled)
            .collect()),
    }
}

// --- project-scoped node/edge selection for push (mirrors sync::merge's
// membership rule: a project's own node plus everything `belongs_to` it) ---

const SYNC_NODE_SELECT: &str =
    "SELECT id, node_type, label, note, source, data, created_at, updated_at,
        memory_kind, last_accessed_at, access_count, content_hash,
        created_by, updated_by, deleted_at, sync_seq FROM nodes";
const SYNC_EDGE_SELECT: &str =
    "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq FROM edges";

fn project_member_ids(conn: &Connection, project_id: &str) -> Result<Vec<String>> {
    let mut ids = vec![project_id.to_string()];
    let mut stmt = conn.prepare(
        "SELECT DISTINCT from_id FROM edges WHERE to_id = ?1 AND relation = 'belongs_to'",
    )?;
    let rows = stmt.query_map(params![project_id], |r| r.get::<_, String>(0))?;
    for id in rows {
        ids.push(id?);
    }
    Ok(ids)
}

/// Selects every node (including soft-deleted tombstones) belonging to the
/// given member ids — pushed as-is so deletions propagate on the next push.
fn select_project_nodes(conn: &Connection, member_ids: &[String]) -> Result<Vec<Node>> {
    if member_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = member_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("{SYNC_NODE_SELECT} WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = member_ids
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();
    let nodes = stmt
        .query_map(refs.as_slice(), row_to_node)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(nodes)
}

fn select_project_edges(conn: &Connection, member_ids: &[String]) -> Result<Vec<Edge>> {
    if member_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = member_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "{SYNC_EDGE_SELECT} WHERE from_id IN ({placeholders}) AND to_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut refs: Vec<&dyn rusqlite::types::ToSql> = member_ids
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();
    refs.extend(member_ids.iter().map(|s| s as &dyn rusqlite::types::ToSql));
    let edges = stmt
        .query_map(refs.as_slice(), row_to_edge)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(edges)
}

/// Upserts pulled nodes as-is (server is the source of truth for what it
/// sends back) — includes tombstones, so deletions apply on the client too.
fn apply_pulled_nodes(conn: &Connection, nodes: &[Node]) -> Result<()> {
    for node in nodes {
        conn.execute(
            "INSERT INTO nodes (id, node_type, label, note, source, data, created_at, updated_at,
                    memory_kind, last_accessed_at, access_count, content_hash,
                    created_by, updated_by, deleted_at, sync_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(id) DO UPDATE SET
                 node_type = excluded.node_type, label = excluded.label, note = excluded.note,
                 source = excluded.source, data = excluded.data, created_at = excluded.created_at,
                 updated_at = excluded.updated_at, memory_kind = excluded.memory_kind,
                 last_accessed_at = excluded.last_accessed_at, access_count = excluded.access_count,
                 content_hash = excluded.content_hash, created_by = excluded.created_by,
                 updated_by = excluded.updated_by, deleted_at = excluded.deleted_at,
                 sync_seq = excluded.sync_seq",
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
                node.sync_seq,
            ],
        )?;
    }
    Ok(())
}

/// Applies a pull response's nodes/edges into the local DB (upsert by id,
/// including tombstones). Shared by the bootstrap pull (`au share <server>
/// <token>`, no `SyncConfig` row yet) and `pull_project` below.
pub fn apply_pull(conn: &Connection, pull: &SyncPullResponse) -> Result<()> {
    apply_pulled_nodes(conn, &pull.nodes)?;
    apply_pulled_edges(conn, &pull.edges)?;
    Ok(())
}

fn apply_pulled_edges(conn: &Connection, edges: &[Edge]) -> Result<()> {
    for edge in edges {
        conn.execute(
            "INSERT INTO edges (id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                 from_id = excluded.from_id, to_id = excluded.to_id, relation = excluded.relation,
                 weight = excluded.weight, created_at = excluded.created_at,
                 created_by = excluded.created_by, deleted_at = excluded.deleted_at,
                 sync_seq = excluded.sync_seq",
            params![
                edge.id.to_string(),
                edge.from_id.to_string(),
                edge.to_id.to_string(),
                edge.relation.to_string(),
                edge.weight,
                edge.created_at.to_rfc3339(),
                edge.created_by,
                edge.deleted_at.map(|d| d.to_rfc3339()),
                edge.sync_seq,
            ],
        )?;
    }
    Ok(())
}

/// Pushes everything locally new/changed for one project to its sync
/// server, applies the accepted `server_seq` as the new `last_seq`. Callers
/// that run this automatically (not on explicit user request) MUST treat a
/// network/HTTP error here as non-blocking (FR-006, FR-011).
pub async fn push_project(
    client: &reqwest::Client,
    conn: &Connection,
    cfg: &SyncConfig,
) -> Result<SyncPushResponse> {
    let project_node = crate::graph::find_project_by_label(conn, &cfg.project_label)?
        .ok_or_else(|| anyhow::anyhow!("local project not found: {}", cfg.project_label))?;
    let member_ids = project_member_ids(conn, &project_node.id.to_string())?;
    let nodes = select_project_nodes(conn, &member_ids)?;
    let edges = select_project_edges(conn, &member_ids)?;

    let resp = client
        .post(format!("{}/push", cfg.server_url))
        .bearer_auth(&cfg.token)
        .json(&SyncPushRequest { nodes, edges })
        .send()
        .await
        .with_context(|| format!("failed to reach sync server at {}", cfg.server_url))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("server returned {status}: {body}");
    }

    let push: SyncPushResponse = resp.json().await?;
    set_sync_last_seq(conn, &cfg.project_label, push.server_seq)?;
    Ok(push)
}

/// Pulls everything new since `cfg.last_seq` for one project, applies it
/// locally (including tombstones), and persists the returned `server_seq`
/// as the new `last_seq`. Same non-blocking-on-error contract as `push_project`.
pub async fn pull_project(
    client: &reqwest::Client,
    conn: &Connection,
    cfg: &SyncConfig,
) -> Result<SyncPullResponse> {
    let resp = client
        .get(format!("{}/pull?since={}", cfg.server_url, cfg.last_seq))
        .bearer_auth(&cfg.token)
        .send()
        .await
        .with_context(|| format!("failed to reach sync server at {}", cfg.server_url))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("server returned {status}: {body}");
    }

    let pull: SyncPullResponse = resp.json().await?;
    apply_pull(conn, &pull)?;
    set_sync_last_seq(conn, &cfg.project_label, pull.server_seq)?;
    Ok(pull)
}
