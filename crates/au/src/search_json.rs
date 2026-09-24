//! Строка находки `au search --json` и разметка «какая половина нашла узел».
//!
//! Форма строки — та же, что у MCP `memory_search` (`id`, `type`, `label`,
//! `claim`, `subject`, `created_at` датой `YYYY-MM-DD`), плюс `score` и
//! `origin`: хукам нужен один вызов, без второго `au recall` на каждый id.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use aurelius_core::{
    graph,
    models::{Node, NodeType},
    provenance::Provenance,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// Имя типа узла так, как его пишут `--type` и JSON (`"problem"`).
pub(crate) fn type_name(node_type: &NodeType) -> String {
    serde_json::to_value(node_type)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Какая половина гибридного поиска держит каждый выданный узел в своём пуле.
///
/// Пулы те же, что строит `graph::hybrid_seeds_pooled` (`pool` с каждой
/// стороны) — повторный запрос к обоим, а не правка ядра: слияние отдаёт
/// только скор, а менять его сигнатуру ради поля вывода — лишний шов.
pub(crate) fn origin(
    conn: &rusqlite::Connection,
    query: &str,
    vector: &[f32],
    want: usize,
    nodes: &[Node],
) -> Result<HashMap<Uuid, &'static str>> {
    let pool = want.max(graph::FUSION_POOL);
    let fts: HashSet<Uuid> = graph::search_ranked(conn, query, pool)?
        .nodes
        .iter()
        .map(|n| n.id)
        .collect();
    let dense: HashSet<Uuid> = graph::dense_search(conn, vector, pool)?
        .iter()
        .map(|n| n.id)
        .collect();
    Ok(nodes
        .iter()
        .map(|n| {
            let tag = match (fts.contains(&n.id), dense.contains(&n.id)) {
                (true, true) => "both",
                (false, true) => "dense",
                _ => "fts",
            };
            (n.id, tag)
        })
        .collect())
}

/// Одна находка машинного вывода. `note` — целиком, как и в человеческом
/// выводе: хук (`aletix hooks/lib/event-recall.mjs`) сверяет ключ ошибки с
/// меткой и заметкой и берёт суть из `claim`, а без неё — из заметки.
/// Переводы строк и кавычки экранирует serde — строку JSON они не рвут.
pub(crate) fn hit(node: &Node, score: Option<f64>, origin: &str) -> Value {
    let p = Provenance::from_data(&node.data);
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "claim": p.claim,
        "subject": p.subject,
        "note": node.note,
        "created_at": node.created_at.format("%Y-%m-%d").to_string(),
        "score": score,
        "origin": origin,
    })
}
