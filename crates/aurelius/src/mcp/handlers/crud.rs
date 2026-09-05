use anyhow::Result;
use aurelius_core::{
    graph, indexer,
    models::{MemoryKind, NodeType, Relation},
    provenance::{self, Provenance, Resolution},
    window,
};
use serde_json::json;
use uuid::Uuid;

use super::{
    edge_brief, node_detail, open_db, parse_node_type, parse_relation, parse_since, resolve_node,
};

/// Бит-и-Дело, ступень 3: превратить recall в транзакцию. Отфильтровать
/// заблокированные пути, открыть лабильные окна и вернуть коррекции-первыми.
/// session_id берём из параметра тула (Claude Code его прокидывает).
fn instrument_recall(
    conn: &rusqlite::Connection,
    query: &str,
    session_id: &str,
    nodes: &[aurelius_core::models::Node],
) -> Vec<serde_json::Value> {
    let sig = window::query_sig(query);
    let corrections: Vec<serde_json::Value> = window::corrections_for(conn, query)
        .unwrap_or_default()
        .into_iter()
        .map(|c| json!({ "correction": c.reason, "replacement": c.replacement_id }))
        .collect();
    for n in nodes {
        let id = n.id.to_string();
        if window::pathway_blocked(conn, &sig, &id).unwrap_or(false) {
            continue;
        }
        let content = n.note.as_deref().unwrap_or(&n.label);
        let _ = window::record_recall(conn, &sig, &id, session_id, content);
    }
    corrections
}

pub fn memory_search(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
    let type_filter = params.get("type").and_then(|t| t.as_str());
    let since = params.get("since").and_then(|s| s.as_str());

    let conn = open_db()?;
    let outcome = if let Some(type_str) = type_filter {
        let node_type = parse_node_type(type_str);
        let (terms, unmatched) = graph::query_terms(&conn, query)?;
        graph::SearchOutcome {
            nodes: graph::search_typed(&conn, query, &node_type, limit)?,
            terms,
            unmatched_terms: unmatched,
        }
    } else {
        graph::search_ranked(&conn, query, limit)?
    };
    let hint = outcome.diagnosis();
    let unmatched = outcome.unmatched_terms;
    let mut nodes = outcome.nodes;

    if let Some(since_str) = since {
        if let Some(cutoff_time) = parse_since(since_str) {
            nodes.retain(|n| n.created_at >= cutoff_time);
        }
    }

    let session_id = params
        .get("session_id")
        .and_then(|s| s.as_str())
        .unwrap_or("mcp");
    let corrections = instrument_recall(&conn, query, session_id, &nodes);

    Ok(json!({
        "query": query,
        "type": type_filter,
        "since": since,
        "corrections": corrections,
        "count": nodes.len(),
        "unmatched_terms": unmatched,
        "query_hint": hint,
        "results": nodes.iter().map(node_detail).collect::<Vec<_>>(),
    }))
}

/// Тот же хаб-узел, что и в `task_view` (см. `graph::context_from_id`): узел
/// проекта копит рёбра `belongs_to` от КАЖДОЙ задачи/решения/проблемы этого
/// проекта. BFS глубины 2 от FTS-посева проходит на первом шаге в проект, а
/// на втором — расходится обратно на ВСЕ остальные узлы проекта, выдавая их
/// как «контекст» темы, к которой они на деле отношения не имеют.
///
/// Живое измерение 30.08.2026 (`au context`, проект aurelius, тема из
/// задачи 67c9a2bb): глубина 2 — 2809 узлов, 2844 рёбра, 2.4 МБ вывода,
/// причём затронуты чужие проекты (boostix, xhub) — потому что каждый из
/// пяти FTS-посевов сам оказывается новым хабом. Глубина 1 на той же теме —
/// 12 узлов, 9 КБ. `memory_recall` (session.rs), который зовёт тот же
/// `graph::context`, уже стоит на глубине 1 по умолчанию; здесь она была
/// вторым, более старым путём к той же функции с более старым дефолтом.
const MEMORY_CONTEXT_DEFAULT_DEPTH: u32 = 1;

