mod crud;
mod doc;
mod path;
mod search;
mod secret;
mod session;
mod skill;
mod snapshot;
mod status;
mod task;

pub use crud::*;
pub use doc::*;
pub use path::*;
pub use search::*;
pub use secret::*;
pub use session::*;
pub use skill::*;
pub use snapshot::*;
pub use status::*;
pub use task::*;

use aurelius_core::{db, graph, models::NodeType, models::Relation};
use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

pub(crate) use aurelius_core::db::db_path;

pub(crate) fn open_db() -> anyhow::Result<Connection> {
    Ok(db::open(&db_path())?)
}

// ---------------------------------------------------------------------------
// Stale-binary detection: installing a new `aurelius` binary over
// `~/.local/bin/aurelius` does not touch an already-running MCP server
// process — Claude Code keeps talking to it until the session restarts, and
// until then the agent finds out only from an "unknown tool" error or a
// missing parameter on a tool whose shape changed. The server can tell by
// itself, by comparing its own executable's mtime against the moment it
// started, and say so in `memory_status`.
// ---------------------------------------------------------------------------

/// Set once from `serve()`, before its request loop starts. `server_started_at`
/// below also falls back to `get_or_init`, so a caller that somehow reaches
/// `memory_status` without going through `serve()` still gets a real value
/// instead of a missing `started_at` (no such caller exists today).
static SERVER_STARTED_AT: std::sync::OnceLock<std::time::SystemTime> = std::sync::OnceLock::new();

/// Called once from `serve()` before it starts reading requests.
pub(crate) fn mark_server_started() {
    SERVER_STARTED_AT.get_or_init(std::time::SystemTime::now);
}

/// The moment this process started serving requests.
pub(crate) fn server_started_at() -> std::time::SystemTime {
    *SERVER_STARTED_AT.get_or_init(std::time::SystemTime::now)
}

/// Pure comparison, no filesystem access: `true` means the binary on disk was
/// written after this server process started, so it's running a stale image
/// and a restart is due to pick up the new one.
pub(crate) fn binary_newer_than_start(
    exe_mtime: std::time::SystemTime,
    started_at: std::time::SystemTime,
) -> bool {
    exe_mtime > started_at
}

/// Reads the running executable's own mtime and applies
/// `binary_newer_than_start`. Any failure along the way (no exe path
/// available, mtime unsupported on this platform) yields `None`: this check
/// is a nice-to-have inside `memory_status`, never a reason to fail it.
pub(crate) fn restart_needed() -> Option<bool> {
    let exe = std::env::current_exe().ok()?;
    let mtime = std::fs::metadata(exe).ok()?.modified().ok()?;
    Some(binary_newer_than_start(mtime, server_started_at()))
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

/// Происхождение факта в форме для выдачи.
///
/// Отдаётся ВСЕГДА, а не только когда заполнено: молчание о происхождении и
/// есть та беда, ради которой поля заводились — ложное «флаги выключены»
/// выглядело ровно как измеренное. Отсутствие читается как `unverified`.
fn provenance_brief(node: &aurelius_core::models::Node) -> serde_json::Value {
    let p = aurelius_core::provenance::Provenance::from_data(&node.data);
    json!({
        "confidence": p.confidence_or_default().as_str(),
        "evidence": p.evidence,
        "measured_at": p.measured_at.map(|d| d.to_rfc3339()),
        // Both were written into `data` and neither was ever read back out:
        // `stale` folds `verify_with` in only once a fact is already overdue,
        // and `volatility` — the field that decides when that happens — was
        // invisible until then. A caller asking for a record in full got
        // silence about how fast it rots.
        "volatility": p.volatility.map(aurelius_core::provenance::Volatility::as_str),
        "verify_with": p.verify_with,
        "subject": p.subject,
        "stale": p.staleness(node.created_at, chrono::Utc::now()).map(|s| s.note()),
    })
}

/// `pub`, unlike its neighbours: `au recall` renders the same record the MCP
/// door does. A second renderer in the CLI would drift from this one, and the
/// drift would show up as two answers to one question.
pub fn node_detail(node: &aurelius_core::models::Node) -> serde_json::Value {
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "claim": aurelius_core::provenance::Provenance::from_data(&node.data).claim,
        "note": node.note,
        "source": node.source,
        "data": node.data,
        "created_at": node.created_at.to_rfc3339(),
        "memory_kind": node.memory_kind,
        "access_count": node.access_count,
        "created_by": node.created_by,
        "updated_by": node.updated_by,
        "provenance": provenance_brief(node),
    })
}

pub(crate) fn node_compact(node: &aurelius_core::models::Node) -> serde_json::Value {
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "claim": aurelius_core::provenance::Provenance::from_data(&node.data).claim,
        "note": node.note,
        "created_at": node.created_at.to_rfc3339(),
        "provenance": provenance_brief(node),
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

/// Тот же резолв, что и `resolve_node`, но для мест, где по контракту ручки
/// узел ОБЯЗАН быть задачей (`task_update`/`task_view`/`task_log`) — как и в
/// CLI (`find_task` в `crates/au/src/commands.rs`). Разница только в
/// последнем фолбэке: полнотекстовый поиск ограничен `NodeType::Task`.
///
/// Находка 7 (адверсариальный разбор спеки 007): без этого ограничения
/// нечёткое совпадение по строке могло указать на узел ЛЮБОГО типа — CLI
/// в этом случае честно отвечает «задача не найдена», а MCP молча находил и
/// мутировал первый попавшийся узел другого типа (например, Decision).
/// `resolve_node` выше не трогаем: там любой тип узла законен (общие ручки
/// вроде `memory_relate`).
pub(crate) fn resolve_task_node(
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
    let results = graph::search_typed(conn, identifier, &NodeType::Task, 1)?;
    results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("task not found: {identifier}"))
}

/// Разбор живёт в ядре (`NodeType::parse`), чтобы CLI и MCP не расходились в
/// том, какие типы вообще существуют. Здесь — мягкий вариант: незнакомое имя
/// становится `Custom`, как и было в контракте инструмента.
pub(crate) fn parse_node_type(s: &str) -> NodeType {
    NodeType::parse(s)
}

/// Как и `parse_node_type`, разбор живёт в ядре — иначе `au relate` и
/// `memory_relate` расходятся в том, какие связи вообще существуют.
pub(crate) fn parse_relation(s: &str) -> anyhow::Result<Relation> {
    Relation::parse_known(s).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown relation: {s}. Known: {}",
            Relation::KNOWN.join(", ")
        )
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

#[cfg(test)]
mod stale_binary_tests {
    use super::binary_newer_than_start;
    use std::time::{Duration, SystemTime};

    #[test]
    fn newer_mtime_means_restart_needed() {
        let started_at = SystemTime::now();
        let exe_mtime = started_at + Duration::from_secs(1);
        assert!(binary_newer_than_start(exe_mtime, started_at));
    }

    #[test]
    fn older_mtime_means_no_restart_needed() {
        let started_at = SystemTime::now();
        let exe_mtime = started_at - Duration::from_secs(1);
        assert!(!binary_newer_than_start(exe_mtime, started_at));
    }

    #[test]
    fn equal_mtime_means_no_restart_needed() {
        let started_at = SystemTime::now();
        assert!(!binary_newer_than_start(started_at, started_at));
    }
}
