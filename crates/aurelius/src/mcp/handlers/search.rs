use anyhow::Result;
use serde_json::json;

use super::open_db;
use crate::search::{brave, cache, perplexity, Provider, SearchResult};

/// Search the web via Brave or Perplexity (selected by `provider`), with
/// per-provider caching and optional graph integration.
pub fn search_web(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;

    let count = params.get("count").and_then(|c| c.as_u64()).unwrap_or(5) as usize;

    let cache_days = params
        .get("cache_days")
        .and_then(|c| c.as_i64())
        .unwrap_or(7);

    let save_to_graph = params
        .get("save_to_graph")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    let provider_str = params
        .get("provider")
        .and_then(|p| p.as_str())
        .unwrap_or("brave");
    let provider = Provider::parse(provider_str).ok_or_else(|| {
        anyhow::anyhow!("unknown provider '{provider_str}', expected one of: brave, perplexity")
    })?;

    let conn = open_db()?;

    // Check cache first, scoped to this provider.
    if let Some(cached) = cache::get(&conn, query, provider.as_str())? {
        return Ok(json!({
            "source": "cache",
            "provider": provider.as_str(),
            "cached_at": cached.created_at,
            "query": cached.query,
            "results": cached.results,
        }));
    }

    // Cache miss — hit the selected provider's API.
    let results: Vec<SearchResult> = match provider {
        Provider::Brave => brave::search(query, count)?,
        Provider::Perplexity => perplexity::search(query, count)?,
    };

    // Store in cache
    cache::put(&conn, query, &results, provider.as_str(), cache_days)?;

    // Optionally save to knowledge graph
    if save_to_graph {
        save_search_to_graph(&conn, query, &results)?;
    }

    Ok(json!({
        "source": provider.as_str(),
        "provider": provider.as_str(),
        "query": query,
        "results": results,
    }))
}

/// Search through previously cached search results via FTS.
pub fn search_recall(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;

    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(10) as usize;

    let conn = open_db()?;
    let results = cache::recall(&conn, query, limit)?;

    Ok(json!({
        "query": query,
        "matches": results.iter().map(|r| json!({
            "original_query": r.query,
            "searched_at": r.created_at,
            "results": r.results,
        })).collect::<Vec<_>>(),
    }))
}

fn save_search_to_graph(
    conn: &rusqlite::Connection,
    query: &str,
    results: &[SearchResult],
) -> Result<()> {
    use aurelius_core::{graph, models::NodeType};

    let summary = results
        .iter()
        .map(|r| format!("- {} ({})", r.title, r.url))
        .collect::<Vec<_>>()
        .join("\n");

    let note = format!("Web search: {query}\n\n{summary}");
    let data = serde_json::to_value(results)?;

    graph::add_node(
        conn,
        NodeType::Concept,
        &format!("search: {query}"),
        Some(&note),
        "search",
        json!({ "search_results": data }),
    )?;

    Ok(())
}
