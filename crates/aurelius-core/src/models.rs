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
    Skill,
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
    /// Git-style `"Name <email>"`, stamped from the local identity config at
    /// creation. `None` for nodes created before sync (migration V6).
    #[serde(default)]
    pub created_by: Option<String>,
    /// Same format as `created_by`; overwritten on every update.
    #[serde(default)]
    pub updated_by: Option<String>,
    /// Soft-delete tombstone set by `memory_forget` instead of a real `DELETE`.
    /// `None` = live.
    #[serde(default)]
    pub deleted_at: Option<DateTime<Utc>>,
    /// Set only by the sync server on upsert (monotonic, shared across all
    /// synced projects on that server). `None` on a client's own local rows
    /// until they've round-tripped through a sync.
    #[serde(default)]
    pub sync_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: Uuid,
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub relation: Relation,
    pub weight: f32,
    pub created_at: DateTime<Utc>,
    /// Same format as `Node.created_by`.
    #[serde(default)]
    pub created_by: Option<String>,
    /// Set in lockstep when either endpoint `Node` is soft-deleted (cascade).
    #[serde(default)]
    pub deleted_at: Option<DateTime<Utc>>,
    /// Same semantics as `Node.sync_seq`.
    #[serde(default)]
    pub sync_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub source: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}
