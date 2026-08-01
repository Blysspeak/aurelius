use anyhow::{Context, Result};
use aurelius_core::{
    db, graph, identity, indexer,
    models::{MemoryKind, NodeType, Relation},
    timeforged,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use crate::{IdentityAction, ShareAction, TaskAction};

fn db_path() -> PathBuf {
    let base = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("aurelius");
    std::fs::create_dir_all(&base).ok();
    base.join("aurelius.db")
}

/// Open DB and auto-index current project if not yet indexed.
fn open_and_ensure(path: &std::path::Path) -> Result<rusqlite::Connection> {
    let conn = db::open(path)?;
    if let Ok(cwd) = std::env::current_dir() {
        if indexer::ensure_indexed(&conn, &cwd)? {
            let name = cwd.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            eprintln!("✓ Auto-indexed project '{name}'");
        }
    }
    Ok(conn)
}

pub async fn init() -> Result<()> {
    let path = db_path();
    let conn = db::open(&path)?;
    // Auto-index current project
    if let Ok(cwd) = std::env::current_dir() {
        match indexer::ensure_indexed(&conn, &cwd) {
            Ok(true) => {
                let name = cwd.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                println!("✓ Aurelius initialized at {}", path.display());
                println!("  Auto-indexed project '{name}'");
            }
            _ => {
                println!("✓ Aurelius initialized at {}", path.display());
            }
        }
    } else {
        println!("✓ Aurelius initialized at {}", path.display());
    }
    println!("  Run 'au mcp' to start the MCP server for Claude Code.");
    Ok(())
}

pub async fn note(
    text: &str,
    type_str: &str,
    label: Option<String>,
    project: Option<String>,
) -> Result<()> {
    let conn = open_and_ensure(&db_path())?;
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
    let conn = open_and_ensure(&db_path())?;
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
    let conn = open_and_ensure(&db_path())?;
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
    let conn = open_and_ensure(&db_path())?;

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
                let pri = t
                    .data
                    .get("priority")
                    .and_then(|p| p.as_str())
                    .unwrap_or("?");
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
            let st = task
                .data
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("?");
            let pri = task
                .data
                .get("priority")
                .and_then(|p| p.as_str())
                .unwrap_or("?");

            println!("Task: {}", task.label);
            println!("  ID:       {}", task.id);
            println!("  Status:   {st}");
            println!("  Priority: {pri}");
            if let Some(note) = &task.note {
                println!("  Note:     {note}");
            }

            // Acceptance criteria
            if let Some(criteria) = task
                .data
                .get("acceptance_criteria")
                .and_then(|c| c.as_array())
            {
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
            let status = task
                .data
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("backlog");
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

        TaskAction::Stats {
            project,
            since_days,
        } => {
            task_stats_cli(&conn, project.as_deref(), since_days)?;
        }
    }

    Ok(())
}

fn task_stats_cli(
    conn: &rusqlite::Connection,
    project: Option<&str>,
    since_days: Option<u64>,
) -> Result<()> {
    let tasks = graph::get_tasks_filtered(conn, project, None, None, 100_000)?;
    if tasks.is_empty() {
        println!("No tasks found.");
        return Ok(());
    }

    let mut by_status: std::collections::BTreeMap<String, usize> = Default::default();
    let mut by_priority: std::collections::BTreeMap<String, usize> = Default::default();
    let mut completion_hours: Vec<f64> = Vec::new();
    let mut blocked = 0usize;
    let mut oldest_active: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut done_in_window = 0usize;

    let now = chrono::Utc::now();
    let cutoff = since_days.map(|d| now - chrono::Duration::days(d as i64));

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
            blocked += 1;
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
            let h = (c - s).num_seconds() as f64 / 3600.0;
            if h >= 0.0 {
                completion_hours.push(h);
            }
            match cutoff {
                Some(cut) if c >= cut => done_in_window += 1,
                None if status == "done" => done_in_window += 1,
                _ => {}
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
        Some(if completion_hours.len() % 2 == 0 {
            (completion_hours[mid - 1] + completion_hours[mid]) / 2.0
        } else {
            completion_hours[mid]
        })
    };

    let total = tasks.len();
    let done = by_status.get("done").copied().unwrap_or(0);
    let rate = done as f64 / total as f64 * 100.0;

    println!(
        "Task stats{}:",
        project.map(|p| format!(" — {p}")).unwrap_or_default()
    );
    println!("  total:           {total}");
    println!("  completion rate: {rate:.1}% ({done}/{total})");
    print!("  by status:      ");
    for (k, v) in &by_status {
        print!(" {k}={v}");
    }
    println!();
    print!("  by priority:    ");
    for k in ["critical", "high", "medium", "low"] {
        if let Some(v) = by_priority.get(k) {
            print!(" {k}={v}");
        }
    }
    println!();
    if let Some(a) = avg {
        println!("  avg duration:    {a:.1}h");
    }
    if let Some(m) = median {
        println!("  median duration: {m:.1}h");
    }
    println!("  currently blocked: {blocked}");
    if let Some(s) = oldest_active {
        let days = (now - s).num_hours() as f64 / 24.0;
        println!("  oldest active:    {days:.1}d");
    }
    if since_days.is_some() {
        println!("  done in window:   {done_in_window}");
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

/// Print the skill index. Plain text by default; with `hook=true` emits the
/// Claude Code SessionStart hook JSON that injects the index into context.
pub async fn skills(hook: bool) -> Result<()> {
    let conn = db::open(&db_path())?;
    let mut skills = graph::get_nodes_by_type(&conn, &NodeType::Skill)?;
    // Most-used first — the index leads with battle-tested skills.
    skills.sort_by(|a, b| b.access_count.cmp(&a.access_count));

    if skills.is_empty() {
        if !hook {
            println!("No skills stored yet. Use skill_save (MCP) to add one.");
        }
        return Ok(());
    }

    let mut text = format!(
        "Aurelius skills ({}) — reusable how-to cards. Call skill_get <name> for the full body.\n",
        skills.len()
    );
    for n in &skills {
        let trigger = n.note.as_deref().unwrap_or("");
        let tags: Vec<&str> = n
            .data
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let tag_suffix = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        text.push_str(&format!("- {}: {}{}\n", n.label, trigger, tag_suffix));
    }

    if hook {
        let out = json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": text,
            }
        });
        println!("{}", serde_json::to_string(&out)?);
    } else {
        print!("{text}");
    }
    Ok(())
}

