use anyhow::Result;
use aurelius_core::{db, graph, indexer, models::NodeType, timeforged};
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

pub async fn note(
    text: &str,
    type_str: &str,
    label: Option<String>,
    project: Option<String>,
) -> Result<()> {
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
    let node = graph::add_node(
        &conn,
        node_type,
        &label,
        Some(text),
        "manual",
        serde_json::json!({}),
    )?;

    // Link to project if specified
    if let Some(proj_name) = project {
        let project_node = match graph::find_node_by_label(&conn, &proj_name)? {
            Some(n) => n,
            None => graph::add_node(
                &conn,
                NodeType::Project,
                &proj_name,
                None,
                "auto",
                serde_json::json!({}),
            )?,
        };
        graph::add_edge(
            &conn,
            node.id,
            project_node.id,
            aurelius_core::models::Relation::BelongsTo,
            1.0,
        )?;
        println!("✓ Saved: [{}] {} → {}", node.id, node.label, proj_name);
    } else {
        println!("✓ Saved: [{}] {}", node.id, node.label);
    }
    Ok(())
}

pub async fn context(topic: &str, depth: u32) -> Result<()> {
    let conn = db::open(&db_path())?;
    let (nodes, edges) = graph::context(&conn, topic, depth)?;
    if nodes.is_empty() {
        println!("No nodes found for '{}'", topic);
        return Ok(());
    }
    println!(
        "Context for '{}' ({} nodes, {} edges):",
        topic,
        nodes.len(),
        edges.len()
    );
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
        println!(
            "  [{type_label}] {} — {}",
            node.label,
            node.note.unwrap_or_default()
        );
    }
    Ok(())
}

pub async fn sync() -> Result<()> {
    let conn = db::open(&db_path())?;

    println!("Syncing connectors...");

    // TimeForged connector
    let since = chrono::Utc::now() - chrono::Duration::days(7);
    let tf = timeforged::TimeForgedConnector::new(since);

    use aurelius_core::connector::Connector;
    match tf.pull().await {
        Ok(events) => {
            if events.is_empty() {
                println!("  timeforged — no new events");
            } else {
                match timeforged::sync_events(&conn, &events) {
                    Ok(result) => {
                        println!(
                            "  timeforged — {} sessions, {} projects, {} languages",
                            result.sessions, result.projects, result.languages
                        );
                    }
                    Err(e) => println!("  timeforged — sync error: {e}"),
                }
            }
        }
        Err(e) => println!("  timeforged — offline ({e})"),
    }

    // TODO: git, beads, beacon connectors
    println!("  git        — TODO");
    println!("  beads      — TODO");
    println!("  beacon     — TODO");

    Ok(())
}

pub async fn reindex(path: Option<String>) -> Result<()> {
    let project_root = match path {
        Some(p) => PathBuf::from(p),
        None => detect_project_root()?,
    };

    let conn = db::open(&db_path())?;
    let result = indexer::index_project(&conn, &project_root)?;

    println!(
        "✓ Indexed '{}': {} crates, {} files, {} deps ({} created, {} updated, {} removed)",
        result.project_name,
        result.crates_found,
        result.files_indexed,
        result.dependencies_found,
        result.nodes_created,
        result.nodes_updated,
        result.nodes_removed
    );
    Ok(())
}

fn detect_project_root() -> Result<PathBuf> {
    // Try git root first
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            return Ok(PathBuf::from(path));
        }
    }
    // Fallback to cwd
    Ok(std::env::current_dir()?)
}

pub async fn touch(file_path: &str) -> Result<()> {
    let conn = db::open(&db_path())?;

    // Canonicalize the path to match what the indexer stores
    let canonical = std::fs::canonicalize(file_path).unwrap_or_else(|_| PathBuf::from(file_path));
    let path_str = canonical.to_string_lossy();

    // Find existing File node by data.path
    if let Some(node) = graph::find_node_by_data_field(&conn, "path", &path_str)? {
        graph::touch_node(&conn, node.id)?;
    }
    // Silently do nothing if node doesn't exist — reindex will pick it up
    Ok(())
}

pub async fn export() -> Result<()> {
    let conn = db::open(&db_path())?;
    let nodes = graph::get_all_nodes(&conn)?;
    let edges = graph::get_all_edges(&conn)?;
    let output = serde_json::json!({
        "nodes": nodes,
        "edges": edges,
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub async fn mcp() -> Result<()> {
    aurelius::mcp::serve().await
}
