use anyhow::Result;
use aurelius_core::{graph, indexer, models::NodeType};
use rusqlite::Connection;
use serde_json::json;

use super::{
    node_brief, node_detail, open_db, restart_needed, server_started_at, sync_pull_if_enabled,
};

/// `server` block of `memory_status`: which MCP server process answered, and
/// whether the binary on disk has moved past it. `restart_needed`/`hint` are
/// omitted rather than `null` when the check couldn't run (see
/// `super::restart_needed`), so their absence reads as "unknown", not "no".
fn server_block() -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    fields.insert("version".to_owned(), json!(env!("CARGO_PKG_VERSION")));
    fields.insert(
        "started_at".to_owned(),
        json!(chrono::DateTime::<chrono::Utc>::from(server_started_at()).to_rfc3339()),
    );
    if let Some(needs_restart) = restart_needed() {
        fields.insert("restart_needed".to_owned(), json!(needs_restart));
        if needs_restart {
            fields.insert(
                "hint".to_owned(),
                json!(
                    "The binary on disk is newer than the running server; restart Claude Code so the MCP server picks it up."
                ),
            );
        }
    }
    serde_json::Value::Object(fields)
}

/// Note budget for one compact item, in characters, clipped at a word
/// boundary through `graph::clip` — the same helper `task_list` uses, so the
/// excerpt a status shows and the excerpt a list shows of the same node can
/// never disagree. The full text stays in the node, reachable through
/// `task_view` / `memory_search`.
const STATUS_NOTE_BUDGET: usize = 200;

/// How many items each capped section shows in compact mode. `full=true`
/// keeps the historical selection instead (5 sessions, 30 skills).
const COMPACT_LIST_CAP: usize = 10;
const COMPACT_SKILLS_CAP: usize = 20;

/// Compact mode fetches with this padded limit and caps in memory, so the
/// truncation block can report the exact number of hidden items (the same
/// trick `snapshot::gather` uses for active tasks) rather than "at least one
/// more". A section past this size gets an undercounted hidden number — by
/// then the graph itself is the problem, not the report.
const COMPACT_FETCH_LIMIT: usize = 1000;

/// Clip one optional note to [`STATUS_NOTE_BUDGET`] and say whether the
/// budget actually cut it — the same honesty rule as `task_list`:
/// `note_truncated` is a boolean flag, not a guess from a trailing ellipsis.
fn clipped_note(note: Option<&str>) -> (Option<String>, bool) {
    match note {
        None => (None, false),
        Some(n) => {
            let clipped = graph::clip(n, STATUS_NOTE_BUDGET);
            let truncated = clipped.ends_with('…');
            (Some(clipped), truncated)
        }
    }
}

/// Provenance in compact form. `confidence` is always present — silence about
/// provenance reads as measured, and that misreading is exactly what the field
/// exists to prevent — while `subject` and the stale warning show up only when
/// they have something to say. The verbose half (evidence command text,
/// `verify_with`, `volatility`, `measured_at`) stays in `data` and in
/// `full=true`: measured on the live project, `data` + full provenance were
/// ~44k of the ~89k compact characters, nearly all of it the same facts
/// serialized twice.
fn compact_provenance(node: &aurelius_core::models::Node) -> serde_json::Value {
    let p = aurelius_core::provenance::Provenance::from_data(&node.data);
    json!({
        "confidence": p.confidence_or_default().as_str(),
        "subject": p.subject,
        "stale": p.staleness(node.created_at, chrono::Utc::now()).map(|s| s.note()),
    })
}

