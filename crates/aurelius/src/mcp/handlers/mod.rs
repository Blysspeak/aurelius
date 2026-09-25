mod crud;
mod doc;
mod path;
mod reminder;
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
pub use reminder::*;
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

/// `memory_add` с привязкой в момент записи ([`graph::attach_on_write`]):
/// без `project` проект выводится из префикса метки или git-репозитория
/// каталога сервера (`graph::infer_project`). Эта дверь дала 382 из 444
/// записей знания без единого ребра на 19.09.2026 — предупреждение
/// `attachment_warning` вызывающие читали и шли дальше.
///
/// Обёртка, а не правка `crud::memory_add`: наряд 19.09.2026 не открывал
/// `crud.rs` для правки. Перенести внутрь, когда тот файл будет в работе.
/// Локальное определение перекрывает одноимённое из `pub use crud::*`.
pub fn memory_add(params: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let mut out = crud::memory_add(params)?;
    // Узел уже записан: сбой привязки не превращает успех в ошибку.
    let Ok(conn) = open_db() else {
        return Ok(out);
    };
    let Some(node) = out
        .get("id")
        .and_then(|id| id.as_str())
        .and_then(|id| graph::get_node(&conn, id).ok().flatten())
    else {
        return Ok(out);
    };
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .map(str::to_owned)
        .or_else(|| {
            graph::infer_project(&conn, &node.label, std::env::current_dir().ok().as_deref())
        });
    let attached = graph::attach_on_write(&conn, &node, project.as_deref());
    if attached.project.is_some() {
        out["project"] = json!(attached.project);
        out["attachment_warning"] = serde_json::Value::Null;
    }
    out["subject_peer"] = json!(attached.subject_peer.map(|id| id.to_string()));
    Ok(out)
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

/// Decides staleness for a given executable path, in two steps:
///
/// (a) If `exe`'s string form ends with the literal suffix `" (deleted)"`,
///     return `Some(true)` without touching the filesystem. On Linux,
///     `current_exe()` resolves through `/proc/self/exe`, and the kernel
///     appends that suffix once the running image has been unlinked or
///     replaced — which is exactly what happens when a new binary is
///     installed over this one via `mv` while the server is still holding
///     the old inode open (`cp` alone fails with `ETXTBSY` for that reason).
///     A deleted running image is proof of staleness by itself; there is no
///     mtime left to compare against, and none is needed. This branch runs
///     first so it never falls through to a metadata call that would just
///     fail on the same path.
/// (b) Otherwise, fall back to reading `exe`'s mtime and comparing it against
///     `started_at` via `binary_newer_than_start`, exactly as before. On
///     Windows a running executable cannot be unlinked or overwritten out
///     from under the process holding it, so branch (a) never applies there
///     and this is the branch that actually runs.
pub(crate) fn restart_needed_for(
    exe: &std::path::Path,
    started_at: std::time::SystemTime,
) -> Option<bool> {
    if exe.to_string_lossy().ends_with(" (deleted)") {
        return Some(true);
    }
    let mtime = std::fs::metadata(exe).ok()?.modified().ok()?;
    Some(binary_newer_than_start(mtime, started_at))
}

/// Reads the running executable's own path and applies `restart_needed_for`.
/// Any failure along the way (no exe path available, mtime unsupported on
/// this platform) yields `None`: this check is a nice-to-have inside
/// `memory_status`, never a reason to fail it.
pub(crate) fn restart_needed() -> Option<bool> {
    let exe = std::env::current_exe().ok()?;
    restart_needed_for(&exe, server_started_at())
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

// ---------------------------------------------------------------------------
// Спека 011 («hybrid everywhere»): вектор запроса для трёх ручек, которые
// берут тему/запрос — `memory_search`, `memory_recall`, `memory_context`.
// Мостик sync→async — тот же приём, что у `sync_pull_if_enabled`/
// `sync_push_if_enabled` выше: обработчик остаётся синхронной функцией
// (менять на async — раздувать это по всему диспетчеру MCP), а сервер уже
// живёт под tokio-рантаймом, поэтому один блокирующий `block_on` внутри него
// безопасен.
// ---------------------------------------------------------------------------

/// Просит у демона вектор запроса — тонкая обёртка над
/// [`aurelius_core::graph::query_vector_for_search`] (единственная реализация
/// лесенки отказа, спека 011): сама ручка не решает, что делать с сокетом, она
/// только мостит синхронный вызов в асинхронный `request_vector` и отдаёт
/// результат как есть. `(None, Some(reason))` на любом отказе — сокета нет,
/// отказано в соединении, таймаут, демон ответил `{"error": ...}` — никогда
/// `Err`: вызывающая ручка обязана продолжить по FTS5 и обязана показать
/// `reason` в ответе (spec.md, ограничение №2).
pub(crate) fn query_vector_for_topic(query: &str) -> (Option<Vec<f32>>, Option<String>) {
    // `try_current`, не `current`: юнит-тесты ручек зовут их напрямую, вне
    // сервера и вне какого-либо рантайма, а паниковать здесь нельзя (правило
    // проекта — ни одного runtime-пути с паникой). Нет рантайма вокруг —
    // тот же случай, что и отказ сокета: деградация к FTS5, а не крах.
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return (
            None,
            Some("нет tokio-рантайма вокруг вызова — векторная половина недоступна".to_owned()),
        );
    };
    let path = db_path();
    let home = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    handle.block_on(graph::query_vector_for_search(home, query))
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

/// Stale notes from recorded probe failures, keyed by node id — one query for
/// the whole set. A failed probe outranks the age note: the file or commit a
/// record leans on is known gone, not merely old. The newest failing check is
/// the one named. Best-effort: a read error leaves the age notes in place.
pub(crate) fn probe_stale_notes(
    conn: &Connection,
    nodes: &[&aurelius_core::models::Node],
) -> std::collections::HashMap<String, String> {
    let ids: Vec<String> = nodes.iter().map(|n| n.id.to_string()).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    match aurelius_core::probes::failing_for(conn, &refs) {
        Ok(failing) => failing
            .into_iter()
            .filter_map(|(id, probes)| {
                probes
                    .first()
                    .map(|p| (id, aurelius_core::probes::stale_note(p)))
            })
            .collect(),
        Err(e) => {
            tracing::warn!("could not read probe results: {e}");
            std::collections::HashMap::new()
        }
    }
}

/// Replace the `stale` value at `pointer` inside a rendered record with the
/// probe note for its id, if there is one. The field must already exist:
/// this never adds `stale` where the shape had none.
pub(crate) fn apply_probe_stale(
    record: &mut serde_json::Value,
    pointer: &str,
    notes: &std::collections::HashMap<String, String>,
) {
    let Some(note) = record
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|id| notes.get(id))
        .cloned()
    else {
        return;
    };
    if let Some(slot) = record.pointer_mut(pointer) {
        *slot = json!(note);
    }
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

/// Бюджет окна вокруг совпадения. Ставится только там, где `claim` пуст:
/// заполненный claim — это и есть суть, и добавлять к нему кусок тела значит
/// возвращать одно и то же дважды.
const RECALL_WINDOW: usize = 200;

/// Форма ответа `memory_recall`: суть, а не дамп. От [`node_detail`] отличается
/// тем, чего здесь НЕТ — `data` не отдаётся никогда. Именно она раздувала
/// выдачу: тело карточки навыка уходило целиком, и три записи стоили тысячи
/// токенов. За телом идут по `id`, отдельным вызовом и осознанно.
///
/// `label` не отдаётся, когда он лишь префикс или обрезка `claim` (правило
/// общее со снапшотом, `graph::label_repeats`): 19.09.2026 у 6-7 из 12 записей
/// ответа метка была копией утверждения рядом. `created_at` — датой: время
/// до наносекунды читателю recall ни к чему.
pub(crate) fn node_recall(node: &aurelius_core::models::Node, query: &str) -> serde_json::Value {
    let p = aurelius_core::provenance::Provenance::from_data(&node.data);
    let claim = p.claim.clone();
    // Окно — замена сути, а не приложение к ней.
    let window = match &claim {
        Some(_) => None,
        None => node
            .note
            .as_deref()
            .map(|note| window_around(note, query, RECALL_WINDOW)),
    };
    let mut record = json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "claim": claim,
        "window": window,
        "subject": p.subject,
        "confidence": p.confidence_or_default().as_str(),
        "created_at": node.created_at.format("%Y-%m-%d").to_string(),
    });
    let label_copies_claim = claim
        .as_deref()
        .is_some_and(|c| graph::label_repeats(&node.label, c));
    if let (true, Some(fields)) = (label_copies_claim, record.as_object_mut()) {
        fields.remove("label");
    }
    record
}

