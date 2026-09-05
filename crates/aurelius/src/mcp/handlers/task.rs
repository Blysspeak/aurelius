use anyhow::Result;
use aurelius_core::{
    graph,
    models::{MemoryKind, NodeType, Relation},
    provenance::{self, Provenance},
};
use serde_json::json;

use super::{node_compact, node_detail, open_db, resolve_task_node, truncate};

pub fn task_create(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    task_create_with_conn(&conn, params)
}

/// The body of `task_create`, taking the connection as an explicit
/// parameter — the same testability trick as `task_update_with_conn`.
fn task_create_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let title = params
        .get("title")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'title' parameter"))?;
    let description = params.get("description").and_then(|d| d.as_str());
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");
    let priority = params
        .get("priority")
        .and_then(|p| p.as_str())
        .unwrap_or("medium");
    let acceptance_criteria = params
        .get("acceptance_criteria")
        .cloned()
        .unwrap_or(json!([]));

    // Provenance is parsed FIRST, as in memory_add: an error in it must not
    // leave a half-written task behind.
    let prov = Provenance::parse(params)?;

    // The same guard as memory_add, before the write. Resolution is not
    // supported here: resolving a subject conflict only goes through
    // memory_add. `exclude: None` — this is a brand-new node, nothing to
    // exclude from the search.
    provenance::guard_subject(conn, prov.subject.as_deref(), false, None)?;

    let mut task_data = json!({
        "status": "backlog",
        "priority": priority,
        "acceptance_criteria": acceptance_criteria,
        "project": project,
        "started_at": null,
        "completed_at": null,
    });
    prov.write_into(&mut task_data);

    let label = format!("[{}] {}", project, title);
    let task = graph::add_node_full(
        conn,
        NodeType::Task,
        &label,
        description,
        "mcp",
        task_data,
        MemoryKind::Semantic,
        None,
    )?;

    // Link to project (auto-create if missing)
    let proj_node = match graph::find_project_by_label(conn, project) {
        Ok(Some(n)) => n,
        _ => graph::add_node(
            conn,
            NodeType::Project,
            project,
            None,
            "mcp-task",
            json!({"auto_created": true}),
        )?,
    };
    graph::add_edge(conn, task.id, proj_node.id, Relation::BelongsTo, 1.0)?;

    // Parent task (subtask_of). Резолв ограничен типом Task по той же причине,
    // что и в `task_update`: обе связи по контракту ручки соединяют задачи, и
    // нестрогий полнотекстовый фолбэк молча привязал бы задачу к решению или
    // проблеме с похожей меткой — связь, которую потом никто не заметит.
    if let Some(parent_id) = params.get("parent").and_then(|p| p.as_str()) {
        if let Ok(parent) = resolve_task_node(conn, parent_id) {
            graph::add_edge(conn, task.id, parent.id, Relation::SubtaskOf, 1.0)?;
        }
    }

    // Blocks edges
    if let Some(blocks) = params.get("blocks").and_then(|b| b.as_array()) {
        for blocked in blocks {
            if let Some(blocked_id) = blocked.as_str() {
                if let Ok(blocked_node) = resolve_task_node(conn, blocked_id) {
                    graph::add_edge(conn, task.id, blocked_node.id, Relation::Blocks, 1.0)?;
                }
            }
        }
    }

    Ok(json!({
        "id": task.id.to_string(),
        "label": task.label,
        "type": "task",
        "status": "backlog",
        "priority": priority,
        "project": project,
        "created": true,
        "created_by": task.created_by,
        "provenance": {
            "confidence": prov.confidence_or_default().as_str(),
            "subject": prov.subject,
        },
    }))
}

pub fn task_update(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    task_update_with_conn(&conn, params)
}

/// Тело `task_update`, принимающее соединение явным параметром — не через
/// глобальный `db_path()`, как `open_db()` — специально ради тестируемости
/// (T0xx, спека 007): тест заводит свою временную БД и вызывает эту функцию
/// напрямую, не трогая настоящую базу пользователя.
fn task_update_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

    let node = resolve_task_node(conn, id)?;

    // Provenance is parsed right after resolving the task, before any edits
    // to `data`: an error in it must not leave a half-applied update behind.
    // A task can gain measured confidence AFTER a measurement — that is
    // exactly the case this field exists for.
    let prov = Provenance::parse(params)?;
    // `exclude`: this call edits `node` itself, so a same-subject fact
    // already sitting on `node` is not a contradiction with itself — only a
    // same-subject fact on some OTHER node still needs a `resolution`.
    let node_id = node.id.to_string();
    provenance::guard_subject(conn, prov.subject.as_deref(), false, Some(node_id.as_str()))?;

    // Merge data fields
    let mut data = node.data.clone();
    let now = chrono::Utc::now();
    let mut evicted: Option<graph::EvictedTask> = None;

    if let Some(status) = params.get("status").and_then(|s| s.as_str()) {
        if status == "active" {
            // Находка 4 (адверсариальный разбор спеки 007): `activated_at`
            // обязан ставиться ОДИН РАЗ, на переход в active — читаем прежний
            // статус ДО того, как он ниже перезапишется на новый (`data` тут
            // ещё исходная копия узла). Вызов на уже активной задаче (заодно
            // с обновлением `priority`, например) не имеет права сдвигать
            // `activated_at`: иначе `since` в `build_resolution` уезжает
            // вперёд и правки, сделанные до этого вызова, выпадают из
            // `resolution.files`, а с починкой находки 3 такой сдвиг ещё и
            // обнулял бы цикл созревания.
            let was_active = data.get("status").and_then(|s| s.as_str()) == Some("active");

            // Легаси-поле: читатели до этой фичи (`task_stats`) ждут именно
            // его, и только на первую активацию — как в CLI (`au task
            // activate`).
            if data.get("started_at").and_then(|s| s.as_str()).is_none() {
                data["started_at"] = json!(now.to_rfc3339());
            }

            // T008/FR-031, симметрия с `au task activate`: в проекте не
            // более одной активной задачи — взятие этой вытесняет прежнюю
            // активную того же проекта в backlog. Общая функция ядра, не
            // вторая копия правила.
            let project = data
                .get("project")
                .and_then(|p| p.as_str())
                .unwrap_or("unknown")
                .to_owned();
            evicted = graph::evict_active(conn, &project, node.id)?;
            if let Some(obj) = data.as_object_mut() {
                obj.remove("blocked_by");
            }

            // FR-001/FR-021c, симметрия с CLI: пишем новое время взятия в
            // работу, не трогая `closed_at`/`resolution` — при переоткрытии
            // они остаются историей, а не стираются. Только на реальный
            // переход (см. `was_active` выше) — не на каждый вызов.
            if !was_active {
                let mut fields = aurelius_core::tasks::TaskFields::from_data(&data);
                fields.activated_at = Some(now);
                data = fields.merge_into(&data);
            }
        }
        if status == "done" {
            // Легаси-поле: `task_stats` считает по нему длительность.
            data["completed_at"] = json!(now.to_rfc3339());

            // T021a, симметрия с `au task done`: коммит определяется сам из
            // состояния репозитория, файлы — из привязанных правок;
            // commit/pull_request/unconfirmed лишь уточняют автособранное.
            let mut fields = aurelius_core::tasks::TaskFields::from_data(&data);
            let project = data.get("project").and_then(|p| p.as_str());
            let commit = params
                .get("commit")
                .and_then(|c| c.as_str())
                .map(str::to_owned);
            let pull_request = params
                .get("pull_request")
                .and_then(|p| p.as_str())
                .map(str::to_owned);
            let unconfirmed = params
                .get("unconfirmed")
                .and_then(|u| u.as_bool())
                .unwrap_or(false);
            // Находка 1, FR-004/FR-006: коммит закрываемой задачи ищется в
            // каталоге ЕЁ ПРОЕКТА (см. `build_resolution`), не в CWD
            // процесса — процесс тут один на все проекты сразу.
            // `fields.activated_at` передаётся как есть, без подмены
            // `node.created_at` (resolution-window finding, measured
            // 2026-09-05): задача из backlog, ни разу не взятая в работу, не
            // имеет окна работы вовсе, см. доккомент `build_resolution`.
            let resolution = aurelius_core::tasks::build_resolution(
                conn,
                fields.activated_at,
                project,
                commit,
                pull_request,
                unconfirmed,
            );
            let confirmed = resolution.confirmed;
            fields.closed_at = Some(now);
            fields.resolution = Some(resolution);
            data = fields.merge_into(&data);
            if !confirmed {
                tracing::warn!(
                    task = %node.id,
                    "closed without confirmation — resolution unknown (FR-005)"
                );
            }
        }
        data["status"] = json!(status);
    }

    if let Some(blocked_by) = params.get("blocked_by").and_then(|b| b.as_str()) {
        data["status"] = json!("blocked");
        data["blocked_by"] = json!(blocked_by);
    }

    if let Some(priority) = params.get("priority").and_then(|p| p.as_str()) {
        data["priority"] = json!(priority);
    }

    if let Some(criteria) = params.get("acceptance_criteria") {
        data["acceptance_criteria"] = criteria.clone();
    }

    // Provenance — alongside the other edits, not a separate call: a task's
    // confidence can change AFTER a measurement, and this is exactly that
    // case. Without any fields given, `write_into` leaves `data` untouched.
    prov.write_into(&mut data);

    let new_note = params.get("note").and_then(|n| n.as_str());

    graph::update_node(conn, node.id, new_note, Some(data.clone()))?;

    let fields = aurelius_core::tasks::TaskFields::from_data(&data);
    // What actually sits on the task NOW, not just what this call brought —
    // a call without provenance fields must show what was already recorded
    // earlier, not pretend the task went back to unverified.
    let current_prov = Provenance::from_data(&data);

    let mut result = json!({
        "id": node.id.to_string(),
        "label": node.label,
        "status": data["status"],
        "priority": data["priority"],
        "updated": true,
        "updated_by": aurelius_core::identity::current().map(|i| i.as_author()),
        "activated_at": fields.activated_at.map(|d| d.to_rfc3339()),
        "closed_at": fields.closed_at.map(|d| d.to_rfc3339()),
        "resolution": fields.resolution,
        "provenance": {
            "confidence": current_prov.confidence_or_default().as_str(),
            "subject": current_prov.subject,
        },
    });
    // T009, симметрия с CLI: молчаливое вытеснение выглядит как потеря
    // задачи — сказать вслух, кого вытеснили.
    if let Some(ev) = &evicted {
        result["evicted"] = json!({"id": ev.id.to_string(), "label": ev.label});
    }
    Ok(result)
}

