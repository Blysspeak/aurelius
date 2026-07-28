use anyhow::Result;
use aurelius_core::{
    graph,
    models::{MemoryKind, Node, NodeType},
};
use serde_json::json;

use super::open_db;

/// Extract tags array from a node's `data` JSON.
fn tags_of(node: &Node) -> Vec<String> {
    node.data
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Compact index entry for a skill: name + trigger + tags + usage count.
fn index_entry(n: &Node) -> serde_json::Value {
    json!({
        "name": n.label,
        "trigger": n.note,
        "tags": tags_of(n),
        "uses": n.access_count,
    })
}

/// Create or update a skill card. Upserts by `name` (the node label).
/// The `trigger` is stored as the node note (FTS-indexed → discoverable),
/// while `body` (full markdown) and `tags` live in `data` (not FTS-indexed,
/// so the body never pollutes keyword search — progressive disclosure).
pub fn skill_save(params: &serde_json::Value) -> Result<serde_json::Value> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'name' parameter"))?;
    let trigger = params
        .get("trigger")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'trigger' parameter (when to apply this skill)"))?;
    let body = params
        .get("body")
        .and_then(|b| b.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'body' parameter (full markdown content)"))?;
    let tags: Vec<String> = params
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    let data = json!({ "body": body, "tags": tags });
    let conn = open_db()?;

    // Upsert by label, but only treat an existing node as the target if it's a Skill.
    if let Some(existing) = graph::find_node_by_label(&conn, name)? {
        if matches!(existing.node_type, NodeType::Skill) {
            graph::update_node(&conn, existing.id, Some(trigger), Some(data))?;
            return Ok(json!({
                "id": existing.id.to_string(),
                "name": name,
                "updated": true,
            }));
        }
        anyhow::bail!("a non-skill node already uses the label '{name}'");
    }

    let node = graph::add_node_full(
        &conn,
        NodeType::Skill,
        name,
        Some(trigger),
        "skill",
        data,
        MemoryKind::Semantic,
        None,
    )?;

    Ok(json!({
        "id": node.id.to_string(),
        "name": name,
        "created": true,
    }))
}

/// Cheap skill index: returns only name + trigger + tags + uses (never the body).
/// This is what gets loaded every session — token-cheap, scannable.
/// Ranked by usage (access_count) so the most battle-tested skills surface first.
/// Optional `query` filters via FTS over name/trigger; optional `tag` filters by tag.
pub fn skill_list(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .filter(|s| !s.trim().is_empty());
    let tag = params.get("tag").and_then(|t| t.as_str());
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(200) as usize;

    let conn = open_db()?;
    let mut skills = match query {
        // FTS path keeps relevance ranking; no-query path we rank by usage.
        Some(q) => graph::search_typed(&conn, q, &NodeType::Skill, limit)?,
        None => {
            let mut all = graph::get_nodes_by_type(&conn, &NodeType::Skill)?;
            all.sort_by_key(|s| std::cmp::Reverse(s.access_count));
            all
        }
    };

    if let Some(t) = tag {
        skills.retain(|n| tags_of(n).iter().any(|x| x == t));
    }
    skills.truncate(limit);

    let items: Vec<serde_json::Value> = skills.iter().map(index_entry).collect();

    Ok(json!({
        "count": items.len(),
        "skills": items,
        "hint": "Call skill_get with a name to read the full body.",
    }))
}

/// Fetch one skill's full markdown body. Tries exact name first, then falls back
/// to fuzzy FTS matching so a slightly-wrong name still resolves. Bumps access_count.
pub fn skill_get(params: &serde_json::Value) -> Result<serde_json::Value> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'name' parameter"))?;

    let conn = open_db()?;

    // 1. Exact label match.
    let exact =
        graph::find_node_by_label(&conn, name)?.filter(|n| matches!(n.node_type, NodeType::Skill));

    let (node, matched_by) = match exact {
        Some(n) => (n, "exact"),
        None => {
            // 2. Fuzzy fallback: FTS over skill name/trigger.
            let mut hits = graph::search_typed(&conn, name, &NodeType::Skill, 5)?;
            if hits.is_empty() {
                let available: Vec<String> = graph::get_nodes_by_type(&conn, &NodeType::Skill)?
                    .into_iter()
                    .take(15)
                    .map(|n| n.label)
                    .collect();
                anyhow::bail!(
                    "skill not found: '{name}'. Available skills: {}",
                    if available.is_empty() {
                        "(none yet)".to_string()
                    } else {
                        available.join(", ")
                    }
                );
            }
            let best = hits.remove(0);
            // If there were other close matches, surface them so I can disambiguate.
            if !hits.is_empty() {
                let alts: Vec<String> = hits.iter().map(|n| n.label.clone()).collect();
                let body = best.data.get("body").and_then(|b| b.as_str()).unwrap_or("");
                graph::touch_node(&conn, best.id).ok();
                return Ok(json!({
                    "name": best.label,
                    "trigger": best.note,
                    "tags": tags_of(&best),
                    "uses": best.access_count,
                    "body": body,
                    "matched_by": "fuzzy",
                    "other_matches": alts,
                }));
            }
            (best, "fuzzy")
        }
    };

    graph::touch_node(&conn, node.id).ok();
    let body = node.data.get("body").and_then(|b| b.as_str()).unwrap_or("");

    Ok(json!({
        "name": node.label,
        "trigger": node.note,
        "tags": tags_of(&node),
        "uses": node.access_count,
        "body": body,
        "matched_by": matched_by,
    }))
}

/// Delete a skill card by name (skills are managed by name, not UUID).
pub fn skill_remove(params: &serde_json::Value) -> Result<serde_json::Value> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'name' parameter"))?;

    let conn = open_db()?;
    let node = graph::find_node_by_label(&conn, name)?
        .filter(|n| matches!(n.node_type, NodeType::Skill))
        .ok_or_else(|| anyhow::anyhow!("skill not found: {name}"))?;

    let deleted = graph::delete_node(&conn, node.id)?;
    Ok(json!({ "name": name, "deleted": deleted }))
}