/// Строка списка находок `memory_search` — форма [`node_recall`] плюс `stale`.
/// Список находок не карточка записи: [`node_detail`] отдавал на каждую `data`
/// целиком, полный `note`, `source`, авторов и блок происхождения, и
/// 19.09.2026 двадцать находок на «embed socket bge-m3» стоили 38 754 байта
/// против 3 110 у `au search` на том же запросе. `stale` оставлен: это
/// единственное из происхождения, по чему действуют, не открывая запись. За
/// телом идут по `id` — `au recall <id>`.
pub(crate) fn node_hit(node: &aurelius_core::models::Node, query: &str) -> serde_json::Value {
    let stale = aurelius_core::provenance::Provenance::from_data(&node.data)
        .staleness(node.created_at, chrono::Utc::now())
        .map(|s| s.note());
    let mut hit = node_recall(node, query);
    if let Some(fields) = hit.as_object_mut() {
        fields.insert("stale".to_owned(), json!(stale));
    }
    hit
}

/// Кусок текста вокруг первого совпадения любого слова запроса, по границе
/// слова. Совпадения нет — берётся начало: запись всё равно отобрана обходом,
/// и показать её начало честнее, чем не показать ничего.
fn window_around(text: &str, query: &str, budget: usize) -> String {
    let haystack = text.to_lowercase();
    let hit = query
        .split_whitespace()
        .filter(|term| term.chars().count() > 2)
        .filter_map(|term| haystack.find(&term.to_lowercase()))
        .min();

    let Some(hit) = hit else {
        return aurelius_core::graph::clip(text, budget);
    };
    // `find` вернул смещение в БАЙТАХ, а резать надо по символам: иначе на
    // кириллице граница попадёт в середину кодовой точки.
    let hit_chars = text[..hit].chars().count();
    let start = hit_chars.saturating_sub(budget / 3);
    let tail: String = text.chars().skip(start).collect();
    let body = aurelius_core::graph::clip(&tail, budget);
    if start > 0 {
        format!("…{body}")
    } else {
        body
    }
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
/// CLI (`find_task` в `crates/au/src/commands.rs`). Разница в двух местах:
/// после полного UUID пробуется уникальный префикс id (`find_node_by_id_prefix`
/// в ядре), а последний фолбэк — полнотекстовый поиск, ограниченный
/// `NodeType::Task`.
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
    if let Some(node) = graph::find_node_by_id_prefix(conn, identifier)? {
        if matches!(node.node_type, NodeType::Task) {
            return Ok(node);
        }
        let type_label = format!("{:?}", node.node_type).to_lowercase();
        anyhow::bail!("task not found: {identifier} (id prefix names a {type_label} node)");
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

/// Label preview for generated decision/problem/solution labels
/// (`task.rs`'s five call sites). Delegates to `graph::label_preview`
/// (`crates/aurelius-core/src/graph/session.rs`) — see its doc comment for
/// why a blind `chars().take(max)` here can split a structured identifier.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    graph::label_preview(s, max)
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
    use super::{binary_newer_than_start, restart_needed_for};
    use std::path::PathBuf;
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

    #[test]
    fn deleted_suffix_means_restart_needed() {
        let path = PathBuf::from("/nonexistent/dir/aurelius (deleted)");
        assert_eq!(restart_needed_for(&path, SystemTime::now()), Some(true));
    }

    #[test]
    fn missing_path_without_suffix_is_unknown() {
        let path = PathBuf::from("/nonexistent/dir/aurelius");
        assert_eq!(restart_needed_for(&path, SystemTime::now()), None);
    }

    #[test]
    fn existing_file_compares_mtime() {
        let path = std::env::temp_dir().join(format!("aurelius-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"test").expect("write temp file");

        assert_eq!(
            restart_needed_for(&path, SystemTime::UNIX_EPOCH),
            Some(true)
        );
        assert_eq!(
            restart_needed_for(&path, SystemTime::now() + Duration::from_secs(3600)),
            Some(false)
        );

        std::fs::remove_file(&path).expect("remove temp file");
    }
}

#[cfg(test)]
mod recall_window_tests {
    use super::window_around;

    /// `find` возвращает смещение в байтах, а окно режется по символам. На
    /// кириллице байт и символ не совпадают, и наивный `&text[start..]` здесь
    /// паникует на границе кодовой точки — проверяется именно этот случай.
    #[test]
    fn window_lands_on_the_match_without_splitting_a_letter() {
        let text = "начало записи, потом длинная середина, и где-то тут слово улика, \
                    а дальше снова текст";
        let window = window_around(text, "улика", 40);

        assert!(
            window.contains("улика"),
            "окно обязано содержать совпадение: {window}"
        );
        assert!(
            window.starts_with('…'),
            "срезанное начало обязано быть помечено: {window}"
        );
    }

    /// Совпадения нет — окно всё равно должно быть текстом, а не пустотой:
    /// запись отобрана обходом графа, и её начало информативнее тишины.
    #[test]
    fn window_without_a_match_falls_back_to_the_head() {
        let window = window_around("совсем про другое", "ulika", 40);
        assert_eq!(window, "совсем про другое");
    }

    /// Короткие слова запроса игнорируются: по «и» или «в» совпадение находится
    /// в любой строке и окно уезжает в случайное место.
    #[test]
    fn short_terms_do_not_steer_the_window() {
        let text = "и в на длинный текст про улику в самом конце строки";
        let window = window_around(text, "и в улику", 30);
        assert!(
            window.contains("улик"),
            "окно должно вести длинное слово, а не предлог: {window}"
        );
    }
}

#[cfg(test)]
mod recall_shape_tests {
    use super::node_recall;
    use aurelius_core::graph;
    use aurelius_core::models::NodeType;

    fn temp_conn() -> rusqlite::Connection {
        let dir =
            std::env::temp_dir().join(format!("aurelius-recall-shape-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        aurelius_core::db::open(&dir.join("test.db")).expect("open test db")
    }

    /// Метка-обрезка утверждения не отдаётся, самостоятельная метка остаётся;
    /// дата без времени; id, subject, type, confidence — всегда.
    #[test]
    fn label_copy_of_claim_is_dropped_and_created_at_is_a_date() {
        let conn = temp_conn();
        let claim = "Демон единственный владелец BGE-M3 и на чтении: сокет рядом с базой";
        let copied = graph::add_node(
            &conn,
            NodeType::Decision,
            "Демон единственный владелец BGE-M3 и на чт…",
            Some("длинное обоснование"),
            "test",
            serde_json::json!({ "claim": claim, "confidence": "measured",
                                "evidence": "cargo test", "subject": "demo:embed:owner" }),
        )
        .expect("add copied");
        let own = graph::add_node(
            &conn,
            NodeType::Decision,
            "recall: посев 12, глубина 2",
            None,
            "test",
            serde_json::json!({ "claim": claim }),
        )
        .expect("add own");

        let a = node_recall(&copied, "BGE-M3");
        assert!(a.get("label").is_none(), "метка-копия claim осталась: {a}");
        let b = node_recall(&own, "BGE-M3");
        assert_eq!(b["label"], "recall: посев 12, глубина 2");

        for r in [&a, &b] {
            for key in ["id", "subject", "type", "confidence", "claim"] {
                assert!(r.get(key).is_some(), "нет поля {key}: {r}");
            }
            let date = r["created_at"].as_str().expect("created_at строкой");
            assert!(
                chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() && date.len() == 10,
                "created_at обязан быть датой YYYY-MM-DD: {date}"
            );
        }
        assert_eq!(a["subject"], "demo:embed:owner");
        assert_eq!(a["confidence"], "measured");
    }

    /// Находка поиска — не карточка записи: ни `data`, ни полного `note`, ни
    /// авторов с происхождением; `stale` есть всегда, окно — только без claim.
    #[test]
    fn search_hit_is_a_summary_not_a_record_dump() {
        use super::node_hit;
        let conn = temp_conn();
        let body = format!("{} сокет демона рядом с базой", "вступление ".repeat(60));
        let bare = graph::add_node(
            &conn,
            NodeType::Concept,
            "сокет эмбеддингов",
            Some(&body),
            "test",
            serde_json::json!({ "skill_body": "x".repeat(4000) }),
        )
        .expect("add bare");
        let claimed = graph::add_node(
            &conn,
            NodeType::Decision,
            "демон держит модель",
            Some(&body),
            "test",
            serde_json::json!({ "claim": "Модель висит резидентно", "confidence": "measured",
                                "evidence": "cargo test", "subject": "demo:embed:resident" }),
        )
        .expect("add claimed");

        for node in [&bare, &claimed] {
            let hit = node_hit(node, "сокет");
            for key in [
                "data",
                "note",
                "source",
                "created_by",
                "updated_by",
                "provenance",
                "access_count",
                "memory_kind",
            ] {
                assert!(hit.get(key).is_none(), "лишнее поле {key}: {hit}");
            }
            for key in ["id", "type", "claim", "confidence", "subject", "stale"] {
                assert!(hit.get(key).is_some(), "нет поля {key}: {hit}");
            }
            let date = hit["created_at"].as_str().expect("created_at строкой");
            assert_eq!(date.len(), 10, "created_at обязан быть датой: {date}");
        }

        let bare_hit = node_hit(&bare, "сокет");
        let window = bare_hit["window"].as_str().expect("окно при пустом claim");
        assert!(window.contains("сокет"), "окно не на совпадении: {window}");
        assert!(window.chars().count() < body.chars().count());
        assert!(node_hit(&claimed, "сокет")["window"].is_null());
    }
}