/// Бюджет `note` одного узла в символах (по границе слова) — то же
/// `aurelius_core::graph::clip`, что режет ветку `task_view`. Раньше note
/// отдавался сырым: при глубине 2 это и добивало ответ до мегабайт, но
/// проблема отдельная от хаба — сырой note раздувает ответ даже на честных
/// 12 узлах глубины 1, если хоть один из них содержит длинную запись.
const MEMORY_CONTEXT_NOTE_BUDGET: usize = 300;

pub fn memory_context(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    memory_context_with_conn(&conn, params)
}

/// Тело `memory_context` с явным соединением — тот же приём тестируемости,
/// что и у `task_view_with_conn`: тесты сеют граф и проверяют BFS без
/// обхода через файл базы.
fn memory_context_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let topic = params
        .get("topic")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'topic' parameter"))?;
    let depth = params
        .get("depth")
        .and_then(|d| d.as_u64())
        .unwrap_or(u64::from(MEMORY_CONTEXT_DEFAULT_DEPTH)) as u32;
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;

    let (nodes, edges) = graph::context(conn, topic, depth)?;

    let total = nodes.len();
    let capped_nodes: Vec<_> = nodes.iter().take(limit).collect();
    let hidden = total.saturating_sub(capped_nodes.len());

    for node in &capped_nodes {
        // Best effort by design: an access counter must never fail a read.
        // Logged rather than discarded so a failing write is still visible.
        if let Err(e) = graph::touch_node(conn, node.id) {
            tracing::warn!("could not record access for {}: {e}", node.id);
        }
    }

    let compact_nodes: Vec<serde_json::Value> = capped_nodes
        .iter()
        .map(|n| {
            json!({
                "id": n.id.to_string(),
                "type": n.node_type,
                "label": n.label,
                "note": n.note.as_deref().map(|note| graph::clip(note, MEMORY_CONTEXT_NOTE_BUDGET)),
            })
        })
        .collect();

    // Only include edges between nodes in the capped set
    let node_ids: std::collections::HashSet<String> =
        capped_nodes.iter().map(|n| n.id.to_string()).collect();
    let relevant_edges: Vec<serde_json::Value> = edges
        .iter()
        .filter(|e| {
            node_ids.contains(&e.from_id.to_string()) && node_ids.contains(&e.to_id.to_string())
        })
        .map(edge_brief)
        .collect();

    Ok(json!({
        "topic": topic,
        "depth": depth,
        "nodes": compact_nodes,
        "edges": relevant_edges,
        "returned": capped_nodes.len(),
        "total": total,
        // Честный отчёт об урезании — молчаливая обрезка хуже длинного
        // ответа: читатель обязан узнать, сколько осталось за кадром и как
        // это достать, а не догадываться по разнице returned/total.
        "truncation": {
            "applied": hidden > 0,
            "limit": limit,
            "hidden": hidden,
            "how_to_see_more": if hidden > 0 {
                serde_json::Value::String(
                    "memory_context с limit побольше (сейчас видно ровно limit узлов из total), \
                     либо topic поуже, чтобы BFS-посев не расходился так широко"
                        .to_owned(),
                )
            } else {
                serde_json::Value::Null
            },
        },
    }))
}