/// Бюджет `note` одной задачи в `task_list`, в символах, по границе слова.
///
/// Читателю списка (в отличие от `task_view` одной задачи) нужна строка-другая
/// на ориентировку, а не текст целиком — полный текст всё равно лежит в
/// узле и достаётся через `task_view`. Обрезка честная: `graph::clip`
/// помечает урезанное окончанием «…», и ответ отдельно называет бюджет и то,
/// где смотреть текст без урезания — то же требование, что и у `task_view`
/// (`TASK_VIEW_NOTE_BUDGET`), тем же способом.
const TASK_LIST_NOTE_BUDGET: usize = 200;

pub fn task_list(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    task_list_with_conn(&conn, params)
}

/// Тело `task_list` с явным соединением — тот же приём тестируемости, что и
/// у `task_update_with_conn`/`task_view_with_conn` выше.
fn task_list_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let project = params.get("project").and_then(|p| p.as_str());
    let status = params.get("status").and_then(|s| s.as_str());
    let priority = params.get("priority").and_then(|p| p.as_str());
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
    // Опция для того самого маленького аудитора, которому 17 вызовов
    // `task_view` подряд не по карману: заметки целиком прямо в списке,
    // без похода за каждой отдельно. По умолчанию — прежнее поведение.
    let full_notes = params
        .get("full_notes")
        .and_then(|f| f.as_bool())
        .unwrap_or(false);

    let tasks = graph::get_tasks_filtered(conn, project, status, priority, limit)?;

    let task_type = serde_json::to_string(&NodeType::WorkLog)?;

    let items: Vec<serde_json::Value> = tasks
        .iter()
        .map(|t| {
            // Count work logs linked to this task
            let log_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM edges e JOIN nodes n ON n.id = e.to_id
                     WHERE e.from_id = ?1 AND e.relation = 'contains' AND n.node_type = ?2",
                    rusqlite::params![t.id.to_string(), &task_type],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            // Спека 007, T026: аддитивные поля, ничего существующего не
            // переименовано (принцип VI). `ripe` — производное, не хранимое.
            let status = t
                .data
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("backlog");
            let fields = aurelius_core::tasks::TaskFields::from_data(&t.data);
            let ripe = aurelius_core::tasks::is_ripe(&fields, status);

            // full_notes=true — note целиком, без урезания и без пометки
            // усечения; иначе поведение то же, что было всегда.
            let note = if full_notes {
                t.note.clone()
            } else {
                t.note
                    .as_deref()
                    .map(|n| graph::clip(n, TASK_LIST_NOTE_BUDGET))
            };
            let note_truncated = !full_notes && note.as_deref().is_some_and(|n| n.ends_with('…'));

            // Сводка, не журнал прогонов целиком: полный массив с командами,
            // временами и путями к артефактам остаётся только у `task_view`
            // (см. `evidence_summary` в aurelius-core).
            let evidence = aurelius_core::tasks::evidence_summary(&fields);

            json!({
                "id": t.id.to_string(),
                "label": t.label,
                "status": status,
                "priority": t.data.get("priority").and_then(|p| p.as_str()).unwrap_or("medium"),
                "work_logs": log_count,
                "created_at": t.created_at.to_rfc3339(),
                "note": note,
                "note_truncated": note_truncated,
                "created_by": t.created_by,
                "updated_by": t.updated_by,
                "activated_at": fields.activated_at.map(|d| d.to_rfc3339()),
                "closed_at": fields.closed_at.map(|d| d.to_rfc3339()),
                "resolution": fields.resolution,
                "evidence": evidence,
                "ripe": ripe,
            })
        })
        .collect();

    Ok(json!({
        "tasks": items,
        "total": items.len(),
        "filters": {
            "project": project,
            "status": status,
            "priority": priority,
        },
        // Честный отчёт об урезании note — тот же принцип, что и в task_view:
        // молчаливая обрезка неотличима от короткого текста, значит нужно
        // сказать вслух бюджет и куда идти за полным текстом.
        "note_char_budget": TASK_LIST_NOTE_BUDGET,
        "how_to_see_full_note": "task_view с id этой задачи возвращает note целиком, без урезания; либо этот же вызов с full_notes=true отдаёт note целиком сразу для всех задач списка",
    }))
}

/// Созревшие задачи проекта — тот же выбор, что и `au task ripe`, доступный
/// ассистенту через MCP.
///
/// Спека 007 предъявляет созревшие задачи текстом, через `au judge --hook`:
/// работает, только когда задачу закрывает человек в терминале. Ассистент
/// закрывает задачи через `task_update` и текстовый хук не видит — до этой
/// ручки узнать «что созрело» через MCP было нечем, хотя вычисление уже было
/// (`is_ripe`/`ripe_evidence` в `task_list`/`task_view`). Здесь — та же
/// функция ядра, что и у CLI (`aurelius_core::tasks::gather_ripe`), не вторая
/// копия правила.
pub fn task_ripe(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    task_ripe_with_conn(&conn, params)
}

fn task_ripe_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let project = params.get("project").and_then(|p| p.as_str());
    let ripe = aurelius_core::tasks::gather_ripe(conn, project)?;
    Ok(json!({
        "ripe": aurelius_core::tasks::ripe_to_json(&ripe),
        "total": ripe.len(),
        "project": project,
    }))
}

pub fn task_log(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    task_log_with_conn(&conn, params)
}

