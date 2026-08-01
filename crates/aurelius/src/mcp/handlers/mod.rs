mod crud;
mod search;
mod session;
mod skill;
mod status;
mod task;

pub use crud::*;
pub use search::*;
pub use session::*;
pub use skill::*;
pub use status::*;
pub use task::*;

use aurelius_core::{db, graph, models::NodeType, models::Relation};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) fn db_path() -> PathBuf {
    let base = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("aurelius");
    std::fs::create_dir_all(&base).ok();
    base.join("aurelius.db")
}

pub(crate) fn open_db() -> anyhow::Result<Connection> {
    db::open(&db_path())
}

// ---------------------------------------------------------------------------
// US2: automatic sync at session boundaries (memory_status pulls, memory_session
// pushes). Reuses `aurelius_core::sync::client` — the same push/pull logic `au
// share push/pull` uses — never duplicated here. Best-effort per FR-006/FR-011:
// callers get no `Result` back because a sync failure must never fail the
// surrounding MCP call (T022).
// ---------------------------------------------------------------------------

/// If `project` has a sync-enabled `sync_config` row, pulls whatever's new
/// since the last sync before the caller reads the graph. Logs and swallows
/// any failure (offline server, revoked token, etc.) — local reads proceed
/// with whatever's already on disk.
pub(crate) fn sync_pull_if_enabled(conn: &Connection, project: Option<&str>) {
    let Some(project) = project else { return };
    let cfg = match aurelius_core::sync::client::get_sync_config(conn, project) {
        Ok(Some(cfg)) if cfg.enabled => cfg,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!("sync: could not read sync_config for '{project}': {e}");
            return;
        }
    };

    let client = reqwest::Client::new();
    let outcome = tokio::runtime::Handle::current().block_on(
        aurelius_core::sync::client::pull_project(&client, conn, &cfg),
    );
    match outcome {
        Ok(pull) => tracing::debug!(
            project,
            nodes = pull.nodes.len(),
            edges = pull.edges.len(),
            "sync: pulled before memory_status"
        ),
        Err(e) => {
            tracing::warn!("sync: pull for '{project}' failed, continuing with local data: {e}")
        }
    }
}

/// If `project` has a sync-enabled `sync_config` row, pushes whatever's new
/// locally after the caller's write completes. Same swallow-and-log contract
/// as `sync_pull_if_enabled`.
pub(crate) fn sync_push_if_enabled(conn: &Connection, project: &str) {
    let cfg = match aurelius_core::sync::client::get_sync_config(conn, project) {
        Ok(Some(cfg)) if cfg.enabled => cfg,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!("sync: could not read sync_config for '{project}': {e}");
            return;
        }
    };

    let client = reqwest::Client::new();
    let outcome = tokio::runtime::Handle::current().block_on(
        aurelius_core::sync::client::push_project(&client, conn, &cfg),
    );
    match outcome {
        Ok(push) => tracing::debug!(
            project,
            accepted = push.accepted,
            conflicts = push.conflicts,
            "sync: pushed after memory_session"
        ),
        Err(e) => {
            tracing::warn!("sync: push for '{project}' failed, local write is unaffected: {e}")
        }
    }
}

pub(crate) fn node_brief(node: &aurelius_core::models::Node) -> serde_json::Value {
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
    })
}

pub(crate) fn node_detail(node: &aurelius_core::models::Node) -> serde_json::Value {
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "note": node.note,
        "source": node.source,
        "data": node.data,
        "created_at": node.created_at.to_rfc3339(),
        "memory_kind": node.memory_kind,
        "access_count": node.access_count,
        "created_by": node.created_by,
        "updated_by": node.updated_by,
    })
}

pub(crate) fn node_compact(node: &aurelius_core::models::Node) -> serde_json::Value {
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "note": node.note,
        "created_at": node.created_at.to_rfc3339(),
    })
}

pub(crate) fn edge_brief(edge: &aurelius_core::models::Edge) -> serde_json::Value {
    json!({
        "from": edge.from_id.to_string(),
        "to": edge.to_id.to_string(),
        "relation": edge.relation.to_string(),
        "weight": edge.weight,
    })
}

pub(crate) fn resolve_node(
    conn: &Connection,
    identifier: &str,
) -> anyhow::Result<aurelius_core::models::Node> {
    if let Ok(uuid) = identifier.parse::<Uuid>() {
        if let Some(node) = graph::get_node(conn, &uuid.to_string())? {
            return Ok(node);
        }
    }
    if let Some(node) = graph::find_node_by_label(conn, identifier)? {
        return Ok(node);
    }
    let results = graph::search(conn, identifier, 1)?;
    results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("node not found: {identifier}"))
}

pub(crate) fn parse_node_type(s: &str) -> NodeType {
    match s {
        "project" => NodeType::Project,
        "decision" => NodeType::Decision,
        "concept" => NodeType::Concept,
        "problem" => NodeType::Problem,
        "solution" => NodeType::Solution,
        "person" => NodeType::Person,
        "dependency" => NodeType::Dependency,
        "server" => NodeType::Server,
        "file" => NodeType::File,
        "module" => NodeType::Module,
        "crate" => NodeType::Crate,
        "config" => NodeType::Config,
        "session" => NodeType::Session,
        "language" => NodeType::Language,
        "task" => NodeType::Task,
        "work_log" | "worklog" => NodeType::WorkLog,
        "skill" => NodeType::Skill,
        other => NodeType::Custom(other.to_owned()),
    }
}

pub(crate) fn parse_relation(s: &str) -> anyhow::Result<Relation> {
    Ok(match s {
        "uses" => Relation::Uses,
        "depends_on" => Relation::DependsOn,
        "solves" => Relation::Solves,
        "caused_by" => Relation::CausedBy,
        "inspired_by" => Relation::InspiredBy,
        "conflicts_with" => Relation::ConflictsWith,
        "supersedes" => Relation::Supersedes,
        "belongs_to" => Relation::BelongsTo,
        "related_to" => Relation::RelatedTo,
        "learned_from" => Relation::LearnedFrom,
        "contains" => Relation::Contains,
        "imports" => Relation::Imports,
        "exports" => Relation::Exports,
        "implements" => Relation::Implements,
        "configures" => Relation::Configures,
        "tracked_by" => Relation::TrackedBy,
        "subtask_of" => Relation::SubtaskOf,
        "blocks" => Relation::Blocks,
        _ => anyhow::bail!("unknown relation: {s}"),
    })
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

pub(crate) fn parse_since(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = chrono::Utc::now();
    match s.trim().to_lowercase().as_str() {
        "today" => Some(now.date_naive().and_hms_opt(0, 0, 0)?.and_utc()),
        "yesterday" => Some(
            (now.date_naive() - chrono::Duration::days(1))
                .and_hms_opt(0, 0, 0)?
                .and_utc(),
        ),
        s if s.ends_with('d') => {
            let days: i64 = s.trim_end_matches('d').parse().ok()?;
            Some(now - chrono::Duration::days(days))
        }
        s if s.ends_with('h') => {
            let hours: i64 = s.trim_end_matches('h').parse().ok()?;
            Some(now - chrono::Duration::hours(hours))
        }
        other => other.parse().ok(),
    }
}
