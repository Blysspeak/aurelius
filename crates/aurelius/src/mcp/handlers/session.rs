use anyhow::Result;
use aurelius_core::{
    graph::{self, ProblemSolved, SessionInput},
    models::Relation,
    provenance::{self, Provenance},
};
use serde_json::json;

use super::{node_recall, open_db, query_vector_for_topic, resolve_node, sync_push_if_enabled};

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

/// Сверка присланного с записанным (задача 582b1c42). Каждый элемент
/// `decisions`/`problems_solved`, который не станет узлом, называется: поле,
/// индекс, причина. Возвращает отказы и число элементов, которые должны лечь.
fn preflight_children(params: &serde_json::Value) -> (Vec<serde_json::Value>, usize, usize) {
    let mut rejected = vec![];
    let mut decisions_ok = 0;
    for (i, v) in params
        .get("decisions")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .enumerate()
    {
        match v.as_str().map(str::trim) {
            None => {
                rejected.push(json!({ "field": "decisions", "index": i, "reason": "не строка" }))
            }
            Some("") => rejected
                .push(json!({ "field": "decisions", "index": i, "reason": "пустая строка" })),
            Some(_) => decisions_ok += 1,
        }
    }
    let mut problems_ok = 0;
    for (i, v) in params
        .get("problems_solved")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .enumerate()
    {
        let reason = match serde_json::from_value::<ProblemSolved>(v.clone()) {
            Err(_) => Some("нужен объект с полями problem и solution строками"),
            Ok(p) if p.problem.trim().is_empty() || p.solution.trim().is_empty() => {
                Some("пустой problem или solution")
            }
            Ok(_) => None,
        };
        match reason {
            Some(r) => {
                rejected.push(json!({ "field": "problems_solved", "index": i, "reason": r }))
            }
            None => problems_ok += 1,
        }
    }
    (rejected, decisions_ok, problems_ok)
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
    // Ссылка, которая никуда не привела, раньше просто пропускалась: `tasks`
    // при этом оставался в `stored_fields`, то есть ответ утверждал, что поле
    // принято. Теперь несопоставленные ссылки называются поимённо.
    let mut unresolved_tasks: Vec<String> = vec![];
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
                } else {
                    unresolved_tasks.push(task_id.to_owned());
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
    let (mut stored_fields, mut dropped_fields) = super::super::params::field_report(params);

    // `field_report` смотрит на ЗАПРОС, а не на запись: он делит присланное на
    // непустое и пустое. Непустой список задач, из которого не сопоставилась ни
    // одна ссылка, попадал в `stored_fields` — поле числилось принятым, хотя не
    // легло никуда. Пустой `dropped_fields` читается как «всё принято», и
    // опереться на него было нельзя.
    // Сверка ожидания с фактом: отказ, случившийся внутри записи (любой
    // будущий фильтр), тоже виден — ноль в счётчике сравнивается с ожиданием.
    let (mut rejected, decisions_expected, problems_expected) = preflight_children(params);
    if written.decisions < decisions_expected {
        rejected.push(json!({
            "field": "decisions",
            "reason": format!("записано {} из {decisions_expected}", written.decisions),
        }));
    }
    if written.problems < problems_expected {
        rejected.push(json!({
            "field": "problems_solved",
            "reason": format!("записано {} из {problems_expected}", written.problems),
        }));
    }
    for r in &rejected {
        if let Some(field) = r["field"].as_str() {
            // Поле, из которого не легло ничего, принятым не числится.
            let none_written = match field {
                "decisions" => written.decisions == 0,
                _ => written.problems == 0,
            };
            if none_written {
                stored_fields.retain(|f| f != field);
            }
            if !dropped_fields.iter().any(|f| f == field) {
                dropped_fields.push(field.to_owned());
            }
        }
    }
    dropped_fields.sort();

    if !unresolved_tasks.is_empty() && linked_tasks.is_empty() {
        stored_fields.retain(|f| f != "tasks");
        dropped_fields.push("tasks".to_owned());
        dropped_fields.sort();
    }

    let mut response = json!({
        "id": session.id.to_string(),
        "label": session.label,
        "type": "session",
        "memory_kind": "episodic",
        "created": true,
        "decisions_written": written.decisions,
        "problems_written": written.problems,
        "complete": rejected.is_empty(),
        "rejected": rejected,
        "stored_fields": stored_fields,
        "dropped_fields": dropped_fields,
        "linked_tasks": linked_tasks,
        "unresolved_tasks": unresolved_tasks,
        "active_tasks_hint": active_tasks,
        "provenance": provenance_response,
    });
    super::super::params::report_shortened_claim(&response_prov, &mut response);
    Ok(response)
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
    let now = chrono::Utc::now();
    // Обход, отсев, порядок и срезы — общий код ядра
    // (`graph::recall_selection`/`recall_selection_hybrid`). Он здесь не
    // повторяется: `au eval` мерит боевой путь только пока путь один, а не
    // копия в обработчике. Вопреки имени, `memory_recall` берёт ТЕМУ и ищет
    // по ней — не точный ключ, как CLI-команда `au recall` (та осталась вне
    // этой волны намеренно, см. отчёт агента), поэтому именно этот
    // инструмент и получает векторную половину (спека 011).
    let (vector, notice) = query_vector_for_topic(topic);
    let (selection, vector_notice) = match vector {
        Some(vector) => {
            match graph::recall_selection_hybrid(&conn, topic, depth, now, Some(&vector)) {
                Ok(s) => (s, None),
                Err(e) => (
                    graph::recall_selection(&conn, topic, depth, now)?,
                    Some(format!(
                        "гибридный обход не выполнился, отвечаю по полнотекстовому — {e}"
                    )),
                ),
            }
        }
        None => (graph::recall_selection(&conn, topic, depth, now)?, notice),
    };

    // Инкремент `access_count` — единственное, что осталось от сборки в
    // обработчике, и переезжать ему некуда: фикстура прогона открывается
    // только на чтение, и одна эта запись на общем пути роняла бы каждый
    // кейс `recall_top5`.
    for node in selection.knowledge.iter().chain(selection.recent.iter()) {
        // Best effort by design: an access counter must never fail a read.
        if let Err(e) = graph::touch_node(&conn, node.id) {
            tracing::warn!("could not record access for {}: {e}", node.id);
        }
    }

    Ok(json!({
        "topic": topic,
        "knowledge": selection
            .knowledge
            .iter()
            .map(|n| node_recall(n, topic))
            .collect::<Vec<_>>(),
        "recent": selection
            .recent
            .iter()
            .map(|n| node_recall(n, topic))
            .collect::<Vec<_>>(),
        "shown": selection.knowledge.len(),
        "matched_knowledge": selection.matched_knowledge,
        "matched_recent": selection.matched_recent,
        "total_graph_nodes": selection.total_graph_nodes,
        "truncation": {
            "truncated": selection.truncated_at_depth.is_some(),
            "hidden_nodes": selection.hidden_nodes,
            "truncated_at_depth": selection.truncated_at_depth,
        },
        // `None` — гибрид сработал, `Some` — причина отката к
        // полнотекстовому обходу, отдельным полем (spec.md, ограничение №2).
        "vector_notice": vector_notice,
    }))
}

