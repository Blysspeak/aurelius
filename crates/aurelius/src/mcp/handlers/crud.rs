use anyhow::Result;
use aurelius_core::{
    graph, indexer,
    models::{MemoryKind, NodeType, Relation},
    provenance::{self, Provenance, Resolution},
    tasks, window,
};
use serde_json::json;
use uuid::Uuid;

use super::{
    edge_brief, node_detail, node_hit, open_db, parse_node_type, parse_relation, parse_since,
    query_vector_for_topic, resolve_node, resolve_task_node,
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

/// Сколько находок `memory_search` отдаёт без явного `limit` — столько же,
/// сколько `au search` (`Commands::Search` в `crates/au/src/main.rs`,
/// решение 13.09.2026: пять — там, где кончается конвейер выдачи). Две двери
/// к одному поиску с разными дефолтами отвечали на один вопрос списками
/// разной длины; больше — через `limit`, он не урезается.
const SEARCH_DEFAULT_LIMIT: u64 = 5;

pub fn memory_search(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
    let limit = params
        .get("limit")
        .and_then(|l| l.as_u64())
        .unwrap_or(SEARCH_DEFAULT_LIMIT) as usize;
    let type_filter = params.get("type").and_then(|t| t.as_str());
    let since = params.get("since").and_then(|s| s.as_str());

    let conn = open_db()?;
    let node_type = type_filter.map(parse_node_type);
    let (outcome, vector_notice) = search_outcome(
        &conn,
        query,
        limit,
        node_type.as_ref(),
        query_vector_for_topic(query),
    )?;
    let hint = outcome.diagnosis();
    let unmatched = outcome.unmatched_terms;
    let (route, mut nodes) = route_subject(&conn, query, node_type.as_ref(), outcome.nodes, limit)?;

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
        "route": route,
        "type": type_filter,
        "since": since,
        "corrections": corrections,
        "count": nodes.len(),
        "unmatched_terms": unmatched,
        "query_hint": hint,
        // `None` — гибридный путь сработал (с фильтром по типу и без).
        // `Some` — причина, по которой ответ дан по чистому
        // полнотекстовому индексу; поле, а не только строка в тексте — так
        // потребитель JSON видит деградацию, а не только человек (spec.md,
        // ограничение №2).
        "vector_notice": vector_notice,
        "results": nodes.iter().map(|n| node_hit(n, query)).collect::<Vec<_>>(),
    }))
}

/// True when the trimmed query is one token shaped like a subject key:
/// `^[A-Za-z0-9][A-Za-z0-9._/@#-]*(:\\S+)+$`.
fn is_subject_key(query: &str) -> bool {
    let q = query.trim();
    if q.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = q.split(':');
    let head = parts.next().unwrap_or("");
    let head_ok = head
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/@#-".contains(c));
    let mut tail = parts.peekable();
    head_ok && tail.peek().is_some() && tail.all(|seg| !seg.is_empty())
}

/// Subject routing for `memory_search`: a key-shaped query puts live nodes
/// with that exact subject first, then the newest member of each
/// `query:`-prefixed family, then the usual hits without duplicates, within
/// `limit`. Returns the route name and the merged list.
fn route_subject(
    conn: &rusqlite::Connection,
    query: &str,
    node_type: Option<&NodeType>,
    hits: Vec<aurelius_core::models::Node>,
    limit: usize,
) -> Result<(&'static str, Vec<aurelius_core::models::Node>)> {
    if !is_subject_key(query) {
        return Ok(("search", hits));
    }
    let key = query.trim();
    let child = format!("{key}:");
    let mut families = graph::find_subject_families_by_prefix(conn, key)?;
    families.retain(|f| f.subject == key || f.subject.starts_with(&child));
    // Stable sort keeps newest-first within each group.
    families.sort_by_key(|f| f.subject != key);
    let mut nodes: Vec<aurelius_core::models::Node> = families
        .into_iter()
        .map(|f| f.newest)
        .filter(|n| node_type.is_none_or(|t| format!("{:?}", n.node_type) == format!("{t:?}")))
        .collect();
    for n in hits {
        if !nodes.iter().any(|m| m.id == n.id) {
            nodes.push(n);
        }
    }
    nodes.truncate(limit);
    Ok(("subject", nodes))
}