/// The body of `task_log`, taking the connection as an explicit parameter —
/// the same testability trick as `task_update_with_conn`.
fn task_log_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let task_id = params
        .get("task")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'task' parameter"))?;
    let text = params
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'text' parameter"))?;

    let task = resolve_task_node(conn, task_id)?;

    // Provenance is parsed FIRST THING after resolving the task, before any
    // write: an error in it must not leave a half-written work_log behind.
    let prov = Provenance::parse(params)?;
    // `exclude: None` — a work_log is always a new node, nothing to exclude.
    provenance::guard_subject(conn, prov.subject.as_deref(), false, None)?;

    // Extract project from task data
    let project = task
        .data
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");

    // Recording a log line is an observation, not a decision to take the
    // task into work — status is read here only to report it back, never
    // written. Activation is explicit, via `task_update status=active`.
    let status = task
        .data
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("backlog")
        .to_owned();

    // Create WorkLog node
    let mut log_data = json!({"task_id": task.id.to_string()});
    prov.write_into(&mut log_data);
    let log_node = aurelius_core::tasks::log_work(conn, &task, text, "mcp-task", log_data)?;

    let mut created_nodes = vec![node_compact(&log_node)];

    // Nodes spawned alongside the call (decisions/problems/solutions) inherit
    // what backs the record, but not subject/claim — both belong to exactly
    // one fact, see `Provenance::inherited`.
    let inherited = prov.inherited();

    // Create decision nodes
    if let Some(decisions) = params.get("decisions").and_then(|d| d.as_array()) {
        for decision in decisions {
            if let Some(dec_text) = decision.as_str() {
                let mut dec_data = json!({"task_id": task.id.to_string()});
                inherited.write_into(&mut dec_data);
                let dec_node = graph::add_node(
                    conn,
                    NodeType::Decision,
                    &format!("[{}] {}", project, truncate(dec_text, 60)),
                    Some(dec_text),
                    "mcp-task",
                    dec_data,
                )?;
                graph::add_edge(conn, task.id, dec_node.id, Relation::Contains, 1.0)?;
                graph::add_edge(conn, log_node.id, dec_node.id, Relation::Contains, 1.0)?;
                if let Ok(Some(proj_node)) = graph::find_project_by_label(conn, project) {
                    graph::add_edge(conn, dec_node.id, proj_node.id, Relation::BelongsTo, 1.0)?;
                }
                created_nodes.push(node_compact(&dec_node));
            }
        }
    }

    // Create problem+solution pairs
    if let Some(problems) = params.get("problems_solved").and_then(|p| p.as_array()) {
        for problem in problems {
            let prob_text = problem.get("problem").and_then(|p| p.as_str());
            let sol_text = problem.get("solution").and_then(|s| s.as_str());
            if let (Some(prob), Some(sol)) = (prob_text, sol_text) {
                let mut prob_data = json!({"task_id": task.id.to_string()});
                inherited.write_into(&mut prob_data);
                let prob_node = graph::add_node(
                    conn,
                    NodeType::Problem,
                    &format!("[{}] {}", project, truncate(prob, 60)),
                    Some(prob),
                    "mcp-task",
                    prob_data,
                )?;
                let mut sol_data = json!({"task_id": task.id.to_string()});
                inherited.write_into(&mut sol_data);
                let sol_node = graph::add_node(
                    conn,
                    NodeType::Solution,
                    &format!("[{}] {}", project, truncate(sol, 60)),
                    Some(sol),
                    "mcp-task",
                    sol_data,
                )?;
                graph::add_edge(conn, sol_node.id, prob_node.id, Relation::Solves, 1.0)?;
                graph::add_edge(conn, task.id, prob_node.id, Relation::Contains, 1.0)?;
                graph::add_edge(conn, task.id, sol_node.id, Relation::Contains, 1.0)?;
                graph::add_edge(conn, log_node.id, prob_node.id, Relation::Contains, 1.0)?;
                if let Ok(Some(proj_node)) = graph::find_project_by_label(conn, project) {
                    graph::add_edge(conn, prob_node.id, proj_node.id, Relation::BelongsTo, 1.0)?;
                    graph::add_edge(conn, sol_node.id, proj_node.id, Relation::BelongsTo, 1.0)?;
                }
                created_nodes.push(node_compact(&prob_node));
                created_nodes.push(node_compact(&sol_node));
            }
        }
    }

    let mut response = json!({
        "task_id": task.id.to_string(),
        "task_label": task.label,
        "created_nodes": created_nodes,
        "total_created": created_nodes.len(),
        "task_status": status,
        "provenance": {
            "confidence": prov.confidence_or_default().as_str(),
            "subject": prov.subject,
        },
    });
    if status == "backlog" {
        response["hint"] = json!(
            "This task was not activated: logging never changes status. \
             Use task_update status=active or `au task activate` to take it into work."
        );
    }

    Ok(response)
}

pub fn task_stats(params: &serde_json::Value) -> Result<serde_json::Value> {
    let project = params.get("project").and_then(|p| p.as_str());
    let since_days = params.get("since_days").and_then(|d| d.as_u64());

    let conn = open_db()?;
    let tasks = graph::get_tasks_filtered(&conn, project, None, None, 100_000)?;

    let mut by_status: std::collections::BTreeMap<String, usize> = Default::default();
    let mut by_priority: std::collections::BTreeMap<String, usize> = Default::default();
    let mut completion_hours: Vec<f64> = Vec::new();
    let mut currently_blocked = 0usize;
    let mut oldest_active: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut done_in_window = 0usize;

    let now = chrono::Utc::now();
    let window_cutoff = since_days.map(|d| now - chrono::Duration::days(d as i64));

    for t in &tasks {
        let status = t
            .data
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("backlog");
        let priority = t
            .data
            .get("priority")
            .and_then(|p| p.as_str())
            .unwrap_or("medium");
        *by_status.entry(status.to_string()).or_insert(0) += 1;
        *by_priority.entry(priority.to_string()).or_insert(0) += 1;

        if status == "blocked" {
            currently_blocked += 1;
        }

        let started = t
            .data
            .get("started_at")
            .and_then(|s| s.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        let completed = t
            .data
            .get("completed_at")
            .and_then(|s| s.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        if let (Some(s), Some(c)) = (started, completed) {
            let hours = (c - s).num_seconds() as f64 / 3600.0;
            if hours >= 0.0 {
                completion_hours.push(hours);
            }
            if let Some(cutoff) = window_cutoff {
                if c >= cutoff {
                    done_in_window += 1;
                }
            } else if status == "done" {
                done_in_window += 1;
            }
        }

        if status == "active" {
            if let Some(s) = started {
                oldest_active = Some(oldest_active.map_or(s, |cur| cur.min(s)));
            }
        }
    }

    completion_hours.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let avg = if completion_hours.is_empty() {
        None
    } else {
        Some(completion_hours.iter().sum::<f64>() / completion_hours.len() as f64)
    };
    let median = if completion_hours.is_empty() {
        None
    } else {
        let mid = completion_hours.len() / 2;
        Some(if completion_hours.len().is_multiple_of(2) {
            (completion_hours[mid - 1] + completion_hours[mid]) / 2.0
        } else {
            completion_hours[mid]
        })
    };

    let total = tasks.len();
    let done_count = by_status.get("done").copied().unwrap_or(0);
    let cancelled_count = by_status.get("cancelled").copied().unwrap_or(0);
    let closed = done_count + cancelled_count;
    let completion_rate = if total > 0 {
        Some(done_count as f64 / total as f64)
    } else {
        None
    };

    let oldest_active_days = oldest_active.map(|s| (now - s).num_hours() as f64 / 24.0);

    Ok(json!({
        "project": project,
        "window_days": since_days,
        "total": total,
        "closed": closed,
        "by_status": by_status,
        "by_priority": by_priority,
        "completion_rate": completion_rate,
        "avg_active_to_done_hours": avg,
        "median_active_to_done_hours": median,
        "currently_blocked": currently_blocked,
        "oldest_active_age_days": oldest_active_days,
        "done_in_window": done_in_window,
    }))
}

/// Сколько узлов каждого типа показывать по умолчанию, если вызывающий не
/// попросил `full: true` и не назвал свой `limit`.
///
/// Измерено на живой задаче (f69a9e4f, проект aurelius): ручка отдала 107 473
/// символа, из них 121 вложенный узел — 66 decisions, 28 problems, 16
/// solutions, 11 «subtasks» — притом ни один не был датирован позже самой
/// задачи. Причина оказалась не в объёме на узел, а в BFS глубины 2: с шага
/// task→project BFS шёл ЕЩЁ на шаг дальше и подбирал вообще все узлы всего
/// проекта — чужие задачи и их decisions/problems/solutions, выданные как
/// будто relations этой задачи. Это не только раздувало ответ, но и было
/// неверно по существу. Исправлено ниже (глубина 1); этот предел — вторая,
/// независимая линия защиты для задач, у которых своя ветка (task_log на неё
/// саму) действительно велика.
const TASK_VIEW_DEFAULT_ITEM_CAP: usize = 5;

/// Бюджет `note` одного вложенного узла в символах (по границе слова).
const TASK_VIEW_NOTE_BUDGET: usize = 300;

pub fn task_view(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    task_view_with_conn(&conn, params)
}

/// Тело `task_view` с явным соединением — тот же приём тестируемости, что и у
/// `task_update_with_conn` выше.
fn task_view_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;
    let full = params
        .get("full")
        .and_then(|f| f.as_bool())
        .unwrap_or(false);
    let item_cap = params
        .get("limit")
        .and_then(|l| l.as_u64())
        .map(|l| l as usize)
        .unwrap_or(TASK_VIEW_DEFAULT_ITEM_CAP);

    let task = resolve_task_node(conn, id)?;
    // Best effort by design: an access counter must never fail a read.
    if let Err(e) = graph::touch_node(conn, task.id) {
        tracing::warn!("could not record access for {}: {e}", task.id);
    }

    // Глубина 1, не 2: ровно один шаг от задачи достаёт всё, что ей ДЕЙСТВИТЕЛЬНО
    // принадлежит — work_log/decision/problem/solution через `contains` (их
    // ставит task_log и от задачи, и от work_log сразу — второй хоп не добавляет
    // ничего нового) и дочерние подзадачи через `subtask_of` (ребро child→parent
    // видно от parent на первом же шаге). Второй хоп нужен был бы только чтобы
    // пройти ЧЕРЕЗ узел проекта (task --belongs_to--> project) и вернуться на
    // все остальные задачи и их ветки того же проекта — то есть ровно та утечка,
    // что и раздувала ответ (см. TASK_VIEW_DEFAULT_ITEM_CAP).
    let (nodes, edges) = graph::context_from_id(conn, &task.id.to_string(), 1)?;

    let mut work_logs: Vec<&aurelius_core::models::Node> = vec![];
    let mut decisions: Vec<&aurelius_core::models::Node> = vec![];
    let mut problems: Vec<&aurelius_core::models::Node> = vec![];
    let mut solutions: Vec<&aurelius_core::models::Node> = vec![];
    let mut subtasks: Vec<&aurelius_core::models::Node> = vec![];

    for node in &nodes {
        if node.id == task.id {
            continue;
        }
        match &node.node_type {
            NodeType::WorkLog => work_logs.push(node),
            NodeType::Decision => decisions.push(node),
            NodeType::Problem => problems.push(node),
            NodeType::Solution => solutions.push(node),
            NodeType::Task => subtasks.push(node),
            _ => {}
        }
    }

    let (timeline, work_logs_hidden) = branch_json(work_logs, full, item_cap, true);
    let (decisions, decisions_hidden) = branch_json(decisions, full, item_cap, false);
    let (problems, problems_hidden) = branch_json(problems, full, item_cap, false);
    let (solutions, solutions_hidden) = branch_json(solutions, full, item_cap, false);
    let (subtasks, subtasks_hidden) = branch_json(subtasks, full, item_cap, false);

    // Спека 007, T026: аддитивные поля из типизированных полей задачи
    // (`crates/aurelius-core/src/tasks.rs`) — ничего существующего не
    // переименовано, новых обязательных параметров нет (принцип VI).
    let status_str = task
        .data
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("backlog");
    let fields = aurelius_core::tasks::TaskFields::from_data(&task.data);
    let ripe = aurelius_core::tasks::is_ripe(&fields, status_str);

    Ok(json!({
        // Сама задача НИКОГДА не режется: все поля целиком, как и раньше.
        "task": node_detail(&task),
        "status": task.data.get("status"),
        "priority": task.data.get("priority"),
        "acceptance_criteria": task.data.get("acceptance_criteria"),
        "timeline": timeline,
        "decisions": decisions,
        "problems": problems,
        "solutions": solutions,
        "subtasks": subtasks,
        "total_edges": edges.len(),
        "activated_at": fields.activated_at.map(|d| d.to_rfc3339()),
        "closed_at": fields.closed_at.map(|d| d.to_rfc3339()),
        "resolution": fields.resolution,
        "evidence": fields.evidence,
        "ripe": ripe,
        // Честный отчёт об урезании — молчаливая обрезка хуже длинного ответа:
        // читатель обязан узнать, что именно и сколько осталось за кадром, и
        // как это достать, а не догадываться по круглым числам вроде "5".
        "truncation": {
            "applied": !full && (work_logs_hidden + decisions_hidden + problems_hidden
                + solutions_hidden + subtasks_hidden > 0),
            "item_limit_per_type": if full { serde_json::Value::Null } else { json!(item_cap) },
            "note_char_budget": if full { serde_json::Value::Null } else { json!(TASK_VIEW_NOTE_BUDGET) },
            "hidden": {
                "timeline": work_logs_hidden,
                "decisions": decisions_hidden,
                "problems": problems_hidden,
                "solutions": solutions_hidden,
                "subtasks": subtasks_hidden,
            },
            "how_to_see_more": "task_view с full=true (без урезания вовсе) или с limit=N (свой предел на категорию)",
        },
    }))
}

/// Один вложенный узел (decision/problem/solution/work_log/subtask) в форме
/// для выдачи task_view. Не переиспользует `node_compact`: тот тащит
/// `provenance_brief` безусловно, а у узлов, которые заводит `task_log`, это
/// поле НИКОГДА не заполнено (task_log не пишет claim/evidence/measured_at/
/// subject) — пять `null` на узел без единого сигнала. Здесь блок печатается,
/// только когда в нём есть хоть что-то не тождественное умолчанию.
fn nested_node_json(node: &aurelius_core::models::Node, full: bool) -> serde_json::Value {
    let claim = aurelius_core::provenance::Provenance::from_data(&node.data).claim;
    let note = node.note.as_deref().map(|n| {
        if full {
            n.to_owned()
        } else {
            aurelius_core::graph::clip(n, TASK_VIEW_NOTE_BUDGET)
        }
    });
    let mut v = json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "claim": claim,
        "note": note,
        "created_at": node.created_at.to_rfc3339(),
    });
    if let Some(p) = provenance_if_present(node) {
        v["provenance"] = p;
    }
    v
}