pub async fn mcp() -> Result<()> {
    aurelius::mcp::serve().await
}

pub async fn merge(source: &str, target: &str) -> Result<()> {
    let conn = open_and_ensure(&db_path())?;
    let src = resolve_node_any(&conn, source)?;
    let tgt = resolve_node_any(&conn, target)?;

    if src.id == tgt.id {
        anyhow::bail!("source and target resolved to the same node");
    }

    println!("Merging:");
    println!("  source: {} ({})", src.label, src.id);
    println!("  target: {} ({})", tgt.label, tgt.id);

    let stats = graph::merge_nodes(&conn, src.id, tgt.id)?;

    println!("✓ Merged");
    println!("  edges rewired:           {}", stats.edges_rewired);
    println!("  self-loops removed:      {}", stats.self_loops_removed);
    println!(
        "  duplicate edges removed: {}",
        stats.duplicate_edges_removed
    );
    if stats.note_merged {
        println!("  notes merged");
    }
    Ok(())
}

fn resolve_node_any(conn: &rusqlite::Connection, id: &str) -> Result<aurelius_core::models::Node> {
    if let Ok(uuid) = id.parse::<uuid::Uuid>() {
        if let Some(node) = graph::get_node(conn, &uuid.to_string())? {
            return Ok(node);
        }
    }
    if let Some(node) = graph::find_node_by_label(conn, id)? {
        return Ok(node);
    }
    let results = graph::search(conn, id, 1)?;
    results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("node not found: {id}"))
}

// ---------------------------------------------------------------------------
// au identity
// ---------------------------------------------------------------------------