/// `memory_journal` — MCP door onto `au journal --session <s> --limit <n>`
/// (`graph::nodes_by_agent_session`): every node written under one agent
/// session (`data.agent_session`, the tag `memory_add`/`memory_session`
/// stamp when a `session_id` is passed), oldest first. What an end-of-session
/// hook replays to know what it itself wrote — not the project's whole
/// traffic, and not `memory_recall`'s topic search: this is an exact key
/// (the session id), addressed the same way `au journal --session` is.
pub fn memory_journal(params: &serde_json::Value) -> Result<serde_json::Value> {
    let session_id = params
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'session_id' parameter"))?;
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;

    let conn = open_db()?;
    let nodes = graph::nodes_by_agent_session(&conn, session_id, limit)?;

    Ok(json!({
        "session_id": session_id,
        "count": nodes.len(),
        "entries": nodes.iter().map(|n| json!({
            "id": n.id.to_string(),
            "type": n.node_type,
            "label": n.label,
            "created_at": n.created_at.to_rfc3339(),
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelius_core::{db, models::NodeType};

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

    /// Пустой `dropped_fields` читается как «всё принято», и на нём строят
    /// решения. Ссылка на задачу, которая никуда не привела, раньше молча
    /// пропускалась, а `tasks` оставался среди принятых полей — ответ утверждал
    /// то, чего не сделал.
    #[test]
    fn an_unresolvable_task_reference_is_named_not_swallowed() {
        let (_tmp, conn) = setup();

        let result = memory_session_with_conn(
            &conn,
            &json!({
                "summary": "итог со ссылкой в никуда",
                "project": "proj-session-tasks",
                "tasks": ["нет-такой-задачи-12345"],
            }),
        )
        .expect("memory_session");

        let stored: Vec<&str> = result["stored_fields"]
            .as_array()
            .expect("stored_fields")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let dropped: Vec<&str> = result["dropped_fields"]
            .as_array()
            .expect("dropped_fields")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        assert!(
            !stored.contains(&"tasks"),
            "непривязанные задачи не должны числиться принятыми: {stored:?}"
        );
        assert!(
            dropped.contains(&"tasks"),
            "непринятое поле обязано быть названо: {dropped:?}"
        );
        assert_eq!(
            result["unresolved_tasks"][0], "нет-такой-задачи-12345",
            "ссылка называется поимённо, а не общим числом"
        );
    }

    /// Обратная сторона: сопоставившаяся ссылка оставляет `tasks` принятым.
    #[test]
    fn a_resolvable_task_reference_keeps_the_field_stored() {
        let (_tmp, conn) = setup();
        let task = graph::add_node(
            &conn,
            NodeType::Task,
            "[proj-session-tasks] живая задача",
            None,
            "test",
            json!({ "status": "active" }),
        )
        .expect("task");

        let result = memory_session_with_conn(
            &conn,
            &json!({
                "summary": "итог с живой ссылкой",
                "project": "proj-session-tasks",
                "tasks": [task.id.to_string()],
            }),
        )
        .expect("memory_session");

        let dropped: Vec<&str> = result["dropped_fields"]
            .as_array()
            .expect("dropped_fields")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(!dropped.contains(&"tasks"), "{dropped:?}");
        assert_eq!(result["linked_tasks"].as_array().map(Vec::len), Some(1));
    }

    /// Задача 582b1c42: дочерний узел, который не лёг, пропадал молча —
    /// ответ `created: true`, счётчик ноль, `dropped_fields` пустой. Теперь
    /// каждый непринятый элемент назван: поле, индекс, причина.
    #[test]
    fn a_child_that_was_not_written_is_named_not_swallowed() {
        let (_tmp, conn) = setup();

        let result = memory_session_with_conn(
            &conn,
            &json!({
                "summary": "итог с кривой парой",
                "project": "proj-session-rejected",
                "decisions": ["живое решение", "   "],
                "problems_solved": [
                    { "problem": "есть", "solution": "есть" },
                    { "problem": "без решения", "fix": "не то поле" },
                ],
            }),
        )
        .expect("memory_session");

        assert_eq!(result["decisions_written"], 1);
        assert_eq!(result["problems_written"], 1);
        assert_eq!(result["complete"], false, "{result}");
        let rejected = result["rejected"].as_array().expect("rejected");
        let named: Vec<(String, u64)> = rejected
            .iter()
            .map(|r| {
                (
                    r["field"].as_str().unwrap_or_default().to_owned(),
                    r["index"].as_u64().unwrap_or(u64::MAX),
                )
            })
            .collect();
        assert!(named.contains(&("decisions".to_owned(), 1)), "{named:?}");
        assert!(
            named.contains(&("problems_solved".to_owned(), 1)),
            "{named:?}"
        );
        assert!(rejected
            .iter()
            .all(|r| r["reason"].as_str().is_some_and(|s| !s.is_empty())));
        let dropped = result["dropped_fields"].as_array().expect("dropped");
        assert!(dropped.contains(&json!("problems_solved")), "{dropped:?}");
    }

    /// Всё легло — отчёт полный, отказов нет.
    #[test]
    fn a_fully_written_session_reports_complete() {
        let (_tmp, conn) = setup();
        let result = memory_session_with_conn(
            &conn,
            &json!({
                "summary": "итог целиком",
                "project": "proj-session-rejected",
                "decisions": ["одно"],
                "problems_solved": [{ "problem": "п", "solution": "р" }],
            }),
        )
        .expect("memory_session");
        assert_eq!(result["complete"], true, "{result}");
        assert_eq!(result["rejected"], json!([]));
    }

    #[test]
    fn a_shortened_session_claim_is_named_in_the_response() {
        let (_tmp, conn) = setup();
        let long = "слово ".repeat(80);
        let result = memory_session_with_conn(
            &conn,
            &json!({ "summary": "итог с длинным claim", "project": "proj-claim", "claim": long }),
        )
        .expect("memory_session");
        let node = aurelius_core::graph::get_node(&conn, result["id"].as_str().expect("id"))
            .expect("get")
            .expect("node");
        let n = aurelius_core::provenance::Provenance::from_data(&node.data)
            .claim
            .expect("claim")
            .chars()
            .count();
        assert_eq!(
            result["warnings"],
            json!([format!(
                "claim сокращён до {n} символов, полный текст сохранён в claim_full"
            )])
        );
        let stored = result["stored_fields"].as_array().expect("stored_fields");
        assert!(stored.contains(&json!("claim_auto")) && stored.contains(&json!("claim_full")));

        let short = memory_session_with_conn(
            &conn,
            &json!({ "summary": "итог с коротким claim", "project": "proj-claim", "claim": "коротко" }),
        )
        .expect("memory_session");
        assert!(short.get("warnings").is_none(), "{short}");
    }

    /// `memory_recall` items are `node_recall` records, which carry no
    /// `stale` field: probe notes are not grafted on, and the shape stays
    /// exactly this key set.
    #[test]
    fn memory_recall_item_shape_has_no_stale_field() {
        let (_tmp, conn) = setup();
        let node = graph::add_node(
            &conn,
            NodeType::Concept,
            "recall shape",
            Some("заметка"),
            "test",
            json!({}),
        )
        .expect("add node");
        let record = node_recall(&node, "recall");
        let mut keys: Vec<&str> = record
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "claim",
                "confidence",
                "created_at",
                "id",
                "label",
                "subject",
                "type",
                "window"
            ]
        );
    }
}
