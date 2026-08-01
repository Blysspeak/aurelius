use anyhow::{Context, Result};
use aurelius_core::{
    db, graph, identity, indexer,
    models::{MemoryKind, NodeType, Relation},
    timeforged,
};
use serde_json::json;
use std::path::PathBuf;

use crate::{DbAction, IdentityAction, ShareAction, TaskAction};

use aurelius_core::db::db_path;

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

pub async fn context(topic: &str, depth: u32, verbose: bool) -> Result<()> {
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
        if verbose {
            print_sync_conflict(node);
        }
    }
    Ok(())
}

/// T027: surfaces `data._sync_conflict` (see data-model.md's conflict
/// bookkeeping) — the losing edit a sync conflict retained on this node —
/// when `au context -v/--verbose` is passed.
fn print_sync_conflict(node: &aurelius_core::models::Node) {
    let Some(conflict) = node.data.get("_sync_conflict") else {
        return;
    };
    println!("    ⚠ sync conflict — a losing edit was retained:");
    if let Some(updated_by) = conflict.get("updated_by").and_then(|v| v.as_str()) {
        println!("      from: {updated_by}");
    }
    if let Some(updated_at) = conflict.get("updated_at").and_then(|v| v.as_str()) {
        println!("      at:   {updated_at}");
    }
    if let Some(note) = conflict.get("note").and_then(|v| v.as_str()) {
        println!("      note: {note}");
    }
    match conflict.get("data") {
        Some(data) if !data.is_null() && *data != json!({}) => {
            println!("      data: {data}");
        }
        _ => {}
    }
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
                if let Some(created_by) = &t.created_by {
                    print!("    by: {created_by}");
                    match &t.updated_by {
                        Some(updated_by) if updated_by != created_by => {
                            println!(" (last: {updated_by})")
                        }
                        _ => println!(),
                    }
                }
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
            if let Some(created_by) = &task.created_by {
                println!("  Created by: {created_by}");
            }
            if let Some(updated_by) = &task.updated_by {
                println!("  Last actor: {updated_by}");
            }
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
        Some(if completion_hours.len().is_multiple_of(2) {
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
    skills.sort_by_key(|s| std::cmp::Reverse(s.access_count));

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

pub async fn db(action: DbAction) -> Result<()> {
    match action {
        // A snapshot is an ordinary database, so verifying one is the same
        // command pointed at a different file.
        DbAction::Check { path, full } => {
            db_check_cli(&path.map_or_else(db_path, PathBuf::from), full)
        }
        DbAction::Backup { out } => db_backup_cli(&db_path(), out),
    }
}

fn db_check_cli(path: &std::path::Path, full: bool) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("no database at {}", path.display());
    }
    let report = db::check(path, full)?;

    println!("Database: {}", path.display());
    println!(
        "  size:     {} bytes ({} pages × {})",
        report.file_bytes, report.page_count, report.page_size
    );
    if report.wal_bytes > 0 {
        println!("  wal:      {} bytes", report.wal_bytes);
    }
    match (report.nodes, report.edges) {
        (Some(nodes), Some(edges)) => println!("  content:  {nodes} nodes, {edges} edges"),
        _ => println!("  content:  unreadable"),
    }

    let mode = if full {
        "integrity_check"
    } else {
        "quick_check"
    };
    if report.ok {
        println!("✓ Integrity OK ({mode})");
        return Ok(());
    }

    println!("✗ Integrity FAILED");
    for problem in &report.problems {
        for line in problem.lines() {
            println!("    {line}");
        }
    }
    println!("  Next: `au db backup` to snapshot what is still readable.");
    anyhow::bail!(
        "integrity check failed ({} problem(s))",
        report.problems.len()
    )
}

fn db_backup_cli(path: &std::path::Path, out: Option<String>) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("no database at {}", path.display());
    }
    let dest = match out {
        Some(p) => PathBuf::from(p),
        None => path.with_file_name(format!(
            "aurelius-{}.db",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        )),
    };
    if dest.exists() {
        anyhow::bail!("destination already exists: {}", dest.display());
    }

    let bytes = db::backup_into(path, &dest)?;

    println!("✓ Backup written");
    println!("  source: {}", path.display());
    println!("  dest:   {}", dest.display());
    println!("  size:   {bytes} bytes");
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

// Push/pull, `sync_config` table access, and node/edge selection live in
// `aurelius_core::sync::client`, shared with `aurelius`'s MCP handlers
// (memory_status/memory_session automatic sync) — not duplicated here.
use aurelius_core::sync::client as sync_client;
use aurelius_core::sync::SyncPullResponse;

pub async fn share(action: ShareAction) -> Result<()> {
    match action {
        ShareAction::Issue {
            project,
            for_,
            server,
        } => share_issue(&project, for_.as_deref(), &server).await,
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
async fn share_issue(project: &str, for_: Option<&str>, server: &str) -> Result<()> {
    let admin_token = admin_token()?;
    let (person_name, person_email) = match for_ {
        Some(spec) => parse_person(spec)?,
        None => {
            let identity = aurelius_core::identity::current().ok_or_else(|| {
                anyhow::anyhow!(
                    "no --for given and no local identity set — run `au identity set --name <name> --email <email>` first, or pass --for \"Name <email>\" to issue for someone else"
                )
            })?;
            (identity.name, identity.email)
        }
    };

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

    let pull: SyncPullResponse = resp.json().await?;
    let conn = db::open(&db_path())?;

    let already_this_config = sync_client::get_sync_config(&conn, &pull.project)?
        .map(|c| c.server_url == base_url && c.token == token)
        .unwrap_or(false);
    let existed_before = graph::find_project_by_label(&conn, &pull.project)?.is_some();

    if existed_before && !already_this_config {
        println!(
            "note: local project '{}' already exists — attaching sync to it rather than merging silently",
            pull.project
        );
    }

    // Apply the bootstrapped nodes/edges first — if the server already has
    // this project (the common case), its own Project node arrives here and
    // becomes the local anchor. Only fall back to minting a local stub below
    // if the project still doesn't exist afterward (e.g. connecting before
    // anyone has pushed yet), so we never create a redundant duplicate
    // alongside the one the pull just brought down.
    sync_client::apply_pull(&conn, &pull)?;

    if graph::find_project_by_label(&conn, &pull.project)?.is_none() {
        graph::add_node(
            &conn,
            NodeType::Project,
            &pull.project,
            None,
            "sync",
            json!({}),
        )?;
    }
    sync_client::upsert_sync_config(
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
    let targets = sync_client::resolve_sync_targets(&conn, project.as_deref())?;
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
    cfg: &sync_client::SyncConfig,
) -> Result<()> {
    let push = sync_client::push_project(client, conn, cfg).await?;
    println!(
        "✓ Pushed '{}': {} accepted, {} conflicts, server_seq={}",
        cfg.project_label, push.accepted, push.conflicts, push.server_seq
    );
    Ok(())
}

async fn share_pull(project: Option<String>) -> Result<()> {
    let conn = db::open(&db_path())?;
    let targets = sync_client::resolve_sync_targets(&conn, project.as_deref())?;
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
    cfg: &sync_client::SyncConfig,
) -> Result<()> {
    let pull = sync_client::pull_project(client, conn, cfg).await?;
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
    let configs = sync_client::list_sync_configs(&conn)?;
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
    let updated = sync_client::set_sync_enabled(&conn, project, false)?;
    if !updated {
        anyhow::bail!(
            "project '{project}' has no sync_config row — nothing to disable (run `au share <server> <token>` first)"
        );
    }
    println!("✓ Sync disabled for '{project}' (local data untouched)");
    Ok(())
}