/// Picks the engines for `memory_search`. Typed and untyped queries take the
/// same hybrid path; the type filter is applied to the fused candidates. When
/// the vector half is unavailable, the answer is FTS-only and the notice says
/// why, for both shapes of the query.
fn search_outcome(
    conn: &rusqlite::Connection,
    query: &str,
    limit: usize,
    node_type: Option<&NodeType>,
    (vector, notice): (Option<Vec<f32>>, Option<String>),
) -> Result<(graph::SearchOutcome, Option<String>)> {
    let fts = |conn: &rusqlite::Connection| -> Result<graph::SearchOutcome> {
        match node_type {
            Some(t) => {
                let (terms, unmatched_terms) = graph::query_terms(conn, query)?;
                Ok(graph::SearchOutcome {
                    nodes: graph::search_typed(conn, query, t, limit)?,
                    terms,
                    unmatched_terms,
                })
            }
            None => graph::search_ranked(conn, query, limit),
        }
    };
    let outcome = fts(conn)?;
    let Some(vector) = vector else {
        return Ok((outcome, notice));
    };
    let fused = match node_type {
        Some(t) => {
            let wanted = serde_json::to_string(t).unwrap_or_default();
            graph::hybrid_seeds_pooled(
                conn,
                query,
                &vector,
                usize::MAX,
                graph::FILTERED_POOL.max(limit),
            )
            .map(|(nodes, _)| {
                let mut nodes: Vec<aurelius_core::models::Node> = nodes
                    .into_iter()
                    .filter(|n| serde_json::to_string(&n.node_type).unwrap_or_default() == wanted)
                    .collect();
                nodes.truncate(limit);
                nodes
            })
        }
        None => graph::hybrid_seeds(conn, query, &vector, limit).map(|(nodes, _)| nodes),
    };
    Ok(match fused {
        Ok(nodes) => (
            graph::SearchOutcome {
                nodes,
                terms: outcome.terms,
                unmatched_terms: outcome.unmatched_terms,
            },
            None,
        ),
        Err(e) => (
            outcome,
            Some(format!(
                "гибридный поиск не выполнился, отвечаю по полнотекстовому — {e}"
            )),
        ),
    })
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

    let (vector, notice) = query_vector_for_topic(topic);
    let (traversal, vector_notice) = match vector {
        Some(vector) => match graph::context_with_report_seeded_hybrid(
            conn,
            topic,
            depth,
            graph::DEFAULT_SEEDS,
            Some(&vector),
        ) {
            Ok(t) => (t, None),
            Err(e) => (
                graph::context_with_report(conn, topic, depth)?,
                Some(format!(
                    "гибридный обход не выполнился, отвечаю по полнотекстовому — {e}"
                )),
            ),
        },
        None => (graph::context_with_report(conn, topic, depth)?, notice),
    };
    let (nodes, edges) = (traversal.nodes, traversal.edges);

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
        // Как и у `memory_search`: `None` — гибрид сработал, `Some` —
        // причина отката к полнотекстовому обходу, отдельным полем, не
        // только строкой (spec.md, ограничение №2).
        "vector_notice": vector_notice,
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
    // `exclude: None` — memory_add always creates a new node.
    let conflicts =
        provenance::guard_subject(&conn, prov.subject.as_deref(), resolution.is_some(), None)?;

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

/// `task_criterion` — MCP door onto `au task criterion <id> [--met|--unmet] <criterion>`
/// (`crates/au/src/commands.rs`, `TaskAction::Criterion`; the underlying
/// logic — `tasks::task_criteria`/`resolve_criterion`/`set_criterion_met` —
/// lives in `aurelius_core::tasks` and is called here exactly as the CLI
/// calls it, not reimplemented). Marks one acceptance criterion of a task
/// met or unmet, addressed by the stable handle a listing call prints (or by
/// its exact text, or `#N`) — never by position, since adding a criterion
/// renumbers every one after it. With neither `met` nor `unmet`, lists the
/// task's criteria and each one's handle, the same "list first, act second"
/// shape the CLI has.
///
/// Marking a criterion met is a record of progress, not a closing condition:
/// neither `task_ripe` nor `task_update`'s status transition reads these
/// marks — closing a task stays a decision made through `task_update`.
pub fn task_criterion(params: &serde_json::Value) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter (task UUID or label)"))?;
    let met = params.get("met").and_then(|m| m.as_str());
    let unmet = params.get("unmet").and_then(|m| m.as_str());
    if met.is_some() && unmet.is_some() {
        anyhow::bail!("pass only one of 'met' or 'unmet', not both");
    }

    let conn = open_db()?;
    let task = resolve_task_node(&conn, id)?;
    let criteria = tasks::task_criteria(&task.data);

    if let Some(selector) = met.or(unmet) {
        let mark = unmet.is_none();
        let criterion = tasks::resolve_criterion(&criteria, selector)?;
        let handle = criterion.handle.clone();
        let text = criterion.text.clone();
        let changed = tasks::set_criterion_met(&conn, task.id, &handle, mark)?;
        return Ok(json!({
            "id": task.id.to_string(),
            "label": task.label,
            "handle": handle,
            "text": text,
            "met": mark,
            "changed": changed,
        }));
    }

    let orphaned_marks = tasks::orphaned_criteria_marks(&task.data);
    Ok(json!({
        "id": task.id.to_string(),
        "label": task.label,
        "criteria": criteria.iter().map(|c| json!({
            "handle": c.handle,
            "text": c.text,
            "met": c.met_at.is_some(),
            "met_at": c.met_at.map(|t| t.to_rfc3339()),
        })).collect::<Vec<_>>(),
        "orphaned_marks": orphaned_marks,
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

    fn seed(conn: &rusqlite::Connection, t: NodeType, label: &str) -> Uuid {
        graph::add_node_full(
            conn,
            t,
            label,
            None,
            "test",
            json!({}),
            MemoryKind::Semantic,
            None,
        )
        .expect("seed node")
        .id
    }

    #[test]
    fn subject_key_query_routes_exact_subject_first() {
        let (_tmp, conn) = setup();
        let exact = graph::add_node_full(
            &conn,
            NodeType::Decision,
            "walrus rule",
            None,
            "test",
            json!({ "subject": "zoo:walrus" }),
            MemoryKind::Semantic,
            None,
        )
        .expect("seed exact")
        .id;
        let other = seed(&conn, NodeType::Decision, "zoo walrus plain");
        assert!(is_subject_key("zoo:walrus"));
        assert!(!is_subject_key("zoo walrus"));
        assert!(!is_subject_key("zoo:"));

        let other_node = graph::get_node(&conn, &other.to_string())
            .expect("get")
            .expect("node");
        let (route, nodes) =
            route_subject(&conn, "zoo:walrus", None, vec![other_node], 5).expect("route");
        assert_eq!(route, "subject");
        assert_eq!(nodes.first().map(|n| n.id), Some(exact));
        assert_eq!(nodes.len(), 2);

        let (route, _) = route_subject(&conn, "walrus", None, vec![], 5).expect("route");
        assert_eq!(route, "search");
    }

    #[test]
    fn typed_search_without_vectors_reports_notice_like_untyped() {
        let (_tmp, conn) = setup();
        let problem = seed(&conn, NodeType::Problem, "walrus deadlock in daemon");
        seed(&conn, NodeType::Decision, "walrus deadlock decision");
        let offline = || (None, Some("демон недоступен".to_owned()));

        let (typed, typed_notice) =
            search_outcome(&conn, "walrus", 5, Some(&NodeType::Problem), offline())
                .expect("typed search");
        let (untyped, untyped_notice) =
            search_outcome(&conn, "walrus", 5, None, offline()).expect("untyped search");

        assert_eq!(
            typed.nodes.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![problem]
        );
        assert_eq!(untyped.nodes.len(), 2);
        assert!(typed_notice.is_some(), "typed FTS-only answer must say so");
        assert_eq!(
            typed_notice, untyped_notice,
            "both shapes ran the same engines"
        );
    }

    #[test]
    fn typed_search_with_vector_takes_hybrid_path() {
        let (_tmp, conn) = setup();
        let problem = seed(&conn, NodeType::Problem, "walrus deadlock in daemon");
        seed(&conn, NodeType::Decision, "walrus deadlock decision");
        // A vector that fails to fuse (wrong dimension or no table) must
        // surface as a notice, exactly as on the untyped path.
        let v = || (Some(vec![0.0_f32; 3]), None);
        let (typed, typed_notice) =
            search_outcome(&conn, "walrus", 5, Some(&NodeType::Problem), v()).expect("typed");
        let (_, untyped_notice) = search_outcome(&conn, "walrus", 5, None, v()).expect("untyped");
        assert_eq!(typed_notice.is_some(), untyped_notice.is_some());
        assert!(typed
            .nodes
            .iter()
            .all(|n| matches!(n.node_type, NodeType::Problem)));
        if typed_notice.is_none() {
            assert!(typed.nodes.iter().any(|n| n.id == problem));
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
