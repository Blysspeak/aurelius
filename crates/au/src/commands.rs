use anyhow::Result;
use aurelius_core::{db, graph, models::NodeType};
use std::path::PathBuf;

fn db_path() -> PathBuf {
    let base = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("aurelius");
    std::fs::create_dir_all(&base).ok();
    base.join("aurelius.db")
}

pub async fn init() -> Result<()> {
    let path = db_path();
    let _conn = db::open(&path)?;
    println!("✓ Aurelius initialized at {}", path.display());
    println!("  Run 'au mcp' to start the MCP server for Claude Code.");
    Ok(())
}

pub async fn note(text: &str, type_str: &str, label: Option<String>) -> Result<()> {
    let conn = db::open(&db_path())?;
    let node_type = match type_str {
        "concept" => NodeType::Concept,
        "problem" => NodeType::Problem,
        "solution" => NodeType::Solution,
        _ => NodeType::Decision,
    };
    let label = label.unwrap_or_else(|| {
        let t = text.chars().take(60).collect::<String>();
        t.trim_end().to_owned()
    });
    let node = graph::add_node(&conn, node_type, &label, Some(text), "manual", serde_json::json!({}))?
    ;
    println!("✓ Saved: [{}] {}", node.id, node.label);
    Ok(())
}

pub async fn context(topic: &str, depth: u32) -> Result<()> {
    let conn = db::open(&db_path())?;
    let (nodes, edges) = graph::context(&conn, topic, depth)?;
    if nodes.is_empty() {
        println!("No nodes found for '{}'", topic);
        return Ok(());
    }
    println!("Context for '{}' ({} nodes, {} edges):", topic, nodes.len(), edges.len());
    println!();
    for node in &nodes {
        let type_label = serde_json::to_string(&node.node_type).unwrap_or_default();
        println!("  [{type_label}] {}", node.label);
        if let Some(note) = &node.note {
            println!("    → {note}");
        }
    }
    Ok(())
}

pub async fn search(query: &str) -> Result<()> {
    let conn = db::open(&db_path())?;
    let nodes = graph::search(&conn, query, 20)?;
    if nodes.is_empty() {
        println!("No results for '{}'", query);
        return Ok(());
    }
    println!("{} results:", nodes.len());
    for node in nodes {
        let type_label = serde_json::to_string(&node.node_type).unwrap_or_default();
        println!("  [{type_label}] {} — {}", node.label, node.note.unwrap_or_default());
    }
    Ok(())
}

pub async fn sync() -> Result<()> {
    println!("Syncing connectors...");
    // TODO: git, beads, timeforged, beacon connectors
    println!("  git      — TODO");
    println!("  beads    — TODO");
    println!("  timeforged — TODO");
    println!("  beacon   — TODO");
    Ok(())
}

pub async fn export() -> Result<()> {
    let conn = db::open(&db_path())?;
    let nodes = graph::search(&conn, "*", usize::MAX)?;
    println!("{}", serde_json::to_string_pretty(&nodes)?);
    Ok(())
}

pub async fn mcp() -> Result<()> {
    aurelius::mcp::serve().await
}
