//! Запись сессии в граф: узел `Session` плюс порождённые им решения и пары
//! «проблема — решение».
//!
//! Живёт в ядре, а не в MCP-хендлере, по той же причине, что и
//! [`crate::models::NodeType::KNOWN`]: слой 4 снапшота («Последние сессии»)
//! читается ТОЛЬКО из узлов типа `Session`, а писать их умел один лишь
//! `memory_session` по MCP. Механический хук мог положить лишь `note` — и тот
//! приезжал в слой 5, к вечным фактам, рядом с решениями. Самая важная запись
//! за сессию зависела от того, вспомнит ли модель позвать инструмент.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::models::{MemoryKind, Node, NodeType, Relation};
use crate::provenance::Provenance;

/// Ключ в `data`, под которым лежит идентификатор ПРОГОНА — той сессии агента,
/// что сделала запись.
///
/// Намеренно не `session_id`: под этим именем у решений и проблем уже лежит id
/// узла `Session`, а это другое. Узел графа против прогона харнесса — совпади
/// имена, и однажды выборка «что записано в этом прогоне» молча притащила бы
/// половину чужой сессии.
pub const AGENT_SESSION_KEY: &str = "agent_session";

/// Проставить метку прогона в `data`.
///
/// Без метки объект возвращается как был: «сессия неизвестна» обязана остаться
/// ОТСУТСТВИЕМ ключа, а не пустой строкой. Иначе выборка по сессии начала бы
/// совпадать с записями, которых никто не метил, — а это ровно та беда, от
/// которой метка и заводится.
#[must_use]
pub fn with_agent_session(
    mut data: serde_json::Value,
    agent_session: Option<&str>,
) -> serde_json::Value {
    let Some(id) = agent_session.map(str::trim).filter(|s| !s.is_empty()) else {
        return data;
    };
    if data.is_null() {
        data = serde_json::json!({});
    }
    if let Some(obj) = data.as_object_mut() {
        obj.insert(AGENT_SESSION_KEY.to_owned(), id.into());
    }
    data
}

/// Всё, что записано в прогоне `agent_session`, в порядке появления.
///
/// Это вторая половина метки: пометить и не уметь выбрать — то же самое, что не
/// метить. Порядок по возрастанию `created_at` — журнал читают сверху вниз, как
/// он писался.
pub fn nodes_by_agent_session(
    conn: &Connection,
    agent_session: &str,
    limit: usize,
) -> Result<Vec<Node>> {
    let id = agent_session.trim();
    if id.is_empty() {
        anyhow::bail!("пустой идентификатор сессии — выбирать нечего");
    }
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes
         WHERE json_extract(data, '$.agent_session') = ?1 AND deleted_at IS NULL
         ORDER BY created_at ASC
         LIMIT ?2",
    )?;
    let nodes = stmt
        .query_map(rusqlite::params![id, limit as i64], super::row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Пара «что сломалось — как починили». В графе это два узла и ребро `solves`
/// между ними: половина пары бессмысленна, поэтому и в типе они неразделимы.
#[derive(Debug, Clone, Deserialize)]
pub struct ProblemSolved {
    pub problem: String,
    pub solution: String,
}

/// Что случилось за сессию. Обязателен только `summary` — остальное
/// необязательные ветки одной записи.
#[derive(Debug)]
pub struct SessionInput<'a> {
    /// Проект, к которому сессия привязывается ребром `belongs_to`. Без него
    /// запись не попадёт ни в одну проектную выборку.
    pub project: &'a str,
    pub summary: &'a str,
    pub decisions: &'a [String],
    pub problems_solved: &'a [ProblemSolved],
    /// Хвосты на следующий раз: их читает `consolidate` при сборке дистиллята.
    pub next_steps: &'a [String],
    pub key_files: &'a [String],
    /// Чем записано — `mcp` или `cli`; видно в `source` узла.
    pub source: &'a str,
    /// Идентификатор прогона агента. Им метится и сам узел сессии, и всё, что
    /// она породила: без метки хук конца сессии не отличит свои записи от
    /// вчерашних. См. [`AGENT_SESSION_KEY`].
    pub agent_session: Option<&'a str>,
    /// Provenance of the record: the same parse as `memory_add` (see
    /// `Provenance::parse`). Lands on the session node whole; the decision and
    /// problem/solution nodes the session spawns alongside it only get
    /// [`Provenance::inherited`] from it — without `subject` or `claim`.
    pub provenance: Provenance,
}