/// One knowledge node (decision/problem/solution/session) in compact form:
/// claim whole, note as a budgeted excerpt, provenance down to its signal,
/// and the fields that are boilerplate in a status view dropped — the listed
/// `source`/`memory_kind`/`created_by`/`updated_by`, plus `data`, which on
/// every node duplicates what `claim` and `provenance` already say (and
/// carries `next_steps`/`key_files`/evidence texts that belong to a detail
/// view, not an orientation one). `full=true` returns `node_detail` instead,
/// which carries every field.
fn compact_node_json(node: &aurelius_core::models::Node) -> serde_json::Value {
    let (note, note_truncated) = clipped_note(node.note.as_deref());
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "label": node.label,
        "claim": aurelius_core::provenance::Provenance::from_data(&node.data).claim,
        "note": note,
        "note_truncated": note_truncated,
        "created_at": node.created_at.to_rfc3339(),
        "access_count": node.access_count,
        "provenance": compact_provenance(node),
    })
}

pub fn memory_status(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    memory_status_with_conn(&conn, params)
}

/// Body of `memory_status` with an explicit connection — the same
/// testability trick as `task_update_with_conn`: a test seeds its own temp
/// database and calls this directly, never the user's live graph.
fn memory_status_with_conn(
    conn: &Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let project_filter = params.get("project").and_then(|p| p.as_str());
    // Compact is the default: a session-start answer has to fit into the
    // client's tool-result window, and today's full shape (full notes and
    // every field on every node) measures ~100k characters on the live
    // project — inlined nowhere, read by no one.
    let full = params
        .get("full")
        .and_then(|f| f.as_bool())
        .unwrap_or(false);

    // Auto-index current working directory if not yet indexed
    if let Ok(cwd) = std::env::current_dir() {
        // Opportunistic: a failed auto-index must not fail the status call.
        if let Err(e) = indexer::ensure_indexed(conn, &cwd) {
            tracing::warn!("could not auto-index {}: {e}", cwd.display());
        }
    }

    // US2: pull any pending sync updates for a shared project before reading
    // the graph below, so the response reflects the peer's latest work.
    // Best-effort — never fails memory_status (T022).
    sync_pull_if_enabled(conn, project_filter);

    let projects = graph::search_typed(conn, "*", &NodeType::Project, 10)?;
    let crates = graph::search_typed(conn, "*", &NodeType::Crate, 20)?;
    let mut skills = graph::get_nodes_by_type(conn, &NodeType::Skill)?;
    skills.sort_by_key(|s| std::cmp::Reverse(s.access_count));
    let total_nodes = graph::count_nodes(conn)?;
    let total_edges = graph::count_edges(conn)?;

    if full {
        // The historical selection and shape, unchanged: the same limits, the
        // same fields, the same key order as before compact mode existed.
        let (recent_decisions, problems, recent_solutions, recent_sessions, active_tasks) = (
            graph::typed_in_project(conn, &NodeType::Decision, project_filter, 10)?,
            graph::get_unsolved_problems(conn, project_filter, 10)?,
            graph::typed_in_project(conn, &NodeType::Solution, project_filter, 10)?,
            graph::typed_in_project(conn, &NodeType::Session, project_filter, 5)?,
            graph::get_tasks_filtered(
                conn,
                project_filter,
                Some(graph::OPEN_TASK_STATUSES),
                None,
                10,
            )?,
        );

        let active_tasks_json: Vec<serde_json::Value> = active_tasks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id.to_string(),
                    "label": t.label,
                    "status": t.data.get("status"),
                    "priority": t.data.get("priority"),
                    "note": t.note,
                    "created_at": t.created_at.to_rfc3339(),
                    "created_by": t.created_by,
                    "updated_by": t.updated_by,
                })
            })
            .collect();

        return Ok(json!({
            "server": server_block(),
            "summary": {
                "total_nodes": total_nodes,
                "total_edges": total_edges,
            },
            "project_filter": project_filter,
            "projects": projects.iter().map(node_brief).collect::<Vec<_>>(),
            "crates": crates.iter().map(node_brief).collect::<Vec<_>>(),
            "skills": skills.iter().take(30).map(|n| json!({
                "name": n.label,
                "trigger": n.note,
                "uses": n.access_count,
            })).collect::<Vec<_>>(),
            "active_tasks": active_tasks_json,
            "recent_decisions": recent_decisions.iter().map(node_detail).collect::<Vec<_>>(),
            "open_problems": problems.iter().map(node_detail).collect::<Vec<_>>(),
            "recent_solutions": recent_solutions.iter().map(node_detail).collect::<Vec<_>>(),
            "recent_sessions": recent_sessions.iter().map(node_detail).collect::<Vec<_>>(),
        }));
    }

    // Compact mode: same sections, padded fetches so the number of hidden
    // items is exact, one uniform shrink rule per item (claim whole, note
    // excerpted with `note_truncated`, no boilerplate fields, task evidence
    // summarized), and one honest truncation block.
    let (recent_decisions, problems, recent_solutions, recent_sessions, active_tasks) = (
        graph::typed_in_project(
            conn,
            &NodeType::Decision,
            project_filter,
            COMPACT_FETCH_LIMIT,
        )?,
        graph::get_unsolved_problems(conn, project_filter, COMPACT_FETCH_LIMIT)?,
        graph::typed_in_project(
            conn,
            &NodeType::Solution,
            project_filter,
            COMPACT_FETCH_LIMIT,
        )?,
        graph::typed_in_project(
            conn,
            &NodeType::Session,
            project_filter,
            COMPACT_FETCH_LIMIT,
        )?,
        graph::get_tasks_filtered(
            conn,
            project_filter,
            Some(graph::OPEN_TASK_STATUSES),
            None,
            COMPACT_FETCH_LIMIT,
        )?,
    );

    let hidden_decisions = recent_decisions.len().saturating_sub(COMPACT_LIST_CAP);
    let hidden_problems = problems.len().saturating_sub(COMPACT_LIST_CAP);
    let hidden_solutions = recent_solutions.len().saturating_sub(COMPACT_LIST_CAP);
    let hidden_sessions = recent_sessions.len().saturating_sub(COMPACT_LIST_CAP);
    let hidden_tasks = active_tasks.len().saturating_sub(COMPACT_LIST_CAP);
    let hidden_skills = skills.len().saturating_sub(COMPACT_SKILLS_CAP);

    let active_tasks_json: Vec<serde_json::Value> = active_tasks
        .iter()
        .take(COMPACT_LIST_CAP)
        .map(|t| {
            let (note, note_truncated) = clipped_note(t.note.as_deref());
            let fields = aurelius_core::tasks::TaskFields::from_data(&t.data);
            json!({
                "id": t.id.to_string(),
                "label": t.label,
                "status": t.data.get("status"),
                "priority": t.data.get("priority"),
                "note": note,
                "note_truncated": note_truncated,
                "created_at": t.created_at.to_rfc3339(),
                // The run-by-run journal stays with `task_view`; a status
                // answer needs "is there proof and is it green", which is
                // exactly what the summary says.
                "evidence": aurelius_core::tasks::evidence_summary(&fields),
            })
        })
        .collect();

    let truncation = json!({
        "applied": hidden_tasks + hidden_decisions + hidden_problems + hidden_solutions
            + hidden_sessions + hidden_skills > 0,
        "caps": {
            "active_tasks": COMPACT_LIST_CAP,
            "open_problems": COMPACT_LIST_CAP,
            "recent_decisions": COMPACT_LIST_CAP,
            "recent_solutions": COMPACT_LIST_CAP,
            "recent_sessions": COMPACT_LIST_CAP,
            "skills": COMPACT_SKILLS_CAP,
        },
        "hidden": {
            "active_tasks": hidden_tasks,
            "open_problems": hidden_problems,
            "recent_decisions": hidden_decisions,
            "recent_solutions": hidden_solutions,
            "recent_sessions": hidden_sessions,
            "skills": hidden_skills,
        },
        "how_to_see_more": "task_view with a task id returns its full notes and evidence runs; memory_search(query) finds nodes beyond the cap; `au journal --session <id>` replays a session",
    });

    Ok(json!({
        "server": server_block(),
        "summary": {
            "total_nodes": total_nodes,
            "total_edges": total_edges,
        },
        "project_filter": project_filter,
        "projects": projects.iter().map(node_brief).collect::<Vec<_>>(),
        "crates": crates.iter().map(node_brief).collect::<Vec<_>>(),
        "skills": skills.iter().take(COMPACT_SKILLS_CAP).map(|n| {
            let (trigger, note_truncated) = clipped_note(n.note.as_deref());
            json!({
                "name": n.label,
                "trigger": trigger,
                "uses": n.access_count,
                "note_truncated": note_truncated,
            })
        }).collect::<Vec<_>>(),
        "active_tasks": active_tasks_json,
        "recent_decisions": recent_decisions.iter().take(COMPACT_LIST_CAP).map(compact_node_json).collect::<Vec<_>>(),
        "open_problems": problems.iter().take(COMPACT_LIST_CAP).map(compact_node_json).collect::<Vec<_>>(),
        "recent_solutions": recent_solutions.iter().take(COMPACT_LIST_CAP).map(compact_node_json).collect::<Vec<_>>(),
        "recent_sessions": recent_sessions.iter().take(COMPACT_LIST_CAP).map(compact_node_json).collect::<Vec<_>>(),
        "truncation": truncation,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelius_core::db;
    use serde_json::json;

    /// Same pattern as the task-handler tests: a real temp file, not
    /// `:memory:` — `db::open` requires WAL. Never the user's live database.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-mcp-status-test-{tag}-{}.db",
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

    fn setup() -> (TmpDb, Connection) {
        let tmp = TmpDb::new("setup");
        let conn = db::open(&tmp.0).expect("open temp db");
        // `memory_status` auto-indexes the current directory; the cwd during
        // `cargo test` is this crate (named "aurelius"), and a project node
        // with that exact label makes `ensure_indexed` short-circuit instead
        // of indexing the whole repository into the fixture database.
        graph::add_node(
            &conn,
            NodeType::Project,
            "aurelius",
            None,
            "test",
            json!({}),
        )
        .expect("seed project node");
        (tmp, conn)
    }

    const LONG_NOTE: &str =
        "a very long note that must be clipped in compact mode and returned whole in full mode, \
         padded past the two hundred character budget by this repeated tail:";

    fn seed_fixture(conn: &Connection) {
        let long = format!("{LONG_NOTE} {}", "tail ".repeat(40));
        graph::add_node(
            conn,
            NodeType::Decision,
            "[aurelius] test decision with a long note",
            Some(&long),
            "test",
            json!({}),
        )
        .expect("seed decision");
        graph::add_node_full(
            conn,
            NodeType::Task,
            "[aurelius] test active task with evidence",
            Some(&long),
            "test",
            json!({
                "status": "active",
                "priority": "high",
                "project": "aurelius",
                "evidence": [
                    {"command": "cargo fmt --all -- --check", "exit_code": 1, "at": "2026-09-07T09:00:00Z"},
                    {"command": "cargo clippy --workspace", "exit_code": 0, "at": "2026-09-07T09:10:00Z"},
                    {"command": "cargo test --workspace", "exit_code": 0, "at": "2026-09-07T09:20:00Z"},
                ],
            }),
            aurelius_core::models::MemoryKind::Semantic,
            None,
        )
        .expect("seed task");
    }

    #[test]
    fn compact_clips_notes_summarizes_evidence_and_reports_truncation() {
        let (_tmp, conn) = setup();
        seed_fixture(&conn);

        let status = memory_status_with_conn(&conn, &json!({})).expect("compact memory_status");

        // (a) note excerpted at a word boundary near 200 chars, flagged.
        let decision = &status["recent_decisions"][0];
        let note = decision["note"].as_str().expect("note is a string");
        assert!(
            note.chars().count() <= 220,
            "compact note must be an excerpt, got {} chars",
            note.chars().count()
        );
        assert!(note.ends_with('…'), "excerpt must end with the ellipsis");
        assert_eq!(decision["note_truncated"], json!(true));
        // Boilerplate fields are gone from compact items, `data` included:
        // every fact it carried is already surfaced as claim/provenance.
        assert!(decision.get("created_by").is_none());
        assert!(decision.get("source").is_none());
        assert!(decision.get("memory_kind").is_none());
        assert!(decision.get("data").is_none());
        let prov = decision["provenance"]
            .as_object()
            .expect("provenance object");
        assert!(prov
            .keys()
            .all(|k| ["confidence", "subject", "stale"].contains(&k.as_str())));
        // The claim itself is never clipped.
        assert!(decision["claim"].is_null());

        // (b) task evidence is the summary object, not the run array.
        let task = &status["active_tasks"][0];
        let evidence = &task["evidence"];
        assert!(
            evidence.is_object(),
            "evidence must be a summary: {evidence:?}"
        );
        assert_eq!(evidence["total"], json!(3));
        assert_eq!(evidence["green"], json!(2));
        assert_eq!(
            evidence["last_green"]["command"],
            json!("cargo test --workspace")
        );
        assert!(evidence["last_green"].get("artifact").is_none());
        // Boilerplate gone here too, excerpt flagged.
        assert!(task.get("created_by").is_none());
        assert_eq!(task["note_truncated"], json!(true));
        // Skills carry the same clip flag.
        assert!(status["skills"]
            .as_array()
            .expect("skills")
            .iter()
            .all(|s| { s.get("note_truncated").is_some() }));

        // (d) the truncation block is present in compact mode.
        let truncation = &status["truncation"];
        assert!(
            truncation.is_object(),
            "compact must carry a truncation block"
        );
        assert!(truncation["hidden"].is_object());
        assert_eq!(truncation["hidden"]["active_tasks"], json!(0));
    }

    #[test]
    fn full_returns_unclipped_note_and_todays_exact_shape() {
        let (_tmp, conn) = setup();
        seed_fixture(&conn);
        let long = format!("{LONG_NOTE} {}", "tail ".repeat(40));

        let status =
            memory_status_with_conn(&conn, &json!({"full": true})).expect("full memory_status");

        // (c) notes come back whole, no truncation block, no summary.
        let decision = &status["recent_decisions"][0];
        assert_eq!(decision["note"], json!(long));
        assert!(decision.get("note_truncated").is_none());
        // node_detail carries the raw data object and the verbose provenance.
        assert!(decision.get("data").is_some());
        assert!(decision["provenance"].get("evidence").is_some());
        assert!(status.get("truncation").is_none());

        let task = &status["active_tasks"][0];
        assert_eq!(task["note"], json!(long));
        // Today's shape: created_by/updated_by present, no evidence summary.
        assert!(task.get("created_by").is_some());
        assert!(task.get("updated_by").is_some());
        assert!(task.get("evidence").is_none());
    }

    #[test]
    fn compact_reports_the_exact_number_of_hidden_items() {
        let (_tmp, conn) = setup();
        for i in 0..12 {
            graph::add_node(
                &conn,
                NodeType::Decision,
                &format!("[aurelius] decision number {i:02}"),
                Some("short note"),
                "test",
                json!({}),
            )
            .expect("seed decision");
        }

        let status = memory_status_with_conn(&conn, &json!({})).expect("compact memory_status");

        let decisions = status["recent_decisions"].as_array().expect("decisions");
        assert_eq!(decisions.len(), COMPACT_LIST_CAP);
        assert_eq!(status["truncation"]["hidden"]["recent_decisions"], json!(2));
        assert_eq!(status["truncation"]["applied"], json!(true));
    }

    #[test]
    fn compact_payload_stays_bounded_on_a_seeded_fixture() {
        let (_tmp, conn) = setup();
        seed_fixture(&conn);

        let status = memory_status_with_conn(&conn, &json!({})).expect("compact memory_status");
        let serialized = serde_json::to_string(&status).expect("serialize");

        assert!(
            serialized.len() <= 10_000,
            "compact status ballooned: {} bytes on a tiny fixture",
            serialized.len()
        );
    }
}
