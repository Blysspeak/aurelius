use anyhow::Result;
use aurelius_core::{
    graph::{self, ProblemSolved, SessionInput},
    models::{MemoryKind, NodeType, Relation},
    provenance::{self, Provenance},
};
use serde_json::json;

use super::{node_recall, open_db, resolve_node, sync_push_if_enabled};

/// Сколько записей знания отдаёт `memory_recall`. Ответ читает модель с
/// ограниченным окном: двенадцать отранжированных записей она использует, сто
/// — пролистывает.
const RECALL_LIMIT: usize = 12;

/// Хвост эпизодического. Двух хватает, чтобы ответить «чем занимались
/// последний раз»; больше — это уже журнал, а за ним идут в `au journal`.
const RECALL_TAIL_LIMIT: usize = 2;

/// Строки массива параметра, пустой вектор при отсутствии или чужом типе.
fn string_list(params: &serde_json::Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub fn memory_session(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    memory_session_with_conn(&conn, params)
}

/// The body of `memory_session`, taking the connection as an explicit
/// parameter — the same testability trick as `task_update_with_conn`/
/// `task_log_with_conn`: a test sets up its own temp database instead of
/// hitting the real one (`open_db()`).
fn memory_session_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let summary = params
        .get("summary")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'summary' parameter"))?;
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");

    let decisions = string_list(params, "decisions");
    let next_steps = string_list(params, "next_steps");
    let key_files = string_list(params, "key_files");
    let problems_solved: Vec<ProblemSolved> = params
        .get("problems_solved")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    // Метка прогона. Не обязательна, но без неё запись невозможно отличить от
    // вчерашней: id прогона знает только вызывающий, граф его ниоткуда не
    // выведет.
    let agent_session = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Provenance — parsed the same way as `memory_add`: an error in it must
    // not leave a half-written session behind.
    let prov = Provenance::parse(params)?;

    // The same guard as memory_add, before the write. Resolution is not
    // supported here: resolving a subject conflict only goes through
    // memory_add. `exclude: None` — a session is always a new node.
    provenance::guard_subject(conn, prov.subject.as_deref(), false, None)?;

    // Сама запись — общий код с `au session` (graph::record_session). Здесь
    // остаётся только то, что есть у инструмента и нет у CLI: привязка к
    // задачам, подсказка об активных и авто-push синка.
    let written = graph::record_session(
        conn,
        &SessionInput {
            decisions: &decisions,
            problems_solved: &problems_solved,
            next_steps: &next_steps,
            key_files: &key_files,
            agent_session,
            provenance: prov,
            ..SessionInput::new(project, summary, "mcp")
        },
    )?;
    let session = written.session;
    // What actually landed on the node — not what the call sent, but what got
    // written (the two match on a fresh write; on a duplicate this shows the
    // provenance of the session that already existed).
    let response_prov = Provenance::from_data(&session.data);
    let provenance_response = json!({
        "confidence": response_prov.confidence_or_default().as_str(),
        "subject": response_prov.subject,
    });

    if written.duplicate {
        return Ok(json!({
            "id": session.id.to_string(),
            "label": session.label,
            "type": "session",
            "memory_kind": "episodic",
            "duplicate": true,
            "provenance": provenance_response,
        }));
    }

    // Link session to tasks if specified
    let mut linked_tasks = vec![];
    if let Some(tasks) = params.get("tasks").and_then(|t| t.as_array()) {
        for task_ref in tasks {
            if let Some(task_id) = task_ref.as_str() {
                if let Ok(task_node) = resolve_node(conn, task_id) {
                    graph::add_edge(conn, session.id, task_node.id, Relation::RelatedTo, 1.0)?;
                    linked_tasks.push(json!({
                        "id": task_node.id.to_string(),
                        "label": task_node.label,
                        "status": task_node.data.get("status"),
                    }));
                }
            }
        }
    }

    // Always show active tasks for this project as a hint
    let active_tasks: Vec<serde_json::Value> = graph::get_tasks_filtered(
        conn,
        Some(project),
        Some(graph::OPEN_TASK_STATUSES),
        None,
        5,
    )?
    .iter()
    .map(|t| {
        json!({
            "id": t.id.to_string(),
            "label": t.label,
            "status": t.data.get("status"),
            "priority": t.data.get("priority"),
        })
    })
    .collect();

    // US2: push everything new locally for a shared project right after this
    // session write. Best-effort — never fails memory_session (T022).
    sync_push_if_enabled(conn, project);

    // Ровно та беда, ради которой это писалось: имена параметров теперь
    // проверены заслонкой, но правильно названный пустой список выглядел
    // переданным — и решения терялись при ответе "created": true.
    let (stored_fields, dropped_fields) = super::super::params::field_report(params);

    Ok(json!({
        "id": session.id.to_string(),
        "label": session.label,
        "type": "session",
        "memory_kind": "episodic",
        "created": true,
        "decisions_written": written.decisions,
        "problems_written": written.problems,
        "stored_fields": stored_fields,
        "dropped_fields": dropped_fields,
        "linked_tasks": linked_tasks,
        "active_tasks_hint": active_tasks,
        "provenance": provenance_response,
    }))
}