pub async fn identity(action: IdentityAction) -> Result<()> {
    match action {
        IdentityAction::Set { name, email } => {
            let id = identity::Identity { name, email };
            id.save()?;
            println!("✓ Identity set: {}", id.as_author());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// au share
// ---------------------------------------------------------------------------

/// Wire shape of `GET /sync/pull`'s response body, per contracts/sync-api.md.
#[derive(Deserialize)]
struct SyncPullWire {
    project: String,
    nodes: Vec<aurelius_core::models::Node>,
    edges: Vec<aurelius_core::models::Edge>,
    server_seq: i64,
}

/// Wire shape of `POST /sync/push`'s response body, per contracts/sync-api.md.
#[derive(Deserialize)]
struct SyncPushWire {
    accepted: usize,
    conflicts: usize,
    server_seq: i64,
}

pub async fn share(action: ShareAction) -> Result<()> {
    match action {
        ShareAction::Issue {
            project,
            for_,
            server,
        } => share_issue(&project, &for_, &server).await,
        ShareAction::Revoke {
            project,
            for_,
            server,
        } => share_revoke(&project, &for_, &server).await,
        ShareAction::Push { project } => share_push(project).await,
        ShareAction::Pull { project } => share_pull(project).await,
        ShareAction::List => share_list().await,
        ShareAction::Disable { project } => share_disable(&project).await,
        ShareAction::Connect(args) => {
            let args: Vec<String> = args
                .into_iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            if args.len() != 2 {
                anyhow::bail!("usage: au share <server> <token>");
            }
            share_connect(&args[0], &args[1]).await
        }
    }
}

/// Normalizes a `--server`/positional server argument: a bare host becomes
/// `https://{host}/sync`; anything that already looks like a URL (including
/// plain `http://` for local testing) is used as-is.
fn normalize_server(server: &str) -> String {
    let server = server.trim_end_matches('/');
    if server.starts_with("http://") || server.starts_with("https://") {
        server.to_string()
    } else {
        format!("https://{server}/sync")
    }
}

/// Parses `"Name <email>"` into `(name, email)`.
fn parse_person(input: &str) -> Result<(String, String)> {
    let trimmed = input.trim();
    let (start, end) = match (trimmed.find('<'), trimmed.rfind('>')) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => anyhow::bail!("expected \"Name <email>\" format, got: {trimmed}"),
    };
    let name = trimmed[..start].trim().to_string();
    let email = trimmed[start + 1..end].trim().to_string();
    if name.is_empty() || email.is_empty() {
        anyhow::bail!("expected \"Name <email>\" format, got: {trimmed}");
    }
    Ok((name, email))
}

fn admin_token() -> Result<String> {
    std::env::var("AURELIUS_SYNC_ADMIN_TOKEN").map_err(|_| {
        anyhow::anyhow!(
            "AURELIUS_SYNC_ADMIN_TOKEN must be set in the environment for admin commands (au share issue/revoke)"
        )
    })
}

/// [ADMIN] Issues a collaborator token for an EXISTING local project — never
/// find-or-create, so a typo can't mint access to the wrong/a new project.
async fn share_issue(project: &str, for_: &str, server: &str) -> Result<()> {
    let admin_token = admin_token()?;
    let (person_name, person_email) = parse_person(for_)?;

    let conn = db::open(&db_path())?;
    let project_node = match graph::find_project_by_label(&conn, project)? {
        Some(n) => n,
        None => {
            let existing = graph::get_nodes_by_type(&conn, &NodeType::Project)?;
            let labels: Vec<String> = existing.into_iter().map(|n| n.label).collect();
            if labels.is_empty() {
                anyhow::bail!("no project named \"{project}\" — no local projects exist yet");
            }
            anyhow::bail!(
                "no project named \"{project}\" — did you mean one of: {}?",
                labels.join(", ")
            );
        }
    };

    let base_url = normalize_server(server);
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/grants"))
        .bearer_auth(&admin_token)
        .json(&json!({
            "project": project_node.label,
            "person_name": person_name,
            "person_email": person_email,
        }))
        .send()
        .await
        .with_context(|| format!("failed to reach sync server at {base_url}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("server returned {status}: {body}");
    }

    let body: serde_json::Value = resp.json().await?;
    let token = body
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("server response missing 'token' field"))?;

    println!(
        "✓ Issued token for {person_name} <{person_email}> on project '{}'",
        project_node.label
    );
    println!();
    println!("  {token}");
    println!();
    println!("Hand this off to the collaborator out of band. They connect with:");
    println!("  au share {server} {token}");
    Ok(())
}

/// [ADMIN] Revokes a collaborator's access; does not retract data already delivered.
async fn share_revoke(project: &str, email: &str, server: &str) -> Result<()> {
    let admin_token = admin_token()?;
    let base_url = normalize_server(server);
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/grants/revoke"))
        .bearer_auth(&admin_token)
        .json(&json!({ "project": project, "person_email": email }))
        .send()
        .await
        .with_context(|| format!("failed to reach sync server at {base_url}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("server returned {status}: {body}");
    }

    let body: serde_json::Value = resp.json().await?;
    let revoked = body.get("revoked").and_then(|v| v.as_i64()).unwrap_or(0);
    println!("✓ Revoked {revoked} grant(s) for {email} on project '{project}'");
    Ok(())
}

/// `au share <server> <token>` — the only participant-side bootstrap command.
/// Learns the project's name from the pull response itself (never typed).
async fn share_connect(server: &str, token: &str) -> Result<()> {
    if identity::current().is_none() {
        anyhow::bail!(
            "no identity configured — run `au identity set --name <name> --email <email>` first"
        );
    }

    let base_url = normalize_server(server);
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base_url}/pull?since=0"))
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("failed to reach sync server at {base_url}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("server returned {status}: {body}");
    }

    let pull: SyncPullWire = resp.json().await?;
    let conn = db::open(&db_path())?;

    let already_this_config = get_sync_config(&conn, &pull.project)?
        .map(|c| c.server_url == base_url && c.token == token)
        .unwrap_or(false);

    match graph::find_project_by_label(&conn, &pull.project)? {
        Some(_) if !already_this_config => {
            println!(
                "note: local project '{}' already exists — attaching sync to it rather than merging silently",
                pull.project
            );
        }
        Some(_) => {}
        None => {
            graph::add_node(
                &conn,
                NodeType::Project,
                &pull.project,
                None,
                "sync",
                json!({}),
            )?;
        }
    }

    apply_pulled_nodes(&conn, &pull.nodes)?;
    apply_pulled_edges(&conn, &pull.edges)?;
    upsert_sync_config(
        &conn,
        &pull.project,
        &base_url,
        token,
        true,
        pull.server_seq,
    )?;

    println!(
        "✓ Connected '{}' to {base_url} — {} nodes, {} edges bootstrapped",
        pull.project,
        pull.nodes.len(),
        pull.edges.len()
    );
    Ok(())
}

async fn share_push(project: Option<String>) -> Result<()> {
    let conn = db::open(&db_path())?;
    let targets = resolve_sync_targets(&conn, project.as_deref())?;
    if targets.is_empty() {
        println!("No sync-enabled projects.");
        return Ok(());
    }
    let client = reqwest::Client::new();
    for cfg in &targets {
        if let Err(e) = push_one(&client, &conn, cfg).await {
            eprintln!("⚠ push to '{}' failed: {e}", cfg.project_label);
        }
    }
    Ok(())
}

async fn push_one(
    client: &reqwest::Client,
    conn: &rusqlite::Connection,
    cfg: &SyncConfigRow,
) -> Result<()> {
    let project_node = graph::find_project_by_label(conn, &cfg.project_label)?
        .ok_or_else(|| anyhow::anyhow!("local project not found: {}", cfg.project_label))?;
    let member_ids = project_member_ids(conn, &project_node.id.to_string())?;
    let nodes = select_project_nodes(conn, &member_ids)?;
    let edges = select_project_edges(conn, &member_ids)?;

    let resp = client
        .post(format!("{}/push", cfg.server_url))
        .bearer_auth(&cfg.token)
        .json(&json!({ "nodes": nodes, "edges": edges }))
        .send()
        .await
        .with_context(|| format!("failed to reach sync server at {}", cfg.server_url))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("server returned {status}: {body}");
    }

    let push: SyncPushWire = resp.json().await?;
    set_sync_last_seq(conn, &cfg.project_label, push.server_seq)?;
    println!(
        "✓ Pushed '{}': {} accepted, {} conflicts, server_seq={}",
        cfg.project_label, push.accepted, push.conflicts, push.server_seq
    );
    Ok(())
}

async fn share_pull(project: Option<String>) -> Result<()> {
    let conn = db::open(&db_path())?;
    let targets = resolve_sync_targets(&conn, project.as_deref())?;
    if targets.is_empty() {
        println!("No sync-enabled projects.");
        return Ok(());
    }
    let client = reqwest::Client::new();
    for cfg in &targets {
        if let Err(e) = pull_one(&client, &conn, cfg).await {
            eprintln!("⚠ pull for '{}' failed: {e}", cfg.project_label);
        }
    }
    Ok(())
}

async fn pull_one(
    client: &reqwest::Client,
    conn: &rusqlite::Connection,
    cfg: &SyncConfigRow,
) -> Result<()> {
    let resp = client
        .get(format!("{}/pull?since={}", cfg.server_url, cfg.last_seq))
        .bearer_auth(&cfg.token)
        .send()
        .await
        .with_context(|| format!("failed to reach sync server at {}", cfg.server_url))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("server returned {status}: {body}");
    }

    let pull: SyncPullWire = resp.json().await?;
    apply_pulled_nodes(conn, &pull.nodes)?;
    apply_pulled_edges(conn, &pull.edges)?;
    set_sync_last_seq(conn, &cfg.project_label, pull.server_seq)?;
    println!(
        "✓ Pulled '{}': {} nodes, {} edges, last_seq={}",
        cfg.project_label,
        pull.nodes.len(),
        pull.edges.len(),
        pull.server_seq
    );
    Ok(())
}

