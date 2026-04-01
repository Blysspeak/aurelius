use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Project,
    Decision,
    Concept,
    Problem,
    Solution,
    Person,
    Dependency,
    Server,
    File,
    Module,
    Crate,
    Config,
    Session,
    Language,
    Task,
    WorkLog,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Uses,
    DependsOn,
    Solves,
    CausedBy,
    InspiredBy,
    ConflictsWith,
    Supersedes,
    BelongsTo,
    RelatedTo,
    LearnedFrom,
    Contains,
    Imports,
    Exports,
    Implements,
    Configures,
    TrackedBy,
    SubtaskOf,
    Blocks,
}

impl std::fmt::Display for Relation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "related_to".to_owned());
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Semantic,
    Episodic,
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryKind::Semantic => write!(f, "semantic"),
            MemoryKind::Episodic => write!(f, "episodic"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: Uuid,
    pub node_type: NodeType,
    pub label: String,
    pub note: Option<String>,
    pub source: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub memory_kind: MemoryKind,
    pub last_accessed_at: DateTime<Utc>,
    pub access_count: i64,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: Uuid,
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub relation: Relation,
    pub weight: f32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub source: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}