pub fn memory_add(params: &serde_json::Value) -> Result<serde_json::Value> {
    let label = params
        .get("label")
        .and_then(|l| l.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'label' parameter"))?;
    let type_str = params
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("concept");
    let note = params.get("note").and_then(|n| n.as_str());
    let source = params
        .get("source")
        .and_then(|s| s.as_str())
        .unwrap_or("mcp");
    // Метка прогона ложится в те же `data`, что и у `au note --session`: одна
    // запись, две двери — иначе выборка по сессии видела бы только половину
    // написанного.
    let data = graph::with_agent_session(
        params.get("data").cloned().unwrap_or(json!({})),
        params
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    );
    let memory_kind = match params.get("memory_kind").and_then(|m| m.as_str()) {
        Some("episodic") => MemoryKind::Episodic,
        _ => MemoryKind::Semantic,
    };

    // Происхождение разбирается ПЕРВЫМ: ошибка в нём не имеет права оставить
    // за собой полузаписанный узел.
    let prov = Provenance::parse(params)?;
    let mut data = data;
    prov.write_into(&mut data);

    // Разбор resolution — тоже до записи, по той же причине.
    let resolution = Resolution::parse_arg(params.get("resolution").and_then(|r| r.as_str()))?;

    let node_type = parse_node_type(type_str);
    let conn = open_db()?;

    // Противоречие ловится ДО записи. Два утверждения об одном предмете не
    // могут быть истинны одновременно, а граф до сих пор принимал оба молча —
    // ребро supersedes ставилось руками, то есть по памяти.
    let conflicts =
        provenance::guard_subject(&conn, prov.subject.as_deref(), resolution.is_some())?;

    let node = graph::add_node_full(
        &conn,
        node_type,
        label,
        note,
        source,
        data.clone(),
        memory_kind,
        None,
    )?;

    // Разрешение противоречия — рёбрами, а не на словах: иначе в графе снова
    // окажутся два факта без единого следа того, как они соотносятся.
    let mut resolved = Vec::new();
    if let Some(kind) = resolution {
        for old in &conflicts {
            if let Some(r) = kind.relation() {
                graph::add_edge(&conn, node.id, old.id, r, 1.0)?;
            }
            resolved.push(old.id.to_string());
        }
    }

    // Принадлежность проекту. Узел, не привязанный ни префиксом метки, ни
    // ребром, не найдётся НИ ОДНОЙ проектной выборкой — а memory_add при этом
    // возвращал "created": true. Запись, которую никто не найдёт, не имеет
    // права выглядеть удачной, поэтому: либо привязываем сами по параметру
    // project, либо говорим вслух, что узел повис.
    let project = params.get("project").and_then(|p| p.as_str());
    let mut attachment: Option<String> = None;
    if let Some(p) = project {
        let proj_node = match graph::find_project_by_label(&conn, p) {
            Ok(Some(n)) => Some(n),
            _ => graph::add_node(
                &conn,
                NodeType::Project,
                p,
                None,
                "mcp",
                json!({ "auto_created": true }),
            )
            .ok(),
        };
        match proj_node {
            Some(pn) => {
                graph::add_edge(&conn, node.id, pn.id, Relation::BelongsTo, 1.0)?;
            }
            None => attachment = Some(format!("не удалось привязать узел к проекту '{p}'")),
        }
    } else if !label.starts_with('[')
        && !matches!(
            node.node_type,
            NodeType::Project | NodeType::UserFact | NodeType::Skill
        )
    {
        attachment = Some(
            "узел не привязан ни к одному проекту: он не попадёт ни в memory_status(project=…), \
             ни в снапшот. Передай project или свяжи через memory_relate"
                .to_owned(),
        );
    }

    // Бит-и-Дело, ступень 2 (advisory-режим): проверяемые утверждения памяти
    // исполняются против ground truth прямо при рождении. Провал пока не
    // убивает узел — но виден вызывающему и записан в probes для судьи исхода.
    let node_id = node.id.to_string();
    let text = format!("{} {}", label, note.unwrap_or(""));
    let probe_warnings: Vec<String> = {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        match aurelius_core::probes::check_and_record(&conn, &node_id, &text, &cwd) {
            Ok(report) => report
                .failed
                .iter()
                .map(|p| format!("проба не прошла: {}", p.expr))
                .collect(),
            Err(e) => {
                tracing::warn!("probes failed for {}: {e}", node.id);
                Vec::new()
            }
        }
    };

    // Проба НЕ понижает уверенность, хотя один день понижала.
    //
    // Она вытаскивает путеподобные токены из прозы и проверяет их на диске —
    // улика слабая по устройству: импорт по алиасу, путь на другой машине, файл
    // в чужом репозитории провалят её, ничего не сообщив о самом факте.
    // `evidence` с командой и кодом возврата — улика сильная. Понижая measured
    // до unverified по слабой улике, инструмент обесценивал сильную: все записи
    // одного проекта разом прочитались как непроверенные, и сигнал
    // происхождения, ради которого поля и заводились, перестал что-либо значить.
    // Провал остаётся в `probe_warnings` — как замечание, а не как приговор.
    let confidence_downgraded = false;

    // Ступень 2, шлюз сюрприза (advisory): NCS против словаря scope. Scope —
    // префикс проекта из label ([proj] ...) либо global. Запись только меряет.
    let scope = label
        .strip_prefix('[')
        .and_then(|s| s.split_once(']'))
        .map_or_else(|| "global".to_owned(), |(p, _)| p.to_owned());
    let surprise = aurelius_core::codec::record(&conn, &node_id, &scope, &text)
        .map(|s| json!({ "ncs": s.ncs, "surprisal_bits": s.surprisal_bits, "epoch": s.epoch }))
        .unwrap_or(serde_json::Value::Null);

    // Что из переданного действительно легло, а что оказалось пустым. Имена
    // проверены заслонкой, но параметр с правильным именем и пустым значением
    // выглядит переданным ровно так же, как настоящий.
    let (stored_fields, dropped_fields) = super::super::params::field_report(params);

    Ok(json!({
        "id": node_id,
        "label": node.label,
        "type": type_str,
        "memory_kind": node.memory_kind,
        "created": true,
        "confidence": prov.confidence_or_default().as_str(),
        "confidence_downgraded": confidence_downgraded,
        "probe_warnings": probe_warnings,
        "project": project,
        "attachment_warning": attachment,
        "subject": prov.subject,
        "resolution": resolution.map(Resolution::as_str),
        "resolved_against": resolved,
        "stored_fields": stored_fields,
        "dropped_fields": dropped_fields,
        "surprise": surprise,
    }))
}

pub fn memory_relate(params: &serde_json::Value) -> Result<serde_json::Value> {
    let from_str = params
        .get("from")
        .and_then(|f| f.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'from' parameter"))?;
    let to_str = params
        .get("to")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'to' parameter"))?;
    let relation_str = params
        .get("relation")
        .and_then(|r| r.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'relation' parameter"))?;
    let weight = params.get("weight").and_then(|w| w.as_f64()).unwrap_or(1.0) as f32;

    let conn = open_db()?;
    let from_node = resolve_node(&conn, from_str)?;
    let to_node = resolve_node(&conn, to_str)?;
    let relation = parse_relation(relation_str)?;
    let edge = graph::add_edge(&conn, from_node.id, to_node.id, relation, weight)?;

    Ok(json!({
        "id": edge.id.to_string(),
        "from": from_node.label,
        "to": to_node.label,
        "relation": relation_str,
        "created": true,
    }))
}

pub fn memory_update(params: &serde_json::Value) -> Result<serde_json::Value> {
    let identifier = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter (UUID or label)"))?;
    let note = params.get("note").and_then(|n| n.as_str());
    let data = params.get("data").cloned();

    if note.is_none() && data.is_none() {
        anyhow::bail!("at least one of 'note' or 'data' must be provided");
    }

    let conn = open_db()?;
    let node = resolve_node(&conn, identifier)?;
    let updated = graph::update_node(&conn, node.id, note, data)?;

    Ok(json!({
        "id": node.id.to_string(),
        "label": node.label,
        "updated": updated,
    }))
}

pub fn memory_index(params: &serde_json::Value) -> Result<serde_json::Value> {
    let path = params
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;

    let conn = open_db()?;
    let result = indexer::index_project(&conn, std::path::Path::new(path))?;

    Ok(json!({
        "project": result.project_name,
        "crates_found": result.crates_found,
        "files_indexed": result.files_indexed,
        "dependencies_found": result.dependencies_found,
        "nodes_created": result.nodes_created,
        "nodes_updated": result.nodes_updated,
        "nodes_removed": result.nodes_removed,
    }))
}

pub fn memory_forget(params: &serde_json::Value) -> Result<serde_json::Value> {
    let id_str = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;
    let id: Uuid = id_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid UUID: {id_str}"))?;

    let conn = open_db()?;
    let deleted = graph::delete_node(&conn, id)?;

    Ok(json!({ "id": id_str, "deleted": deleted }))
}

pub fn memory_dump(params: &serde_json::Value) -> Result<serde_json::Value> {
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

    let conn = open_db()?;
    let total_nodes = graph::count_nodes(&conn)?;
    let total_edges = graph::count_edges(&conn)?;
    let nodes = graph::get_nodes_paginated(&conn, offset, limit)?;
    let edges = graph::get_edges_paginated(&conn, offset, limit)?;

    Ok(json!({
        "nodes": nodes.iter().map(node_detail).collect::<Vec<_>>(),
        "edges": edges.iter().map(edge_brief).collect::<Vec<_>>(),
        "total_nodes": total_nodes,
        "total_edges": total_edges,
        "offset": offset,
        "limit": limit,
    }))
}

pub fn memory_merge(params: &serde_json::Value) -> Result<serde_json::Value> {
    let source = params
        .get("source")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'source' parameter"))?;
    let target = params
        .get("target")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'target' parameter"))?;

    let conn = open_db()?;
    let src_node = resolve_node(&conn, source)?;
    let tgt_node = resolve_node(&conn, target)?;
    let stats = graph::merge_nodes(&conn, src_node.id, tgt_node.id)?;

    Ok(json!({
        "source": { "id": src_node.id.to_string(), "label": src_node.label },
        "target": { "id": tgt_node.id.to_string(), "label": tgt_node.label },
        "edges_rewired": stats.edges_rewired,
        "self_loops_removed": stats.self_loops_removed,
        "duplicate_edges_removed": stats.duplicate_edges_removed,
        "note_merged": stats.note_merged,
    }))
}

pub fn memory_gc() -> Result<serde_json::Value> {
    let conn = open_db()?;

    let dup_edges = conn.execute(
        "DELETE FROM edges WHERE id NOT IN (
            SELECT MIN(id) FROM edges GROUP BY from_id, to_id, relation
        )",
        [],
    )?;

    let orphan_edges = conn.execute(
        "DELETE FROM edges WHERE
            from_id NOT IN (SELECT id FROM nodes) OR
            to_id NOT IN (SELECT id FROM nodes)",
        [],
    )?;

    let dup_nodes = conn.execute(
        "DELETE FROM nodes WHERE content_hash IS NOT NULL AND id NOT IN (
            SELECT MIN(id) FROM nodes WHERE content_hash IS NOT NULL GROUP BY content_hash
        )",
        [],
    )?;

    // Бит-и-Дело, ступень 7: банкротство-поглощение бесполезных узлов
    // (ниже порога ценности и без подтверждённых путей) в сильнейшего соседа.
    let gc = aurelius_core::ledger::bankrupt_and_absorb(&conn, 1).unwrap_or(
        aurelius_core::ledger::GcStats {
            scanned: 0,
            absorbed: 0,
        },
    );

    Ok(json!({
        "duplicate_edges_removed": dup_edges,
        "orphan_edges_removed": orphan_edges,
        "duplicate_nodes_removed": dup_nodes,
        "bankrupt_scanned": gc.scanned,
        "bankrupt_absorbed": gc.absorbed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelius_core::db;

    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-mcp-crud-test-{tag}-{}.db",
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

    fn seed_task_in_project(conn: &rusqlite::Connection, project: &str, label: &str) -> Uuid {
        let task = graph::add_node_full(
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
        .id;
        let proj = match graph::find_project_by_label(conn, project) {
            Ok(Some(n)) => n,
            _ => graph::add_node(conn, NodeType::Project, project, None, "test", json!({}))
                .expect("insert project"),
        };
        graph::add_edge(conn, task, proj.id, Relation::BelongsTo, 1.0).expect("belongs_to");
        task
    }

    fn add_decision(conn: &rusqlite::Connection, task_id: Uuid, project: &str, text: &str) -> Uuid {
        let dec = graph::add_node(
            conn,
            NodeType::Decision,
            &format!("[{project}] {text}"),
            Some(text),
            "test",
            json!({"task_id": task_id.to_string()}),
        )
        .expect("decision");
        graph::add_edge(conn, task_id, dec.id, Relation::Contains, 1.0).expect("contains");
        dec.id
    }

    /// Тот же дефект, что и в `task_view` (см. `graph::context_from_id`), в
    /// генерике `graph::context`: живое измерение на реальной базе
    /// (30.08.2026, `au context`, тема из задачи 67c9a2bb) дало на глубине 2
    /// 2809 узлов из 12 при глубине 1 — BFS прошёл через узел проекта и
    /// вернулся на весь проект. Этот тест — та же утечка на синтетическом
    /// графе: до правки дефолта (глубина 2) падал, после (глубина 1) —
    /// проходит.
    #[test]
    fn memory_context_does_not_leak_sibling_task_via_shared_project_at_default_depth() {
        let (_tmp, conn) = setup();
        let task_a = seed_task_in_project(&conn, "proj-x", "уникальная тема альфа про морковь");
        let task_b = seed_task_in_project(&conn, "proj-x", "task B — сосед по проекту, не альфа");
        add_decision(&conn, task_b, "proj-x", "решение, принадлежащее только B");

        let result =
            memory_context_with_conn(&conn, &json!({"topic": "морковь"})).expect("memory_context");

        let nodes = result["nodes"].as_array().expect("nodes array");
        assert!(
            nodes.iter().any(|n| n["id"] == json!(task_a.to_string())),
            "искомая задача обязана быть в ответе: {nodes:?}"
        );
        assert!(
            !nodes.iter().any(|n| n["id"] == json!(task_b.to_string())),
            "сосед по проекту не обязан выглядеть контекстом темы A: {nodes:?}"
        );
    }

    /// Симметрия предыдущему тесту: явно запрошенная глубина 2 — сознательный
    /// выбор вызывающего, и утечка через хаб на ней ожидаема и не является
    /// багом обработчика (сам обход не трогаем, чинили только дефолт).
    #[test]
    fn memory_context_leaks_via_project_hub_when_depth_two_requested_explicitly() {
        let (_tmp, conn) = setup();
        let _task_a = seed_task_in_project(&conn, "proj-y", "уникальная тема бета про капуста");
        let task_b = seed_task_in_project(&conn, "proj-y", "task B — сосед, не бета");

        let result = memory_context_with_conn(&conn, &json!({"topic": "капуста", "depth": 2}))
            .expect("memory_context");

        let nodes = result["nodes"].as_array().expect("nodes array");
        assert!(
            nodes.iter().any(|n| n["id"] == json!(task_b.to_string())),
            "на явно запрошенной глубине 2 хаб-эффект воспроизводится (это ожидаемо для этого теста): {nodes:?}"
        );
    }

    /// FR из тикета: note режется по границе слова через ту же
    /// `aurelius_core::graph::clip`, что и `task_view` — вторая копия не
    /// заводится. До правки note отдавался сырым целиком.
    #[test]
    fn memory_context_clips_note_at_word_boundary() {
        let (_tmp, conn) = setup();
        let words: Vec<String> = (0..150).map(|i| format!("слово{i}")).collect();
        let long_note = words.join(" ");
        graph::add_node(
            &conn,
            NodeType::Concept,
            "уникальнаяметкагамма",
            Some(&long_note),
            "test",
            json!({}),
        )
        .expect("concept");

        let result = memory_context_with_conn(&conn, &json!({"topic": "уникальнаяметкагамма"}))
            .expect("memory_context");
        let nodes = result["nodes"].as_array().expect("nodes array");
        let note = nodes[0]["note"].as_str().expect("note");
        assert!(
            note.ends_with('…'),
            "длинная note обязана быть обрезана: {note}"
        );
        assert!(
            note.chars().count() <= MEMORY_CONTEXT_NOTE_BUDGET,
            "note обязана укладываться в бюджет {MEMORY_CONTEXT_NOTE_BUDGET}: {} символов",
            note.chars().count()
        );
    }

    /// Молчаливая обрезка хуже длинного ответа: сработавший `limit` обязан
    /// сказать, сколько узлов скрыто и как это достать, а не только поменять
    /// `returned` относительно `total`.
    #[test]
    fn memory_context_reports_hidden_count_honestly_when_limit_hits() {
        let (_tmp, conn) = setup();
        let task = seed_task_in_project(&conn, "proj-z", "уникальнаяметкадельта");
        for i in 0..5 {
            add_decision(&conn, task, "proj-z", &format!("решение {i}"));
        }

        let result = memory_context_with_conn(
            &conn,
            &json!({"topic": "уникальнаяметкадельта", "limit": 2}),
        )
        .expect("memory_context");

        assert_eq!(result["returned"], json!(2));
        assert!(result["total"].as_u64().expect("total") > 2);
        assert_eq!(result["truncation"]["applied"], json!(true));
        assert!(result["truncation"]["hidden"].as_u64().expect("hidden") > 0);
        assert!(
            !result["truncation"]["how_to_see_more"].is_null(),
            "обязан быть указан способ достать скрытое: {result:?}"
        );
    }
}