async fn share_list() -> Result<()> {
    let conn = db::open(&db_path())?;
    let configs = list_sync_configs(&conn)?;
    if configs.is_empty() {
        println!("No projects connected to sync.");
        return Ok(());
    }
    for c in &configs {
        println!(
            "{:<20} {:<45} enabled={:<5} last_seq={:<6} updated_at={}",
            c.project_label, c.server_url, c.enabled, c.last_seq, c.updated_at
        );
    }
    Ok(())
}

async fn share_disable(project: &str) -> Result<()> {
    let conn = db::open(&db_path())?;
    let updated = set_sync_enabled(&conn, project, false)?;
    if !updated {
        anyhow::bail!(
            "project '{project}' has no sync_config row — nothing to disable (run `au share <server> <token>` first)"
        );
    }
    println!("✓ Sync disabled for '{project}' (local data untouched)");
    Ok(())
}

// --- sync_config table access -----------------------------------------------

struct SyncConfigRow {
    project_label: String,
    server_url: String,
    token: String,
    enabled: bool,
    last_seq: i64,
    updated_at: String,
}

fn map_sync_config_row(row: &rusqlite::Row) -> rusqlite::Result<SyncConfigRow> {
    Ok(SyncConfigRow {
        project_label: row.get(0)?,
        server_url: row.get(1)?,
        token: row.get(2)?,
        enabled: row.get(3)?,
        last_seq: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn get_sync_config(conn: &rusqlite::Connection, project: &str) -> Result<Option<SyncConfigRow>> {
    let mut stmt = conn.prepare(
        "SELECT project_label, server_url, token, enabled, last_seq, updated_at
         FROM sync_config WHERE project_label = ?1",
    )?;
    let mut rows = stmt.query_map(params![project], map_sync_config_row)?;
    Ok(rows.next().transpose()?)
}

fn list_sync_configs(conn: &rusqlite::Connection) -> Result<Vec<SyncConfigRow>> {
    let mut stmt = conn.prepare(
        "SELECT project_label, server_url, token, enabled, last_seq, updated_at
         FROM sync_config ORDER BY project_label",
    )?;
    let rows = stmt
        .query_map([], map_sync_config_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn upsert_sync_config(
    conn: &rusqlite::Connection,
    project: &str,
    server_url: &str,
    token: &str,
    enabled: bool,
    last_seq: i64,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sync_config (project_label, server_url, token, enabled, last_seq, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(project_label) DO UPDATE SET
             server_url = excluded.server_url,
             token = excluded.token,
             enabled = excluded.enabled,
             last_seq = excluded.last_seq,
             updated_at = excluded.updated_at",
        params![project, server_url, token, enabled, last_seq, now],
    )?;
    Ok(())
}

fn set_sync_last_seq(conn: &rusqlite::Connection, project: &str, last_seq: i64) -> Result<()> {
    conn.execute(
        "UPDATE sync_config SET last_seq = ?1, updated_at = ?2 WHERE project_label = ?3",
        params![last_seq, chrono::Utc::now().to_rfc3339(), project],
    )?;
    Ok(())
}

fn set_sync_enabled(conn: &rusqlite::Connection, project: &str, enabled: bool) -> Result<bool> {
    let affected = conn.execute(
        "UPDATE sync_config SET enabled = ?1, updated_at = ?2 WHERE project_label = ?3",
        params![enabled, chrono::Utc::now().to_rfc3339(), project],
    )?;
    Ok(affected > 0)
}

fn resolve_sync_targets(
    conn: &rusqlite::Connection,
    project: Option<&str>,
) -> Result<Vec<SyncConfigRow>> {
    match project {
        Some(p) => {
            let cfg = get_sync_config(conn, p)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "project '{p}' is not connected to sync — run `au share <server> <token>` first"
                )
            })?;
            if !cfg.enabled {
                anyhow::bail!("project '{p}' has sync disabled — see `au share list`");
            }
            Ok(vec![cfg])
        }
        None => Ok(list_sync_configs(conn)?
            .into_iter()
            .filter(|c| c.enabled)
            .collect()),
    }
}

// --- project-scoped node/edge selection (mirrors sync::merge's membership) -

fn project_member_ids(conn: &rusqlite::Connection, project_id: &str) -> Result<Vec<String>> {
    let mut ids = vec![project_id.to_string()];
    let mut stmt = conn.prepare(
        "SELECT DISTINCT from_id FROM edges WHERE to_id = ?1 AND relation = 'belongs_to'",
    )?;
    let rows = stmt.query_map(params![project_id], |r| r.get::<_, String>(0))?;
    for id in rows {
        ids.push(id?);
    }
    Ok(ids)
}

const SYNC_NODE_SELECT: &str =
    "SELECT id, node_type, label, note, source, data, created_at, updated_at,
        memory_kind, last_accessed_at, access_count, content_hash,
        created_by, updated_by, deleted_at, sync_seq FROM nodes";
const SYNC_EDGE_SELECT: &str =
    "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq FROM edges";

/// Selects every node (including soft-deleted tombstones) belonging to the
/// given member ids — pushed as-is so deletions propagate on the next push.
fn select_project_nodes(
    conn: &rusqlite::Connection,
    member_ids: &[String],
) -> Result<Vec<aurelius_core::models::Node>> {
    if member_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = member_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("{SYNC_NODE_SELECT} WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = member_ids
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();
    let nodes = stmt
        .query_map(refs.as_slice(), map_node_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(nodes)
}

fn select_project_edges(
    conn: &rusqlite::Connection,
    member_ids: &[String],
) -> Result<Vec<aurelius_core::models::Edge>> {
    if member_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = member_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "{SYNC_EDGE_SELECT} WHERE from_id IN ({placeholders}) AND to_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut refs: Vec<&dyn rusqlite::types::ToSql> = member_ids
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();
    refs.extend(member_ids.iter().map(|s| s as &dyn rusqlite::types::ToSql));
    let edges = stmt
        .query_map(refs.as_slice(), map_edge_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(edges)
}

fn map_node_row(row: &rusqlite::Row) -> rusqlite::Result<aurelius_core::models::Node> {
    use aurelius_core::models::{MemoryKind, Node};
    let memory_kind = match row.get::<_, String>(8)?.as_str() {
        "episodic" => MemoryKind::Episodic,
        _ => MemoryKind::Semantic,
    };
    Ok(Node {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        node_type: serde_json::from_str(&row.get::<_, String>(1)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        label: row.get(2)?,
        note: row.get(3)?,
        source: row.get(4)?,
        data: serde_json::from_str(&row.get::<_, String>(5)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_at: row
            .get::<_, String>(6)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        updated_at: row
            .get::<_, String>(7)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        memory_kind,
        last_accessed_at: row
            .get::<_, Option<String>>(9)?
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(chrono::Utc::now),
        access_count: row.get(10)?,
        content_hash: row.get(11)?,
        created_by: row.get(12)?,
        updated_by: row.get(13)?,
        deleted_at: row
            .get::<_, Option<String>>(14)?
            .and_then(|s| s.parse().ok()),
        sync_seq: row.get(15)?,
    })
}

fn map_edge_row(row: &rusqlite::Row) -> rusqlite::Result<aurelius_core::models::Edge> {
    use aurelius_core::models::Edge;
    Ok(Edge {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        from_id: row
            .get::<_, String>(1)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        to_id: row
            .get::<_, String>(2)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        relation: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        weight: row.get(4)?,
        created_at: row
            .get::<_, String>(5)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_by: row.get(6)?,
        deleted_at: row
            .get::<_, Option<String>>(7)?
            .and_then(|s| s.parse().ok()),
        sync_seq: row.get(8)?,
    })
}

/// Upserts pulled nodes as-is (server is the source of truth for what it
/// sends back) — includes tombstones, so deletions apply on the client too.
fn apply_pulled_nodes(
    conn: &rusqlite::Connection,
    nodes: &[aurelius_core::models::Node],
) -> Result<()> {
    for node in nodes {
        conn.execute(
            "INSERT INTO nodes (id, node_type, label, note, source, data, created_at, updated_at,
                    memory_kind, last_accessed_at, access_count, content_hash,
                    created_by, updated_by, deleted_at, sync_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(id) DO UPDATE SET
                 node_type = excluded.node_type, label = excluded.label, note = excluded.note,
                 source = excluded.source, data = excluded.data, created_at = excluded.created_at,
                 updated_at = excluded.updated_at, memory_kind = excluded.memory_kind,
                 last_accessed_at = excluded.last_accessed_at, access_count = excluded.access_count,
                 content_hash = excluded.content_hash, created_by = excluded.created_by,
                 updated_by = excluded.updated_by, deleted_at = excluded.deleted_at,
                 sync_seq = excluded.sync_seq",
            params![
                node.id.to_string(),
                serde_json::to_string(&node.node_type)?,
                node.label,
                node.note,
                node.source,
                serde_json::to_string(&node.data)?,
                node.created_at.to_rfc3339(),
                node.updated_at.to_rfc3339(),
                node.memory_kind.to_string(),
                node.last_accessed_at.to_rfc3339(),
                node.access_count,
                node.content_hash,
                node.created_by,
                node.updated_by,
                node.deleted_at.map(|d| d.to_rfc3339()),
                node.sync_seq,
            ],
        )?;
    }
    Ok(())
}

fn apply_pulled_edges(
    conn: &rusqlite::Connection,
    edges: &[aurelius_core::models::Edge],
) -> Result<()> {
    for edge in edges {
        conn.execute(
            "INSERT INTO edges (id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                 from_id = excluded.from_id, to_id = excluded.to_id, relation = excluded.relation,
                 weight = excluded.weight, created_at = excluded.created_at,
                 created_by = excluded.created_by, deleted_at = excluded.deleted_at,
                 sync_seq = excluded.sync_seq",
            params![
                edge.id.to_string(),
                edge.from_id.to_string(),
                edge.to_id.to_string(),
                edge.relation.to_string(),
                edge.weight,
                edge.created_at.to_rfc3339(),
                edge.created_by,
                edge.deleted_at.map(|d| d.to_rfc3339()),
                edge.sync_seq,
            ],
        )?;
    }
    Ok(())
}
