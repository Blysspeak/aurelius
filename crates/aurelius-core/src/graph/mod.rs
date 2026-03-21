mod crud;
mod search;
mod traverse;

pub use crud::*;
pub use search::*;
pub use traverse::*;

use crate::models::{Edge, MemoryKind, Node};
use chrono::Utc;

pub(crate) fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    let memory_kind_str: String = row
        .get::<_, String>(8)
        .unwrap_or_else(|_| "semantic".to_owned());
    let memory_kind = match memory_kind_str.as_str() {
        "episodic" => MemoryKind::Episodic,
        _ => MemoryKind::Semantic,
    };

    let last_accessed_str: Option<String> = row.get(9).ok();
    let last_accessed_at = last_accessed_str
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(Utc::now);

    Ok(Node {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        node_type: serde_json::from_str(&row.get::<_, String>(1)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        label: row.get(2)?,
        note: row.get(3)?,
        source: row.get(4)?,
        data: serde_json::from_str(&row.get::<_, String>(5)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_at: row
            .get::<_, String>(6)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        updated_at: row
            .get::<_, String>(7)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        memory_kind,
        last_accessed_at,
        access_count: row.get(10).unwrap_or(0),
        content_hash: row.get(11).ok().and_then(|v: Option<String>| v),
    })
}

pub(crate) fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        from_id: row
            .get::<_, String>(1)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        to_id: row
            .get::<_, String>(2)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        relation: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        weight: row.get(4)?,
        created_at: row
            .get::<_, String>(5)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
    })
}