/// `None`, когда провенанс узла — сплошной умолчательный `unverified`/`null`
/// (обычный случай для decision/problem/solution/work_log, заведённых через
/// `task_log`); `Some` — когда в нём есть хоть один реально записанный факт
/// (например, кто-то потом дописал `evidence` через `memory_update`).
fn provenance_if_present(node: &aurelius_core::models::Node) -> Option<serde_json::Value> {
    let p = aurelius_core::provenance::Provenance::from_data(&node.data);
    let has_signal = p.confidence.is_some()
        || p.evidence.is_some()
        || p.measured_at.is_some()
        || p.subject.is_some()
        || p.volatility.is_some();
    if !has_signal {
        return None;
    }
    Some(json!({
        "confidence": p.confidence_or_default().as_str(),
        "evidence": p.evidence,
        "measured_at": p.measured_at.map(|d| d.to_rfc3339()),
        "subject": p.subject,
        "stale": p.staleness(node.created_at, chrono::Utc::now()).map(|s| s.note()),
    }))
}

/// Отобрать, урезать и сериализовать одну категорию вложенных узлов.
///
/// Без `full` берутся самые СВЕЖИЕ (по `created_at`) `item_cap` штук — старое
/// в ветке задачи типично менее нужно, чем недавнее. `ascending`, когда
/// выдача обязана остаться хронологией (`timeline`): после отбора самых
/// свежих порядок в ответе всё равно от старого к новому, как и раньше.
/// Возвращает JSON-массив и число узлов, оставшихся за кадром.
fn branch_json(
    mut nodes: Vec<&aurelius_core::models::Node>,
    full: bool,
    item_cap: usize,
    ascending: bool,
) -> (Vec<serde_json::Value>, usize) {
    let hidden = if full {
        0
    } else {
        nodes.sort_by_key(|n| std::cmp::Reverse(n.created_at));
        let hidden = nodes.len().saturating_sub(item_cap);
        nodes.truncate(item_cap);
        hidden
    };
    if ascending {
        nodes.sort_by_key(|n| n.created_at);
    }
    let items = nodes.iter().map(|n| nested_node_json(n, full)).collect();
    (items, hidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelius_core::db;

    /// Тот же приём, что в `graph::lease` тестах: настоящий temp-файл, не
    /// `:memory:` — `db::open` жёстко требует WAL. Собственный файл на тест,
    /// а не `open_db()` — `open_db()` бьёт в настоящую БД пользователя по
    /// `db_path()`, что для теста непригодно.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-mcp-task-test-{tag}-{}.db",
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

    fn seed_task(conn: &rusqlite::Connection, project: &str, label: &str) -> uuid::Uuid {
        graph::add_node_full(
            conn,
            NodeType::Task,
            label,
            None,
            "test",
            json!({"status": "backlog", "priority": "medium", "project": project}),
            MemoryKind::Semantic,
            None,
        )
        .expect("insert task")
        .id
    }

    // -- task 2c8d25ce: provenance fields in task_create/task_log/task_update --

    /// Exactly what was missing: an agent that just ran a command must be
    /// able to record that through `task_log`, the same as through
    /// `memory_add`.
    #[test]
    fn task_log_with_measured_and_evidence_writes_provenance_on_worklog_and_inherits_on_decision() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(&conn, "proj-prov", "задача под измерение");

        let result = task_log_with_conn(
            &conn,
            &json!({
                "task": task_id.to_string(),
                "text": "прогнали cargo test",
                "confidence": "measured",
                "evidence": "cargo test --workspace",
                "subject": "aurelius:proj-prov:worklog",
                "decisions": ["решили не трогать схему"],
            }),
        )
        .expect("task_log");

        assert_eq!(result["provenance"]["confidence"], "measured");

        let created = result["created_nodes"].as_array().expect("created_nodes");
        let log_id = created[0]["id"].as_str().expect("worklog id");
        let log_node = graph::get_node(&conn, log_id)
            .expect("get_node")
            .expect("worklog exists");
        let log_prov = aurelius_core::provenance::Provenance::from_data(&log_node.data);
        assert_eq!(
            log_prov.confidence,
            Some(aurelius_core::provenance::Confidence::Measured)
        );
        assert_eq!(log_prov.evidence.as_deref(), Some("cargo test --workspace"));
        assert_eq!(
            log_prov.subject.as_deref(),
            Some("aurelius:proj-prov:worklog")
        );

        // The spawned decision inherits confidence/evidence, but NOT subject:
        // the subject key is unique to one fact.
        let dec_id = created[1]["id"].as_str().expect("decision id");
        let dec_node = graph::get_node(&conn, dec_id)
            .expect("get_node")
            .expect("decision exists");
        let dec_prov = aurelius_core::provenance::Provenance::from_data(&dec_node.data);
        assert_eq!(
            dec_prov.confidence,
            Some(aurelius_core::provenance::Confidence::Measured)
        );
        assert_eq!(dec_prov.evidence.as_deref(), Some("cargo test --workspace"));
        assert_eq!(
            dec_prov.subject, None,
            "subject не обязан копироваться на попутный узел: {dec_prov:?}"
        );
    }

    /// confidence=measured without evidence — the same refusal as
    /// `memory_add`, and NOTHING is created: no work_log, task untouched.
    #[test]
    fn task_log_measured_without_evidence_is_refused_and_creates_nothing() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(&conn, "proj-refused", "задача без улики");
        let before = graph::get_all_nodes(&conn).expect("nodes before").len();

        let err = task_log_with_conn(
            &conn,
            &json!({
                "task": task_id.to_string(),
                "text": "прогнали что-то",
                "confidence": "measured",
            }),
        )
        .expect_err("measured без evidence обязано быть отказом");
        assert!(format!("{err}").contains("inferred"), "{err}");

        let after = graph::get_all_nodes(&conn).expect("nodes after").len();
        assert_eq!(before, after, "отказ обязан не создавать ни одного узла");
    }

    /// Asymmetry: without provenance fields the behavior is unchanged —
    /// work_log reads as unverified, same as before task 2c8d25ce.
    #[test]
    fn task_log_without_provenance_fields_behaves_as_before() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(&conn, "proj-plain", "задача без происхождения");

        let result = task_log_with_conn(
            &conn,
            &json!({ "task": task_id.to_string(), "text": "работа без provenance" }),
        )
        .expect("task_log");

        assert_eq!(result["provenance"]["confidence"], "unverified");
        let log_id = result["created_nodes"][0]["id"]
            .as_str()
            .expect("worklog id");
        let log_node = graph::get_node(&conn, log_id)
            .expect("get_node")
            .expect("worklog exists");
        let log_prov = aurelius_core::provenance::Provenance::from_data(&log_node.data);
        assert_eq!(log_prov.confidence, None);
    }

    /// `task_create` with a subject writes it onto the task node — the same
    /// parse as `memory_add`.
    #[test]
    fn task_create_with_subject_writes_subject_on_the_task_node() {
        let (_tmp, conn) = setup();

        let result = task_create_with_conn(
            &conn,
            &json!({
                "title": "задача с subject",
                "project": "proj-create-subject",
                "subject": "aurelius:proj-create-subject:task",
            }),
        )
        .expect("task_create");

        assert_eq!(
            result["provenance"]["subject"],
            "aurelius:proj-create-subject:task"
        );

        let id = result["id"].as_str().expect("id");
        let node = graph::get_node(&conn, id)
            .expect("get_node")
            .expect("task exists");
        let prov = aurelius_core::provenance::Provenance::from_data(&node.data);
        assert_eq!(
            prov.subject.as_deref(),
            Some("aurelius:proj-create-subject:task")
        );
    }

    /// Асимметрия задачи 007: до правки MCP `task_update` писал только
    /// легаси `completed_at` при переходе в `done` — `closed_at` и
    /// `resolution` оставались `null`, хотя CLI (`au task done`) их
    /// заполняет. Этот тест падал на прежней реализации и проходит на новой.
    #[test]
    fn task_update_done_sets_closed_at_and_resolution() {
        let (_tmp, conn) = setup();
        let id = seed_task(&conn, "proj-a", "закрываемая задача");

        let result = task_update_with_conn(&conn, &json!({"id": id.to_string(), "status": "done"}))
            .expect("task_update done");

        assert_eq!(result["status"], "done");
        assert!(
            result["closed_at"].is_string(),
            "closed_at обязан быть выставлен при закрытии через MCP, получили: {:?}",
            result["closed_at"]
        );
        assert!(
            !result["resolution"].is_null(),
            "resolution обязан быть собран при закрытии через MCP"
        );

        // Прочитаем узел напрямую — проверка не только ответа ручки, но и
        // того, что реально легло в data.
        let node = graph::get_node(&conn, &id.to_string())
            .expect("get_node")
            .expect("node exists");
        let fields = aurelius_core::tasks::TaskFields::from_data(&node.data);
        assert!(fields.closed_at.is_some());
        assert!(fields.resolution.is_some());
        // Легаси-поле для старых читателей (`task_stats`) остаётся на месте.
        assert!(node
            .data
            .get("completed_at")
            .and_then(|v| v.as_str())
            .is_some());
    }

    /// Асимметрия задачи 007: до правки MCP `task_update` при переходе в
    /// `active` писал только легаси `started_at` (и то один раз) и не знал
    /// правила «одна активная задача на проект» — CLI (`au task activate`)
    /// вытесняет прежнюю активную в backlog и ставит `activated_at` на
    /// каждое взятие. Тест падал на прежней реализации.
    #[test]
    fn task_update_active_sets_activated_at_and_evicts_previous_active() {
        let (_tmp, conn) = setup();
        let old_active = seed_task(&conn, "proj-b", "старая активная");
        // Активируем первую задачу напрямую в data, как будто она уже была
        // взята в работу раньше.
        {
            let node = graph::get_node(&conn, &old_active.to_string())
                .expect("get_node")
                .expect("node exists");
            let mut data = node.data.clone();
            data["status"] = json!("active");
            graph::update_node(&conn, old_active, None, Some(data)).expect("seed active");
        }
        let new_active = seed_task(&conn, "proj-b", "новая активная");

        let result = task_update_with_conn(
            &conn,
            &json!({"id": new_active.to_string(), "status": "active"}),
        )
        .expect("task_update active");

        assert!(
            result["activated_at"].is_string(),
            "activated_at обязан быть выставлен при взятии в работу через MCP, получили: {:?}",
            result["activated_at"]
        );

        let evicted_node = graph::get_node(&conn, &old_active.to_string())
            .expect("get_node")
            .expect("node exists");
        assert_eq!(
            evicted_node.data.get("status").and_then(|s| s.as_str()),
            Some("backlog"),
            "прежняя активная задача проекта обязана быть вытеснена в backlog"
        );
        assert_eq!(result["evicted"]["id"], old_active.to_string());
    }

    /// Находка 4 (адверсариальный разбор спеки 007): повторный вызов со
    /// `status=="active"` на УЖЕ активной задаче (например, попутно с
    /// обновлением `priority`) не имеет права сдвигать `activated_at`
    /// вперёд — иначе `since` в `build_resolution` при закрытии исключит
    /// правки, сделанные до этого вызова, а с починкой находки 3 такой сдвиг
    /// ещё и обнулял бы цикл созревания. Тест падал на прежней реализации
    /// (`activated_at` ставился на `now` при ЛЮБОМ `status=="active"`).
    #[test]
    fn task_update_active_on_already_active_task_keeps_original_activated_at() {
        let (_tmp, conn) = setup();
        let id = seed_task(&conn, "proj-d", "уже активная задача");
        let original_activated_at = "2026-08-30T08:00:00Z";
        {
            let node = graph::get_node(&conn, &id.to_string())
                .expect("get_node")
                .expect("node exists");
            let mut data = node.data.clone();
            data["status"] = json!("active");
            data["activated_at"] = json!(original_activated_at);
            graph::update_node(&conn, id, None, Some(data)).expect("seed active");
        }

        let result = task_update_with_conn(
            &conn,
            &json!({"id": id.to_string(), "status": "active", "priority": "high"}),
        )
        .expect("task_update active again");

        let expected: chrono::DateTime<chrono::Utc> =
            original_activated_at.parse().expect("rfc3339");
        let got: chrono::DateTime<chrono::Utc> = result["activated_at"]
            .as_str()
            .expect("activated_at строкой")
            .parse()
            .expect("rfc3339");
        assert_eq!(
            got, expected,
            "повторный вызов на уже активной задаче не обязан сдвигать activated_at"
        );
    }

    /// Уточняющий `commit` попадает в resolution, а не заменяется
    /// автособранным (FR-006): CLI ведёт себя так же (`--commit` уточняет,
    /// а не единственный источник).
    #[test]
    fn task_update_done_commit_param_refines_resolution() {
        let (_tmp, conn) = setup();
        let id = seed_task(&conn, "proj-c", "задача с явным коммитом");

        let result = task_update_with_conn(
            &conn,
            &json!({"id": id.to_string(), "status": "done", "commit": "deadbeef"}),
        )
        .expect("task_update done with commit");

        assert_eq!(result["resolution"]["commit"], "deadbeef");
        assert_eq!(result["resolution"]["confirmed"], true);
    }

    /// Находка 7 (адверсариальный разбор спеки 007): фолбэк полнотекстового
    /// поиска в резолве задачи обязан быть ограничен типом Task, как в CLI
    /// (`find_task` в `crates/au/src/commands.rs`) — иначе нечёткое имя без
    /// единой существующей задачи молча находит и мутирует узел ДРУГОГО
    /// типа. Воспроизведение — как в разборе: в базе есть только
    /// Decision-узел "[proj-probe] migrate to postgres entirely" и ни одной
    /// задачи; `task_update({id: "migrate postgres", status: "done"})` не
    /// имеет права найти и закрыть этот Decision. Тест падал на прежней
    /// реализации (`resolve_node` без ограничения по типу находил Decision и
    /// молча правил его `data.status`).
    #[test]
    fn task_update_does_not_match_task_of_wrong_type_via_fuzzy_search() {
        let (_tmp, conn) = setup();
        let decision = graph::add_node(
            &conn,
            NodeType::Decision,
            "[proj-probe] migrate to postgres entirely",
            Some("migrate to postgres entirely"),
            "test",
            json!({}),
        )
        .expect("insert decision");

        let err =
            task_update_with_conn(&conn, &json!({"id": "migrate postgres", "status": "done"}))
                .expect_err("нечёткое имя без задач обязано вернуть ошибку, а не найти Decision");
        assert!(
            err.to_string().contains("task not found"),
            "ожидалось «task not found», получили: {err}"
        );

        // Decision обязан остаться нетронутым — ничего не мутировано.
        let node = graph::get_node(&conn, &decision.id.to_string())
            .expect("get_node")
            .expect("decision exists");
        assert!(
            node.data.get("status").is_none(),
            "Decision не обязан обзавестись полем status задачи: {:?}",
            node.data
        );
    }

    /// Находка 1, монтаж на стороне MCP (адверсариальный разбор спеки 007):
    /// `task_update` обязан передавать ПРОЕКТ закрываемой задачи в
    /// `build_resolution`, а не звать его без проекта вовсе — иначе
    /// автоподстановка коммита ушла бы в CWD процесса aurelius-сервера, а не
    /// в каталог проекта задачи. Задача из непроиндексированного проекта —
    /// каталог неизвестен графу — коммит обязан остаться пустым, а не
    /// подставленным из CWD процесса теста (гарантированно git-репозиторий
    /// aurelius: воспроизведение находки — «два настоящих git-репозитория,
    /// CWD в repo_a, resolution для задачи repo_b»). Тест падал на прежней
    /// реализации (`build_resolution` звалась без параметра `project`).
    #[test]
    fn task_update_done_does_not_guess_commit_from_process_cwd() {
        let (_tmp, conn) = setup();
        let id = seed_task(
            &conn,
            "proj-без-индексации",
            "задача из непроиндексированного проекта",
        );

        let result = task_update_with_conn(&conn, &json!({"id": id.to_string(), "status": "done"}))
            .expect("task_update done");

        assert!(
            result["resolution"]["commit"].is_null(),
            "пустой коммит честнее подставленного из CWD чужого процесса: {:?}",
            result["resolution"]["commit"]
        );
    }

    // -- task_view: объём ответа и честность урезания ----------------------

    fn ensure_project(conn: &rusqlite::Connection, project: &str) -> uuid::Uuid {
        if let Ok(Some(p)) = graph::find_project_by_label(conn, project) {
            return p.id;
        }
        graph::add_node(conn, NodeType::Project, project, None, "test", json!({}))
            .expect("project node")
            .id
    }

    /// Задача, реально привязанная к проекту ребром `belongs_to` — как это
    /// делает `task_create`. `seed_task` этого не делает, а именно ребро
    /// task→project и есть путь, которым раньше глубина-2 BFS утекала на
    /// весь проект (см. `TASK_VIEW_DEFAULT_ITEM_CAP`).
    fn seed_task_in_project(conn: &rusqlite::Connection, project: &str, label: &str) -> uuid::Uuid {
        let task_id = seed_task(conn, project, label);
        let proj_id = ensure_project(conn, project);
        graph::add_edge(conn, task_id, proj_id, Relation::BelongsTo, 1.0).expect("belongs_to");
        task_id
    }

    fn add_worklog(
        conn: &rusqlite::Connection,
        task_id: uuid::Uuid,
        project: &str,
        text: &str,
    ) -> uuid::Uuid {
        let log = graph::add_node_full(
            conn,
            NodeType::WorkLog,
            &format!("[{project}] {}", truncate(text, 60)),
            Some(text),
            "test",
            json!({"task_id": task_id.to_string()}),
            MemoryKind::Episodic,
            None,
        )
        .expect("worklog");
        graph::add_edge(conn, task_id, log.id, Relation::Contains, 1.0).expect("contains");
        log.id
    }

    fn add_decision(
        conn: &rusqlite::Connection,
        task_id: uuid::Uuid,
        project: &str,
        text: &str,
    ) -> uuid::Uuid {
        let dec = graph::add_node(
            conn,
            NodeType::Decision,
            &format!("[{project}] {}", truncate(text, 60)),
            Some(text),
            "test",
            json!({"task_id": task_id.to_string()}),
        )
        .expect("decision");
        graph::add_edge(conn, task_id, dec.id, Relation::Contains, 1.0).expect("contains");
        dec.id
    }

    /// Корневая причина измеренного раздутия (107 473 символа на живой
    /// задаче f69a9e4f): BFS глубины 2 от задачи проходил ЧЕРЕЗ узел проекта
    /// и на втором шаге подбирал decisions/problems/solutions ВООБЩЕ ВСЕХ
    /// задач проекта, выдавая их как relations конкретной задачи. Тест
    /// падал на прежней реализации (глубина 2) и проходит на новой (1).
    #[test]
    fn task_view_does_not_leak_sibling_tasks_via_shared_project() {
        let (_tmp, conn) = setup();
        let task_a = seed_task_in_project(&conn, "proj-x", "task A");
        let task_b =
            seed_task_in_project(&conn, "proj-x", "task B — сосед по проекту, не связан с A");
        add_decision(&conn, task_b, "proj-x", "решение, принадлежащее только B");

        let result =
            task_view_with_conn(&conn, &json!({"id": task_a.to_string()})).expect("task_view");

        let subtasks = result["subtasks"].as_array().expect("subtasks array");
        assert!(
            !subtasks
                .iter()
                .any(|s| s["id"] == json!(task_b.to_string())),
            "сосед по проекту не обязан выглядеть дочерней подзадачей A: {subtasks:?}"
        );
        let decisions = result["decisions"].as_array().expect("decisions array");
        assert!(
            decisions.is_empty(),
            "decision чужой задачи не обязана попадать в ветку A: {decisions:?}"
        );
    }

    /// Симметрия предыдущему тесту: НАСТОЯЩАЯ подзадача (ребро `subtask_of`
    /// от ребёнка к родителю) обязана остаться видна на глубине 1 — переход
    /// на глубину 1 не должен был обрезать реальные связи, только утечку
    /// через проект.
    #[test]
    fn task_view_still_shows_direct_subtask() {
        let (_tmp, conn) = setup();
        let parent = seed_task_in_project(&conn, "proj-y", "родитель");
        let child = seed_task_in_project(&conn, "proj-y", "дочерняя подзадача");
        graph::add_edge(&conn, child, parent, Relation::SubtaskOf, 1.0).expect("subtask_of");

        let result =
            task_view_with_conn(&conn, &json!({"id": parent.to_string()})).expect("task_view");

        let subtasks = result["subtasks"].as_array().expect("subtasks array");
        assert!(
            subtasks.iter().any(|s| s["id"] == json!(child.to_string())),
            "прямая подзадача обязана быть видна: {subtasks:?}"
        );
    }

    /// Поля самой задачи (в т.ч. acceptance_criteria) никогда не режутся;
    /// урезается только вложенная ветка, и урезание честно отчитывается —
    /// сколько узлов какого типа скрыто.
    #[test]
    fn task_view_caps_branch_and_reports_hidden_honestly() {
        let (_tmp, conn) = setup();
        let task_id = seed_task_in_project(&conn, "proj-z", "большая задача");
        {
            let node = graph::get_node(&conn, &task_id.to_string())
                .expect("get_node")
                .expect("exists");
            let mut data = node.data.clone();
            data["acceptance_criteria"] = json!([
                "критерий 1",
                "критерий 2",
                "критерий 3",
                "критерий 4",
                "критерий 5"
            ]);
            graph::update_node(&conn, task_id, None, Some(data)).expect("seed acceptance_criteria");
        }
        let long_note = "слово ".repeat(200); // ~1200 символов — заведомо больше бюджета
        for i in 0..8 {
            add_worklog(&conn, task_id, "proj-z", &format!("{long_note}запись{i}"));
        }
        for i in 0..8 {
            add_decision(&conn, task_id, "proj-z", &format!("{long_note}решение{i}"));
        }

        let result =
            task_view_with_conn(&conn, &json!({"id": task_id.to_string()})).expect("task_view");

        // Сама задача — не режется.
        assert_eq!(
            result["acceptance_criteria"].as_array().expect("ac").len(),
            5
        );

        let timeline = result["timeline"].as_array().expect("timeline");
        let decisions = result["decisions"].as_array().expect("decisions");
        assert_eq!(timeline.len(), TASK_VIEW_DEFAULT_ITEM_CAP);
        assert_eq!(decisions.len(), TASK_VIEW_DEFAULT_ITEM_CAP);

        assert_eq!(
            result["truncation"]["hidden"]["timeline"],
            json!(8 - TASK_VIEW_DEFAULT_ITEM_CAP)
        );
        assert_eq!(
            result["truncation"]["hidden"]["decisions"],
            json!(8 - TASK_VIEW_DEFAULT_ITEM_CAP)
        );
        assert_eq!(result["truncation"]["applied"], json!(true));

        let first_note = timeline[0]["note"].as_str().expect("note");
        assert!(
            first_note.ends_with('…'),
            "длинная note обязана быть помечена как обрезанная: {first_note}"
        );

        let serialized = serde_json::to_string(&result).expect("serialize");
        assert!(
            serialized.len() <= 12_000,
            "ответ на задачу с большой веткой обязан укладываться в разумный предел, получили {} байт",
            serialized.len()
        );
    }

    /// FR из тикета: обрезка note идёт по границе слова, а не по счётчику
    /// символов — переиспользуется `aurelius_core::graph::clip` (та же
    /// функция, что режет слои снапшота), вторая копия не заводится.
    #[test]
    fn task_view_note_truncation_cuts_at_word_boundary() {
        let (_tmp, conn) = setup();
        let task_id = seed_task_in_project(&conn, "proj-w", "задача с длинной note");
        let words: Vec<String> = (0..150).map(|i| format!("слово{i}")).collect();
        let note = words.join(" ");
        add_worklog(&conn, task_id, "proj-w", &note);

        let result =
            task_view_with_conn(&conn, &json!({"id": task_id.to_string()})).expect("task_view");
        let shown = result["timeline"][0]["note"].as_str().expect("note");
        assert!(shown.ends_with('…'), "ожидалась обрезка: {shown}");
        let trimmed = shown.trim_end_matches('…').trim_end();
        let last_token = trimmed
            .split_whitespace()
            .last()
            .expect("хотя бы одно слово");
        assert!(
            words.iter().any(|w| w == last_token),
            "обрезка обязана заканчиваться ЦЕЛЫМ словом, получили хвост '{last_token}' в '{shown}'"
        );
    }

    /// `full=true` снимает и предел на число узлов, и обрезку note.
    #[test]
    fn task_view_full_true_skips_truncation() {
        let (_tmp, conn) = setup();
        let task_id = seed_task_in_project(&conn, "proj-f", "задача целиком");
        for i in 0..8 {
            add_worklog(&conn, task_id, "proj-f", &format!("запись номер {i}"));
        }

        let result = task_view_with_conn(&conn, &json!({"id": task_id.to_string(), "full": true}))
            .expect("task_view");

        assert_eq!(result["timeline"].as_array().expect("timeline").len(), 8);
        assert_eq!(result["truncation"]["applied"], json!(false));
        assert!(result["truncation"]["item_limit_per_type"].is_null());
        assert!(result["truncation"]["note_char_budget"].is_null());
    }

    /// Провенанс вложенного узла почти всегда пуст (task_log не пишет
    /// claim/evidence/measured_at/subject) — печатать пять `null` без
    /// единого сигнала не стоит: ключ должен вовсе отсутствовать.
    #[test]
    fn task_view_omits_empty_provenance_on_nested_nodes() {
        let (_tmp, conn) = setup();
        let task_id = seed_task_in_project(&conn, "proj-p", "задача p");
        add_decision(&conn, task_id, "proj-p", "решение без provenance");

        let result =
            task_view_with_conn(&conn, &json!({"id": task_id.to_string()})).expect("task_view");

        let dec = &result["decisions"][0];
        assert!(
            dec.get("provenance").is_none(),
            "пустой provenance не обязан попадать в ответ: {dec:?}"
        );
    }

    /// `limit` — свой предел на категорию вместо умолчания.
    #[test]
    fn task_view_limit_param_overrides_default_cap() {
        let (_tmp, conn) = setup();
        let task_id = seed_task_in_project(&conn, "proj-l", "задача l");
        for i in 0..8 {
            add_worklog(&conn, task_id, "proj-l", &format!("запись {i}"));
        }

        let result = task_view_with_conn(&conn, &json!({"id": task_id.to_string(), "limit": 2}))
            .expect("task_view");

        assert_eq!(result["timeline"].as_array().expect("timeline").len(), 2);
        assert_eq!(result["truncation"]["hidden"]["timeline"], json!(6));
    }

    // -- task_list: обрезка note по границе слова, честно отчитанная --------

    fn seed_task_with_note(conn: &rusqlite::Connection, project: &str, note: &str) -> uuid::Uuid {
        graph::add_node_full(
            conn,
            NodeType::Task,
            &format!("[{project}] задача с note"),
            Some(note),
            "test",
            json!({"status": "backlog", "priority": "medium", "project": project}),
            MemoryKind::Semantic,
            None,
        )
        .expect("insert task")
        .id
    }

    /// Длинная note обрезается по границе слова (через `graph::clip`, как и
    /// в `task_view`) и честно помечена: и по хвосту «…», и отдельным
    /// булевым флагом, чтобы не гадать по внешнему виду строки.
    #[test]
    fn task_list_clips_long_note_at_word_boundary_and_reports_it() {
        let (_tmp, conn) = setup();
        let words: Vec<String> = (0..100).map(|i| format!("слово{i}")).collect();
        let note = words.join(" ");
        seed_task_with_note(&conn, "proj-list-long", &note);

        let result =
            task_list_with_conn(&conn, &json!({"project": "proj-list-long"})).expect("task_list");

        let task = &result["tasks"][0];
        let shown = task["note"].as_str().expect("note");
        assert!(
            shown.ends_with('…'),
            "длинная note обязана быть обрезана: {shown}"
        );
        assert_eq!(task["note_truncated"], json!(true));
        assert!(shown.chars().count() <= TASK_LIST_NOTE_BUDGET);

        let trimmed = shown.trim_end_matches('…').trim_end();
        let last_token = trimmed
            .split_whitespace()
            .last()
            .expect("хотя бы одно слово");
        assert!(
            words.iter().any(|w| w == last_token),
            "обрезка обязана заканчиваться ЦЕЛЫМ словом, получили хвост '{last_token}' в '{shown}'"
        );

        assert_eq!(result["note_char_budget"], json!(TASK_LIST_NOTE_BUDGET));
        assert!(result["how_to_see_full_note"]
            .as_str()
            .expect("подсказка")
            .contains("task_view"));
    }

    /// Асимметрия предыдущего теста: короткая note НЕ обрезается — ни хвоста
    /// «…», ни `note_truncated: true` быть не должно.
    #[test]
    fn task_list_keeps_short_note_whole_and_unmarked() {
        let (_tmp, conn) = setup();
        seed_task_with_note(&conn, "proj-list-short", "короткая note без обрезки");

        let result =
            task_list_with_conn(&conn, &json!({"project": "proj-list-short"})).expect("task_list");

        let task = &result["tasks"][0];
        assert_eq!(task["note"], json!("короткая note без обрезки"));
        assert_eq!(task["note_truncated"], json!(false));
    }

    // -- task_list: сводка улик вместо журнала прогонов ----------------------

    /// Три улики (красная, зелёная, зелёная позже первой) дают в списке
    /// сводку 3/2 с самой свежей зелёной, а не полный массив — то, ради чего
    /// затевалась вся правка (35 записей одной задачи весили большую часть
    /// 20-тысячесимвольного `task_list` по 16 задачам, измерено 2026-09-05).
    #[test]
    fn task_list_reports_evidence_summary_not_full_array() {
        let (_tmp, conn) = setup();
        seed_active_task(
            &conn,
            "proj-evidence-summary",
            json!({
                "status": "active",
                "priority": "medium",
                "project": "proj-evidence-summary",
                "evidence": [
                    {"command": "cargo test", "exit_code": 1, "at": "2026-08-30T09:00:00Z"},
                    {"command": "cargo clippy", "exit_code": 0, "at": "2026-08-30T09:30:00Z"},
                    {"command": "cargo test", "exit_code": 0, "at": "2026-08-30T10:00:00Z"},
                ],
            }),
        );

        let result = task_list_with_conn(&conn, &json!({"project": "proj-evidence-summary"}))
            .expect("task_list");

        let evidence = &result["tasks"][0]["evidence"];
        assert!(
            evidence.is_object(),
            "evidence обязана быть сводкой-объектом, не массивом: {evidence:?}"
        );
        assert_eq!(evidence["total"], json!(3));
        assert_eq!(evidence["green"], json!(2));
        assert_eq!(evidence["last_green"]["command"], json!("cargo test"));
        assert_eq!(evidence["last_green"]["at"], json!("2026-08-30T10:00:00Z"));
    }

    /// `full_notes=true` отдаёт note целиком прямо в списке: по умолчанию
    /// поведение не меняется, обрезка та же, что и раньше.
    #[test]
    fn task_list_full_notes_returns_note_whole() {
        let (_tmp, conn) = setup();
        let note = "a".repeat(400);
        seed_task_with_note(&conn, "proj-full-notes", &note);

        let truncated = task_list_with_conn(&conn, &json!({"project": "proj-full-notes"}))
            .expect("task_list default");
        let truncated_task = &truncated["tasks"][0];
        assert_eq!(truncated_task["note_truncated"], json!(true));
        assert!(
            truncated_task["note"]
                .as_str()
                .expect("note")
                .chars()
                .count()
                <= TASK_LIST_NOTE_BUDGET,
            "по умолчанию note обязана резаться бюджетом"
        );

        let whole = task_list_with_conn(
            &conn,
            &json!({"project": "proj-full-notes", "full_notes": true}),
        )
        .expect("task_list full_notes");
        let whole_task = &whole["tasks"][0];
        assert_eq!(whole_task["note_truncated"], json!(false));
        assert_eq!(whole_task["note"], json!(note));
    }

    /// ripe не теряется при сокращении evidence до сводки — то же
    /// вычисление, что и у `task_ripe` (`seed_active_task` ниже), применённое
    /// здесь к `task_list`.
    #[test]
    fn task_list_still_reports_ripe() {
        let (_tmp, conn) = setup();
        seed_active_task(
            &conn,
            "proj-list-ripe",
            json!({
                "status": "active",
                "priority": "medium",
                "project": "proj-list-ripe",
                "last_edit_at": "2026-08-30T09:00:00Z",
                "evidence": [{
                    "command": "cargo test",
                    "exit_code": 0,
                    "at": "2026-08-30T10:00:00Z",
                }],
            }),
        );

        let result =
            task_list_with_conn(&conn, &json!({"project": "proj-list-ripe"})).expect("task_list");

        assert_eq!(result["tasks"][0]["ripe"], json!(true));
    }

    // -- task_ripe: та же выборка, что и `au task ripe` ----------------------

    fn seed_active_task(
        conn: &rusqlite::Connection,
        project: &str,
        data: serde_json::Value,
    ) -> uuid::Uuid {
        graph::add_node_full(
            conn,
            NodeType::Task,
            &format!("[{project}] активная задача"),
            None,
            "test",
            data,
            MemoryKind::Semantic,
            None,
        )
        .expect("insert task")
        .id
    }

    /// Активная задача с зелёной уликой новее последней правки — предъявлена
    /// с основанием (какая улика, когда), той же функцией ядра, что и CLI.
    #[test]
    fn task_ripe_returns_active_task_with_evidence_basis() {
        let (_tmp, conn) = setup();
        let task_id = seed_active_task(
            &conn,
            "proj-ripe",
            json!({
                "status": "active",
                "priority": "medium",
                "project": "proj-ripe",
                "last_edit_at": "2026-08-30T09:00:00Z",
                "evidence": [{
                    "command": "cargo test",
                    "exit_code": 0,
                    "at": "2026-08-30T10:00:00Z",
                }],
            }),
        );

        let result =
            task_ripe_with_conn(&conn, &json!({"project": "proj-ripe"})).expect("task_ripe");

        assert_eq!(result["total"], json!(1));
        let entry = &result["ripe"][0];
        assert_eq!(entry["id"], json!(task_id.to_string()));
        assert_eq!(entry["evidence"]["command"], "cargo test");
        assert_eq!(entry["evidence"]["exit_code"], 0);
    }

    /// Асимметрия: активная задача без свежей улики — не созревшая,
    /// `task_ripe` обязана вернуть пустой список, а не выдумывать основание.
    #[test]
    fn task_ripe_excludes_task_without_fresh_evidence() {
        let (_tmp, conn) = setup();
        seed_active_task(
            &conn,
            "proj-not-ripe",
            json!({
                "status": "active",
                "priority": "medium",
                "project": "proj-not-ripe",
                "last_edit_at": "2026-08-30T09:00:00Z",
            }),
        );

        let result =
            task_ripe_with_conn(&conn, &json!({"project": "proj-not-ripe"})).expect("task_ripe");

        assert_eq!(result["total"], json!(0));
        assert!(result["ripe"].as_array().expect("ripe array").is_empty());
    }

    // -- task 311cae6a: task_log no longer auto-activates --

    /// Before the fix, `task_log` on a backlog task wrote `status=active`
    /// and the legacy `started_at` straight into the node, bypassing the
    /// real activation rule (`activate_task`/`task_update status=active`):
    /// no `activated_at`, no eviction of the project's real active task —
    /// so a project could end up with two active tasks at once. Recording
    /// a log line is an observation; taking a task into work is a separate,
    /// explicit decision.
    #[test]
    fn task_log_on_backlog_task_leaves_it_backlog_and_does_not_touch_the_active_task() {
        let (_tmp, conn) = setup();
        let active_id = seed_active_task(
            &conn,
            "proj-no-auto-activate",
            json!({
                "status": "active",
                "priority": "medium",
                "project": "proj-no-auto-activate",
            }),
        );
        let backlog_id = seed_task(
            &conn,
            "proj-no-auto-activate",
            "[proj-no-auto-activate] задача в очереди",
        );

        let result = task_log_with_conn(
            &conn,
            &json!({"task": backlog_id.to_string(), "text": "просто отметка о наблюдении"}),
        )
        .expect("task_log");

        assert_eq!(result["task_status"], "backlog");
        assert_eq!(
            result["hint"]
                .as_str()
                .expect("hint present on backlog task"),
            "This task was not activated: logging never changes status. \
             Use task_update status=active or `au task activate` to take it into work."
        );

        let backlog_node = graph::get_node(&conn, &backlog_id.to_string())
            .expect("get_node")
            .expect("backlog task still exists");
        assert_eq!(
            backlog_node.data.get("status").and_then(|s| s.as_str()),
            Some("backlog"),
            "task_log must not activate the task"
        );
        assert!(
            backlog_node.data.get("activated_at").is_none(),
            "task_log must not stamp activated_at"
        );

        let active_node = graph::get_node(&conn, &active_id.to_string())
            .expect("get_node")
            .expect("active task still exists");
        assert_eq!(
            active_node.data.get("status").and_then(|s| s.as_str()),
            Some("active"),
            "the project's real active task must not be evicted by a log entry on another task"
        );
    }

    /// The mirror case: logging on an already-active task reports its
    /// status without a hint — there is nothing to activate.
    #[test]
    fn task_log_on_active_task_reports_status_without_hint() {
        let (_tmp, conn) = setup();
        let active_id = seed_active_task(
            &conn,
            "proj-active-log",
            json!({
                "status": "active",
                "priority": "medium",
                "project": "proj-active-log",
            }),
        );

        let result = task_log_with_conn(
            &conn,
            &json!({"task": active_id.to_string(), "text": "прогресс по задаче"}),
        )
        .expect("task_log");

        assert_eq!(result["task_status"], "active");
        assert!(
            result.get("hint").is_none(),
            "an already-active task has nothing to activate, got: {:?}",
            result.get("hint")
        );
    }

    /// `log_work` (aurelius-core) wires the `contains` edge from the task to
    /// the new work-log node — the one place both CLI and MCP get this edge
    /// from now, instead of two separate copies of the same four lines.
    #[test]
    fn task_log_creates_work_log_node_with_contains_edge_from_task() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(&conn, "proj-edges", "[proj-edges] задача под ребро");

        let result = task_log_with_conn(
            &conn,
            &json!({"task": task_id.to_string(), "text": "запись для проверки ребра"}),
        )
        .expect("task_log");

        let created = result["created_nodes"].as_array().expect("created_nodes");
        let log_id: uuid::Uuid = created[0]["id"]
            .as_str()
            .expect("worklog id")
            .parse()
            .expect("uuid");

        let edge = graph::find_edge(&conn, task_id, log_id, &Relation::Contains)
            .expect("find_edge")
            .expect("task --contains--> worklog edge must exist");
        assert_eq!(edge.from_id, task_id);
        assert_eq!(edge.to_id, log_id);
    }

    // -- task 7e67e832: guard_subject must not contradict itself --

    /// Before the fix, `guard_subject` searched for ANY node carrying the
    /// same subject, including the very task node `task_update` was about
    /// to write to — a second `task_update` with the same subject on the
    /// SAME task was rejected as a contradiction with itself. Now the
    /// edited node is excluded from the search, while the same subject on a
    /// DIFFERENT node is still a real contradiction and still needs a
    /// `resolution`.
    #[test]
    fn task_update_same_subject_twice_on_same_task_is_not_a_self_contradiction() {
        let (_tmp, conn) = setup();

        let task_a =
            task_create_with_conn(&conn, &json!({"title": "задача A", "project": "aurelius"}))
                .expect("create task A");
        let id_a = task_a["id"].as_str().expect("id A").to_owned();

        let task_b =
            task_create_with_conn(&conn, &json!({"title": "задача B", "project": "aurelius"}))
                .expect("create task B");
        let id_b = task_b["id"].as_str().expect("id B").to_owned();

        // First task_update on A stamps the subject.
        task_update_with_conn(
            &conn,
            &json!({
                "id": id_a,
                "note": "первая правка",
                "subject": "test:guard:self",
                "confidence": "measured",
                "evidence": "test",
            }),
        )
        .expect("first task_update on A must succeed");

        // Second task_update on A with the SAME subject: not a
        // contradiction with itself, so no `resolution` is needed.
        task_update_with_conn(
            &conn,
            &json!({
                "id": id_a,
                "note": "вторая правка",
                "subject": "test:guard:self",
                "confidence": "measured",
                "evidence": "test",
            }),
        )
        .expect("second task_update on A with the same subject must succeed");

        // The same subject on a DIFFERENT task (B) is still a genuine
        // contradiction and is rejected without a `resolution`.
        let err = task_update_with_conn(
            &conn,
            &json!({
                "id": id_b,
                "note": "чужая правка",
                "subject": "test:guard:self",
            }),
        )
        .expect_err("same subject on a different task must still be rejected");
        assert!(
            err.to_string().contains("уже сказано"),
            "expected a subject-conflict message, got: {err}"
        );
    }
}
