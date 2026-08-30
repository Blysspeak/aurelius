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
    /// Слой 1 памяти: стабильный факт о владельце (роль, предпочтение, табу).
    /// Отдельный тип, а не Concept: у identity-слоя свой бюджет в снапшоте.
    UserFact,
    /// Слой 7 памяти: дистиллят проекта — структурная выжимка сессий и
    /// незакрытых проблем, пересобирается memory_consolidate. Один на проект.
    Digest,
    Custom(String),
}

impl NodeType {
    /// Имена типов, которые понимают все входы разом — и MCP, и CLI. Единый
    /// список: пока он жил только внутри MCP-хендлера, из хука нельзя было
    /// написать то, что пишется инструментом.
    pub const KNOWN: &'static [&'static str] = &[
        "project",
        "decision",
        "concept",
        "problem",
        "solution",
        "person",
        "dependency",
        "server",
        "file",
        "module",
        "crate",
        "config",
        "session",
        "language",
        "task",
        "work_log",
        "skill",
        "user_fact",
        "digest",
    ];

    /// Строгий разбор: только известные имена. `None` оставляет решение
    /// вызывающему — CLI обязан ругнуться на опечатку, MCP заводит `Custom`.
    #[must_use]
    pub fn parse_known(s: &str) -> Option<Self> {
        Some(match s {
            "project" => Self::Project,
            "decision" => Self::Decision,
            "concept" => Self::Concept,
            "problem" => Self::Problem,
            "solution" => Self::Solution,
            "person" => Self::Person,
            "dependency" => Self::Dependency,
            "server" => Self::Server,
            "file" => Self::File,
            "module" => Self::Module,
            "crate" => Self::Crate,
            "config" => Self::Config,
            "session" => Self::Session,
            "language" => Self::Language,
            "task" => Self::Task,
            "work_log" | "worklog" => Self::WorkLog,
            "skill" => Self::Skill,
            "user_fact" => Self::UserFact,
            "digest" => Self::Digest,
            _ => return None,
        })
    }

    /// Мягкий разбор: незнакомое имя становится `Custom`.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        Self::parse_known(s).unwrap_or_else(|| Self::Custom(s.to_owned()))
    }
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
    /// Уточняет, не заменяя: поздняя запись делает раннюю точнее, а не отменяет
    /// её. Отдельно от `Supersedes` (замена) и `RelatedTo` (связь без смысла).
    Refines,
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
    /// Задача → узел прогона, подтвердившего её уликой (спека 007,
    /// data-model.md «Ребро»). Улика лежит и в `data.evidence` задачи —
    /// это ребро лишь даёт обратный путь, от прогона к задаче.
    VerifiedBy,
}

impl Relation {
    /// Словарь связей, единый для MCP и CLI — тот же приём, что и
    /// [`NodeType::KNOWN`]: пока список жил внутри MCP-хендлера, из хука нельзя
    /// было поставить ребро теми же словами, какими его ставит инструмент.
    pub const KNOWN: &'static [&'static str] = &[
        "uses",
        "depends_on",
        "solves",
        "caused_by",
        "inspired_by",
        "conflicts_with",
        "supersedes",
        "refines",
        "belongs_to",
        "related_to",
        "learned_from",
        "contains",
        "imports",
        "exports",
        "implements",
        "configures",
        "tracked_by",
        "subtask_of",
        "blocks",
        "verified_by",
    ];

    /// Строгий разбор имени связи. Дефис принимается наравне с подчёркиванием
    /// (`part-of`, `related-to`): в командной строке дефис — естественная
    /// форма, и считать её опечаткой значило бы отказывать в записи по
    /// орфографии. `part_of` — синоним `belongs_to`, а не отдельная связь:
    /// два имени одного отношения разорвали бы проектные выборки надвое.
    #[must_use]
    pub fn parse_known(s: &str) -> Option<Self> {
        let name = s.trim().to_ascii_lowercase().replace('-', "_");
        Some(match name.as_str() {
            "uses" => Self::Uses,
            "depends_on" => Self::DependsOn,
            "solves" => Self::Solves,
            "caused_by" => Self::CausedBy,
            "inspired_by" => Self::InspiredBy,
            "conflicts_with" => Self::ConflictsWith,
            "supersedes" => Self::Supersedes,
            "refines" => Self::Refines,
            "belongs_to" | "part_of" => Self::BelongsTo,
            "related_to" => Self::RelatedTo,
            "learned_from" => Self::LearnedFrom,
            "contains" => Self::Contains,
            "imports" => Self::Imports,
            "exports" => Self::Exports,
            "implements" => Self::Implements,
            "configures" => Self::Configures,
            "tracked_by" => Self::TrackedBy,
            "subtask_of" => Self::SubtaskOf,
            "blocks" => Self::Blocks,
            "verified_by" => Self::VerifiedBy,
            _ => return None,
        })
    }
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

impl MemoryKind {
    pub const KNOWN: &'static [&'static str] = &["semantic", "episodic"];

    /// `None` на незнакомом имени: подставлять `Semantic` молча нельзя —
    /// именно так эпизод и превращается в вечный факт.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "semantic" => Some(Self::Semantic),
            "episodic" => Some(Self::Episodic),
            _ => None,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{MemoryKind, NodeType, Relation};

    /// `KNOWN` — это то, что CLI печатает в подсказке при опечатке. Список,
    /// разошедшийся с `parse_known`, обещает пользователю тип, который тут же
    /// будет отвергнут.
    #[test]
    fn every_known_node_type_parses_and_round_trips() {
        for name in NodeType::KNOWN {
            let parsed = NodeType::parse_known(name)
                .unwrap_or_else(|| panic!("KNOWN содержит '{name}', но parse_known его не знает"));
            let serialized = serde_json::to_value(&parsed).expect("тип сериализуется");
            assert_eq!(
                serialized.as_str(),
                Some(*name),
                "имя в KNOWN должно совпадать с тем, что уходит в базу"
            );
        }
    }

    #[test]
    fn unknown_node_type_is_rejected_strictly_and_kept_loosely() {
        assert!(NodeType::parse_known("decison").is_none());
        assert!(matches!(NodeType::parse("decison"), NodeType::Custom(s) if s == "decison"));
    }

    /// То же требование, что и к типам узлов: имя из `KNOWN` должно и
    /// разбираться, и ложиться в базу ровно этой строкой — иначе ребро,
    /// поставленное из CLI, не совпадёт с поставленным из MCP.
    #[test]
    fn every_known_relation_parses_and_round_trips() {
        for name in Relation::KNOWN {
            let parsed = Relation::parse_known(name)
                .unwrap_or_else(|| panic!("KNOWN содержит '{name}', но parse_known его не знает"));
            assert_eq!(parsed.to_string(), *name);
        }
    }

    #[test]
    fn relation_accepts_hyphens_and_part_of_alias() {
        assert_eq!(
            Relation::parse_known("part-of").map(|r| r.to_string()),
            Some("belongs_to".to_owned()),
            "part-of — это belongs_to, а не новая связь"
        );
        assert_eq!(
            Relation::parse_known("related-to").map(|r| r.to_string()),
            Some("related_to".to_owned())
        );
        assert!(Relation::parse_known("солвес").is_none());
    }

    #[test]
    fn every_known_memory_kind_parses_and_round_trips() {
        for name in MemoryKind::KNOWN {
            let parsed = MemoryKind::parse(name)
                .unwrap_or_else(|| panic!("KNOWN содержит '{name}', но parse его не знает"));
            assert_eq!(parsed.to_string(), *name);
        }
        assert!(MemoryKind::parse("epizodic").is_none());
    }
}
