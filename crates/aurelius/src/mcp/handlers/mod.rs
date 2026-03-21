mod crud;
mod session;
mod status;

pub use crud::*;
pub use session::*;
pub use status::*;

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

pub(crate) fn resolve_node(conn: &Connection, identifier: &str) -> anyhow::Result<aurelius_core::models::Node> {
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
        "yesterday" => Some((now.date_naive() - chrono::Duration::days(1)).and_hms_opt(0, 0, 0)?.and_utc()),
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