pub fn memory_recall(params: &serde_json::Value) -> Result<serde_json::Value> {
    let topic = params
        .get("topic")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'topic' parameter"))?;
    // Глубина 2 по умолчанию, а не 1. Единица отвечала темой, до узла-хаба
    // которой обход не доходил: измерено 07.09.2026 на теме «ulika» — 6 узлов
    // при 672 связанных с проектом. Разрастание, из-за которого глубину когда-то
    // опустили до единицы, теперь держит `MAX_TRAVERSAL_NODES`, а не заниженная
    // глубина: потолок в 200 узлов стоит внутри самой прогулки.
    let depth = params.get("depth").and_then(|d| d.as_u64()).unwrap_or(2) as u32;

    let conn = open_db()?;
    let traversal = graph::context_with_report_seeded(&conn, topic, depth, graph::RECALL_SEEDS)?;
    let context_nodes = traversal.nodes;

    // Степень внутри найденного подграфа — мера того, насколько запись держит
    // тему, а не насколько часто в её теле встретилось слово. BM25 по телу
    // поднимал наверх дампы сессий: в каждом мёртвом пути `A:\workSpace\ulika\`
    // имя проекта повторяется десятки раз, и частота терма отвечала за
    // релевантность вместо связей.
    let mut degree: std::collections::HashMap<uuid::Uuid, usize> = std::collections::HashMap::new();
    for edge in &traversal.edges {
        *degree.entry(edge.from_id).or_default() += 1;
        *degree.entry(edge.to_id).or_default() += 1;
    }

    let mut knowledge = vec![];
    let mut episodic_tail = vec![];

    for node in &context_nodes {
        // Карточки навыков приходят на SessionStart через `au skills --hook` и
        // в выдаче recall были бы вторым экземпляром того же текста.
        if matches!(node.node_type, NodeType::Skill) {
            continue;
        }
        // Узел проекта — навигация, а не знание: у него нет ни claim, ни note,
        // метка равна имени проекта. По степени он всегда первый (673 ребра у
        // ulika), то есть занимал бы верхнюю строку ответа, ничего не сообщая.
        if matches!(node.node_type, NodeType::Project) {
            continue;
        }
        // Эпизодическое — снимок момента: сессия, срез перед компакцией. Оно
        // отвечает на «что происходило», а спрашивают «что известно», поэтому
        // уходит в хвост, а не смешивается со знанием.
        if matches!(node.memory_kind, MemoryKind::Episodic) {
            episodic_tail.push(node);
        } else {
            knowledge.push(node);
        }
    }

    let rank = |a: &&aurelius_core::models::Node, b: &&aurelius_core::models::Node| {
        let da = degree.get(&a.id).copied().unwrap_or(0);
        let db = degree.get(&b.id).copied().unwrap_or(0);
        db.cmp(&da).then(b.created_at.cmp(&a.created_at))
    };
    knowledge.sort_by(rank);
    episodic_tail.sort_by(rank);

    let shown: Vec<_> = knowledge.iter().take(RECALL_LIMIT).collect();
    let tail: Vec<_> = episodic_tail.iter().take(RECALL_TAIL_LIMIT).collect();

    for node in shown.iter().chain(tail.iter()) {
        // Best effort by design: an access counter must never fail a read.
        if let Err(e) = graph::touch_node(&conn, node.id) {
            tracing::warn!("could not record access for {}: {e}", node.id);
        }
    }

    Ok(json!({
        "topic": topic,
        "knowledge": shown.iter().map(|n| node_recall(n, topic)).collect::<Vec<_>>(),
        "recent": tail.iter().map(|n| node_recall(n, topic)).collect::<Vec<_>>(),
        "shown": shown.len(),
        "matched_knowledge": knowledge.len(),
        "matched_recent": episodic_tail.len(),
        "total_graph_nodes": context_nodes.len(),
        "truncation": {
            "truncated": traversal.truncated_at_depth.is_some(),
            "hidden_nodes": traversal.hidden_nodes,
            "truncated_at_depth": traversal.truncated_at_depth,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelius_core::db;

    /// The same trick as in `task.rs`: a real temp file, not `:memory:` —
    /// `db::open` hard-requires WAL, and `memory_session` hits the user's real
    /// database through `open_db()`, which a test cannot use.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-mcp-session-test-{tag}-{}.db",
                uuid::Uuid::new_v4()
            )))
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

    fn setup() -> (TmpDb, rusqlite::Connection) {
        let tmp = TmpDb::new("setup");
        let conn = db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    /// Task 2c8d25ce: an agent that just ran a command must be able to record
    /// that through `memory_session` — the same provenance parse as
    /// `memory_add`.
    #[test]
    fn memory_session_with_measured_and_evidence_writes_it_on_the_session_node() {
        let (_tmp, conn) = setup();

        let result = memory_session_with_conn(
            &conn,
            &json!({
                "summary": "прогнали verify-run для задачи провенанса",
                "project": "proj-session",
                "confidence": "measured",
                "evidence": "cargo test --workspace",
            }),
        )
        .expect("memory_session");

        assert_eq!(result["provenance"]["confidence"], "measured");

        let session_id = result["id"].as_str().expect("id");
        let node = graph::get_node(&conn, session_id)
            .expect("get_node")
            .expect("session exists");
        let prov = aurelius_core::provenance::Provenance::from_data(&node.data);
        assert_eq!(
            prov.confidence,
            Some(aurelius_core::provenance::Confidence::Measured)
        );
        assert_eq!(prov.evidence.as_deref(), Some("cargo test --workspace"));
    }

    /// Asymmetry: without provenance fields the behavior is unchanged — the
    /// node reads as unverified, same as before task 2c8d25ce.
    #[test]
    fn memory_session_without_provenance_fields_stays_unverified() {
        let (_tmp, conn) = setup();

        let result = memory_session_with_conn(
            &conn,
            &json!({ "summary": "итог без происхождения", "project": "proj-session-plain" }),
        )
        .expect("memory_session");

        assert_eq!(result["provenance"]["confidence"], "unverified");
    }

    /// confidence=measured without evidence — the same refusal as
    /// `memory_add`: a measurement without the command that made it is
    /// inferred, not measured.
    #[test]
    fn memory_session_measured_without_evidence_is_refused() {
        let (_tmp, conn) = setup();

        let err = memory_session_with_conn(
            &conn,
            &json!({
                "summary": "итог без evidence",
                "project": "proj-session-refused",
                "confidence": "measured",
            }),
        )
        .expect_err("measured без evidence обязано быть отказом");
        assert!(format!("{err}").contains("inferred"), "{err}");
    }
}