impl<'a> SessionInput<'a> {
    /// Минимальная запись: проект и summary.
    #[must_use]
    pub fn new(project: &'a str, summary: &'a str, source: &'a str) -> Self {
        Self {
            project,
            summary,
            decisions: &[],
            problems_solved: &[],
            next_steps: &[],
            key_files: &[],
            source,
            agent_session: None,
            provenance: Provenance::default(),
        }
    }
}

/// Итог записи. `duplicate` — повтор той же сессии (тот же проект и тот же
/// summary): узел вернётся прежний, ничего нового не создастся.
#[derive(Debug)]
pub struct SessionWritten {
    pub session: Node,
    pub duplicate: bool,
    pub decisions: usize,
    pub problems: usize,
}

/// SHA-256 по паре «проект + summary» — ключ дедупликации сессий. Хук,
/// сработавший дважды за один повод, должен оставить одну запись.
fn content_hash(project: &str, summary: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project.as_bytes());
    hasher.update(b"|");
    hasher.update(summary.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}...")
    }
}

/// Записать сессию: узел `Session` (эпизодический — снимок момента, он обязан
/// стареть сам), ребро к проекту, узлы решений и пары «проблема — решение».
///
/// Возвращает `duplicate: true`, если сессия с тем же `project + summary` уже
/// записана; ничего не создаёт и ничего не переписывает в этом случае.
pub fn record_session(conn: &Connection, input: &SessionInput<'_>) -> Result<SessionWritten> {
    let summary = input.summary.trim();
    if summary.is_empty() {
        anyhow::bail!("пустой summary — записывать нечего");
    }
    let project = input.project.trim();
    if project.is_empty() {
        anyhow::bail!("пустой project — запись не найдётся ни одной проектной выборкой");
    }

    let hash = content_hash(project, summary);
    if let Some(existing) = super::find_node_by_content_hash(conn, &hash)? {
        return Ok(SessionWritten {
            session: existing,
            duplicate: true,
            decisions: 0,
            problems: 0,
        });
    }

    let mut data = serde_json::Map::new();
    data.insert("project".to_owned(), project.into());
    if !input.next_steps.is_empty() {
        data.insert("next_steps".to_owned(), input.next_steps.into());
    }
    if !input.key_files.is_empty() {
        data.insert("key_files".to_owned(), input.key_files.into());
    }
    let mut session_data = serde_json::Value::Object(data);
    input.provenance.write_into(&mut session_data);

    let label = format!("[{project}] {}", Utc::now().format("%Y-%m-%d %H:%M"));
    let session = super::add_node_full(
        conn,
        NodeType::Session,
        &label,
        Some(summary),
        input.source,
        with_agent_session(session_data, input.agent_session),
        MemoryKind::Episodic,
        Some(&hash),
    )?;

    let proj_node = match super::find_project_by_label(conn, project)? {
        Some(n) => n,
        None => super::add_node(
            conn,
            NodeType::Project,
            project,
            None,
            input.source,
            serde_json::json!({ "auto_created": true }),
        )?,
    };
    super::add_edge(conn, session.id, proj_node.id, Relation::BelongsTo, 1.0)?;

    // Task 2c8d25ce: nodes the session spawns alongside it inherit what backs
    // the record (confidence/evidence/measured_at/...), but not
    // subject/claim — both belong to exactly one fact, see
    // `Provenance::inherited`.
    let inherited = input.provenance.inherited();

    let mut decisions = 0;
    for text in input.decisions {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let mut node_data = serde_json::json!({ "session_id": session.id.to_string() });
        inherited.write_into(&mut node_data);
        let node = super::add_node(
            conn,
            NodeType::Decision,
            &format!("[{project}] {}", truncate(text, 60)),
            Some(text),
            input.source,
            with_agent_session(node_data, input.agent_session),
        )?;
        super::add_edge(conn, session.id, node.id, Relation::Contains, 1.0)?;
        super::add_edge(conn, node.id, proj_node.id, Relation::BelongsTo, 1.0)?;
        decisions += 1;
    }

    let mut problems = 0;
    for pair in input.problems_solved {
        let (problem, solution) = (pair.problem.trim(), pair.solution.trim());
        if problem.is_empty() || solution.is_empty() {
            continue;
        }
        let mut prob_data = serde_json::json!({ "session_id": session.id.to_string() });
        inherited.write_into(&mut prob_data);
        let prob = super::add_node(
            conn,
            NodeType::Problem,
            &format!("[{project}] {}", truncate(problem, 60)),
            Some(problem),
            input.source,
            with_agent_session(prob_data, input.agent_session),
        )?;
        let mut sol_data = serde_json::json!({ "session_id": session.id.to_string() });
        inherited.write_into(&mut sol_data);
        let sol = super::add_node(
            conn,
            NodeType::Solution,
            &format!("[{project}] {}", truncate(solution, 60)),
            Some(solution),
            input.source,
            with_agent_session(sol_data, input.agent_session),
        )?;
        super::add_edge(conn, sol.id, prob.id, Relation::Solves, 1.0)?;
        super::add_edge(conn, session.id, prob.id, Relation::Contains, 1.0)?;
        super::add_edge(conn, session.id, sol.id, Relation::Contains, 1.0)?;
        super::add_edge(conn, prob.id, proj_node.id, Relation::BelongsTo, 1.0)?;
        super::add_edge(conn, sol.id, proj_node.id, Relation::BelongsTo, 1.0)?;
        problems += 1;
    }

    Ok(SessionWritten {
        session,
        duplicate: false,
        decisions,
        problems,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("aurelius-session-test-{}.db", uuid::Uuid::new_v4())),
            )
        }
    }

    impl Drop for TmpDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.as_os_str().to_owned();
                p.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(p));
            }
        }
    }

    fn setup() -> (TmpDb, Connection) {
        let tmp = TmpDb::new();
        let conn = db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    /// Смысл всей команды: записанное из хука обязано приехать в слой 4
    /// («Последние сессии»), а не в слой 5 к вечным решениям. Проверяем ровно
    /// это — через тот же сборщик снапшота, которым читает хук SessionStart.
    #[test]
    fn recorded_session_lands_in_the_sessions_layer() {
        let (_tmp, conn) = setup();
        let decisions = ["коды возврата разведены: 1 — вызов, 2 — хранилище".to_owned()];
        let input = SessionInput {
            decisions: &decisions,
            ..SessionInput::new("тестпроект", "снимок перед компакцией", "cli")
        };

        let written = record_session(&conn, &input).expect("record session");
        assert!(!written.duplicate);
        assert_eq!(written.decisions, 1);

        let facts = super::super::snapshot_facts(&conn, Some("тестпроект")).expect("facts");
        let session_texts: Vec<&str> = facts
            .facts
            .iter()
            .filter(|f| f.kind == "session")
            .map(|f| f.text.as_str())
            .collect();
        assert!(
            session_texts.contains(&"снимок перед компакцией"),
            "запись должна читаться как сессия (слой 4), а не как решение; \
             факты сессий: {session_texts:?}"
        );

        let decision_texts: Vec<&str> = facts
            .facts
            .iter()
            .filter(|f| f.kind == "decision")
            .map(|f| f.text.as_str())
            .collect();
        assert!(
            !decision_texts.contains(&"снимок перед компакцией"),
            "summary сессии не должен дублироваться в слой решений"
        );
        assert_eq!(
            decision_texts.len(),
            1,
            "переданное решение должно стать отдельным узлом слоя 5"
        );
    }

    /// Хук срабатывает дважды за один повод (авто- и ручная компакция). Второй
    /// заход обязан вернуть ту же сессию, а не завести близнеца.
    #[test]
    fn same_summary_twice_is_one_session() {
        let (_tmp, conn) = setup();
        let input = SessionInput::new("тестпроект", "один и тот же итог", "cli");

        let first = record_session(&conn, &input).expect("first");
        let second = record_session(&conn, &input).expect("second");

        assert!(!first.duplicate);
        assert!(second.duplicate, "повтор обязан опознаться как дубль");
        assert_eq!(first.session.id, second.session.id);

        let sessions =
            super::super::get_nodes_by_type(&conn, &NodeType::Session).expect("sessions");
        assert_eq!(sessions.len(), 1, "близнец не должен появиться");
    }

    #[test]
    fn problem_solution_pair_is_linked_by_solves() {
        let (_tmp, conn) = setup();
        let pairs = [ProblemSolved {
            problem: "снимок ложился в слой решений".to_owned(),
            solution: "хук пишет узел Session, а не note".to_owned(),
        }];
        let input = SessionInput {
            problems_solved: &pairs,
            ..SessionInput::new("тестпроект", "итог сессии с починкой", "cli")
        };

        let written = record_session(&conn, &input).expect("record");
        assert_eq!(written.problems, 1);

        let solutions = super::super::get_nodes_by_type(&conn, &NodeType::Solution).expect("nodes");
        let problems = super::super::get_nodes_by_type(&conn, &NodeType::Problem).expect("nodes");
        let (sol, prob) = (&solutions[0], &problems[0]);
        let edge =
            super::super::find_edge(&conn, sol.id, prob.id, &Relation::Solves).expect("find edge");
        assert!(edge.is_some(), "решение обязано указывать на свою проблему");
    }

    #[test]
    fn empty_summary_is_refused() {
        let (_tmp, conn) = setup();
        let input = SessionInput::new("тестпроект", "   ", "cli");
        assert!(record_session(&conn, &input).is_err());
    }

    /// Метка обязана лечь не только на саму сессию, но и на всё, что она
    /// породила: хук конца сессии собирает свои записи, а не одну заглавную.
    #[test]
    fn the_run_stamps_the_session_and_everything_it_spawned() {
        let (_tmp, conn) = setup();
        let decisions = ["метка прогона легла в data".to_owned()];
        let pairs = [ProblemSolved {
            problem: "журнал не различал сессии".to_owned(),
            solution: "запись метится прогоном".to_owned(),
        }];
        let input = SessionInput {
            decisions: &decisions,
            problems_solved: &pairs,
            agent_session: Some("run-alpha"),
            ..SessionInput::new("тестпроект", "итог прогона alpha", "cli")
        };
        record_session(&conn, &input).expect("record");

        let written = nodes_by_agent_session(&conn, "run-alpha", 50).expect("journal");
        assert_eq!(
            written.len(),
            4,
            "сессия, решение, проблема и решение проблемы — все четыре записи прогона: {:?}",
            written.iter().map(|n| &n.label).collect::<Vec<_>>()
        );
        assert!(written
            .iter()
            .any(|n| matches!(n.node_type, NodeType::Session)));
    }

    /// Главное свойство: чужой прогон и непомеченная запись в выборку не лезут.
    /// Ради этого метка и заводилась.
    #[test]
    fn another_run_and_unstamped_records_stay_out() {
        let (_tmp, conn) = setup();
        record_session(
            &conn,
            &SessionInput {
                agent_session: Some("run-alpha"),
                ..SessionInput::new("тестпроект", "вчерашний итог", "cli")
            },
        )
        .expect("alpha");
        record_session(
            &conn,
            &SessionInput {
                agent_session: Some("run-beta"),
                ..SessionInput::new("тестпроект", "сегодняшний итог", "cli")
            },
        )
        .expect("beta");
        record_session(
            &conn,
            &SessionInput::new("тестпроект", "итог без метки", "cli"),
        )
        .expect("unstamped");

        let beta = nodes_by_agent_session(&conn, "run-beta", 50).expect("journal");
        assert_eq!(beta.len(), 1);
        assert_eq!(beta[0].note.as_deref(), Some("сегодняшний итог"));

        assert!(
            nodes_by_agent_session(&conn, "run-gamma", 50)
                .expect("journal")
                .is_empty(),
            "прогон, которого не было, не должен ничего находить"
        );
        assert!(
            nodes_by_agent_session(&conn, "  ", 50).is_err(),
            "пустой идентификатор — ошибка вызова, а не выборка всего подряд"
        );
    }

    /// «Сессия неизвестна» обязана остаться отсутствием ключа. Пустая строка в
    /// `data` однажды совпала бы с выборкой и притащила чужое.
    #[test]
    fn an_unknown_run_leaves_no_key_behind() {
        assert_eq!(
            with_agent_session(serde_json::json!({"a": 1}), None),
            serde_json::json!({"a": 1})
        );
        assert_eq!(
            with_agent_session(serde_json::json!({"a": 1}), Some("   ")),
            serde_json::json!({"a": 1})
        );
        assert_eq!(
            with_agent_session(serde_json::json!({}), Some(" run-alpha ")),
            serde_json::json!({ AGENT_SESSION_KEY: "run-alpha" })
        );
    }
}
