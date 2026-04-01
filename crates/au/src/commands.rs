use anyhow::Result;
use aurelius_core::{db, graph, indexer, models::{MemoryKind, NodeType, Relation}, timeforged};
use serde_json::json;
use std::path::PathBuf;

use crate::TaskAction;

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
        let project_node = match graph::find_project_by_label(&conn, &proj_name)? {
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

pub async fn task(action: TaskAction) -> Result<()> {
    let conn = db::open(&db_path())?;

    match action {
        TaskAction::New {
            title,
            project,
            priority,
            criteria,
            description,
        } => {
            let project = project.as_deref().unwrap_or("unknown");
            let label = format!("[{}] {}", project, title);
            let task_data = json!({
                "status": "backlog",
                "priority": priority,
                "acceptance_criteria": criteria,
                "project": project,
                "started_at": null,
                "completed_at": null,
            });

            let task = graph::add_node_full(
                &conn,
                NodeType::Task,
                &label,
                description.as_deref(),
                "cli",
                task_data,
                MemoryKind::Semantic,
                None,
            )?;

            // Link to project
            let proj_node = match graph::find_project_by_label(&conn, project)? {
                Some(n) => n,
                None => graph::add_node(
                    &conn,
                    NodeType::Project,
                    project,
                    None,
                    "cli-task",
                    json!({"auto_created": true}),
                )?,
            };
            graph::add_edge(&conn, task.id, proj_node.id, Relation::BelongsTo, 1.0)?;

            println!("✓ Task created: [{}]", task.id);
            println!("  {} ({})", label, priority);
            if !criteria.is_empty() {
                println!("  Acceptance criteria:");
                for c in &criteria {
                    println!("    ☐ {c}");
                }
            }
        }

        TaskAction::List {
            project,
            status,
            priority,
        } => {
            let tasks = graph::get_tasks_filtered(
                &conn,
                project.as_deref(),
                status.as_deref(),
                priority.as_deref(),
                30,
            )?;
            if tasks.is_empty() {
                println!("No tasks found.");
                return Ok(());
            }
            println!("{} tasks:", tasks.len());
            for t in &tasks {
                let st = t.data.get("status").and_then(|s| s.as_str()).unwrap_or("?");
                let pri = t.data.get("priority").and_then(|p| p.as_str()).unwrap_or("?");
                let icon = match st {
                    "active" => "▶",
                    "blocked" => "⛔",
                    "done" => "✓",
                    "cancelled" => "✗",
                    _ => "○",
                };
                println!("  {icon} [{pri}] {} — {st}", t.label);
                println!("    id: {}", t.id);
            }
        }

        TaskAction::Show { id } => {
            let task = find_task(&conn, &id)?;
            let st = task.data.get("status").and_then(|s| s.as_str()).unwrap_or("?");
            let pri = task.data.get("priority").and_then(|p| p.as_str()).unwrap_or("?");

            println!("Task: {}", task.label);
            println!("  ID:       {}", task.id);
            println!("  Status:   {st}");
            println!("  Priority: {pri}");
            if let Some(note) = &task.note {
                println!("  Note:     {note}");
            }

            // Acceptance criteria
            if let Some(criteria) = task.data.get("acceptance_criteria").and_then(|c| c.as_array()) {
                if !criteria.is_empty() {
                    println!("\n  Acceptance criteria:");
                    for c in criteria {
                        if let Some(text) = c.as_str() {
                            println!("    ☐ {text}");
                        }
                    }
                }
            }

            // Show linked nodes via BFS
            let (nodes, _) = graph::context_from_id(&conn, &task.id.to_string(), 1)?;
            let mut work_logs = vec![];
            let mut decisions = vec![];
            let mut problems = vec![];

            for node in &nodes {
                if node.id == task.id {
                    continue;
                }
                match &node.node_type {
                    NodeType::WorkLog => work_logs.push(node),
                    NodeType::Decision => decisions.push(node),
                    NodeType::Problem => problems.push(node),
                    _ => {}
                }
            }

            if !work_logs.is_empty() {
                println!("\n  Work log ({}):", work_logs.len());
                for log in &work_logs {
                    let date = log.created_at.format("%Y-%m-%d %H:%M");
                    println!("    [{date}] {}", log.note.as_deref().unwrap_or(&log.label));
                }
            }
            if !decisions.is_empty() {
                println!("\n  Decisions ({}):", decisions.len());
                for d in &decisions {
                    println!("    • {}", d.note.as_deref().unwrap_or(&d.label));
                }
            }
            if !problems.is_empty() {
                println!("\n  Problems ({}):", problems.len());
                for p in &problems {
                    println!("    • {}", p.note.as_deref().unwrap_or(&p.label));
                }
            }
        }

        TaskAction::Log { id, text } => {
            let task = find_task(&conn, &id)?;
            let project = task
                .data
                .get("project")
                .and_then(|p| p.as_str())
                .unwrap_or("unknown");

            // Auto-activate
            let status = task.data.get("status").and_then(|s| s.as_str()).unwrap_or("backlog");
            if status == "backlog" {
                let mut data = task.data.clone();
                data["status"] = json!("active");
                data["started_at"] = json!(chrono::Utc::now().to_rfc3339());
                graph::update_node(&conn, task.id, None, Some(data))?;
                println!("  ▶ Task auto-activated");
            }

            let truncated: String = text.chars().take(60).collect();
            let log_label = format!("[{}] {}", project, truncated);
            let log_node = graph::add_node_full(
                &conn,
                NodeType::WorkLog,
                &log_label,
                Some(&text),
                "cli-task",
                json!({"task_id": task.id.to_string()}),
                MemoryKind::Episodic,
                None,
            )?;
            graph::add_edge(&conn, task.id, log_node.id, Relation::Contains, 1.0)?;

            if let Ok(Some(proj_node)) = graph::find_project_by_label(&conn, project) {
                graph::add_edge(&conn, log_node.id, proj_node.id, Relation::BelongsTo, 1.0).ok();
            }

            println!("✓ Logged work on: {}", task.label);
        }

        TaskAction::Done { id } => {
            let task = find_task(&conn, &id)?;
            let mut data = task.data.clone();
            data["status"] = json!("done");
            data["completed_at"] = json!(chrono::Utc::now().to_rfc3339());
            graph::update_node(&conn, task.id, None, Some(data))?;
            println!("✓ Task done: {}", task.label);
        }

        TaskAction::Block { id, reason } => {
            let task = find_task(&conn, &id)?;
            let mut data = task.data.clone();
            data["status"] = json!("blocked");
            data["blocked_by"] = json!(reason);
            graph::update_node(&conn, task.id, None, Some(data))?;
            println!("⛔ Task blocked: {} — {}", task.label, reason);
        }

        TaskAction::Activate { id } => {
            let task = find_task(&conn, &id)?;
            let mut data = task.data.clone();
            data["status"] = json!("active");
            if data.get("started_at").and_then(|s| s.as_str()).is_none() {
                data["started_at"] = json!(chrono::Utc::now().to_rfc3339());
            }
            data.as_object_mut().map(|o| o.remove("blocked_by"));
            graph::update_node(&conn, task.id, None, Some(data))?;
            println!("▶ Task activated: {}", task.label);
        }
    }

    Ok(())
}

fn find_task(conn: &rusqlite::Connection, id: &str) -> Result<aurelius_core::models::Node> {
    // Try UUID first
    if let Ok(uuid) = id.parse::<uuid::Uuid>() {
        if let Some(node) = graph::get_node(conn, &uuid.to_string())? {
            return Ok(node);
        }
    }
    // Try label match
    if let Some(node) = graph::find_node_by_label(conn, id)? {
        return Ok(node);
    }
    // FTS search for tasks
    let results = graph::search_typed(conn, id, &NodeType::Task, 1)?;
    results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))
}

pub async fn mcp() -> Result<()> {
    aurelius::mcp::serve().await
}
