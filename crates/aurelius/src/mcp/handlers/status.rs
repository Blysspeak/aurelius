use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use aurelius_core::{graph, indexer, models::NodeType};
use rusqlite::Connection;
use serde_json::json;

use super::{
    apply_probe_stale, db_path, node_brief, node_detail, open_db, probe_stale_notes,
    query_vector_for_topic, restart_needed, server_started_at, sync_pull_if_enabled,
};

/// `server` block of `memory_status`: which MCP server process answered, and
/// whether the binary on disk has moved past it. `restart_needed`/`hint` are
/// omitted rather than `null` when the check couldn't run (see
/// `super::restart_needed`), so their absence reads as "unknown", not "no".
fn server_block() -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    fields.insert("version".to_owned(), json!(crate::BUILD_VERSION));
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

/// How many items each list section shows in compact mode. `full=true`
/// keeps the historical selection instead (5 sessions, 30 skills). Five, not
/// ten: the SessionStart snapshot already orients the agent, and a compact
/// status is the "a bit more" view, not the whole picture.
const COMPACT_LIST_CAP: usize = 5;

/// Compact mode fetches with this padded limit and caps in memory, so the
/// truncation block can report the exact number of hidden items (the same
/// trick `snapshot::gather` uses for active tasks) rather than "at least one
/// more". A section past this size gets an undercounted hidden number — by
/// then the graph itself is the problem, not the report.
const COMPACT_FETCH_LIMIT: usize = 1000;

/// One node in compact form: id, type, date, the claim when present or else
/// the label, and provenance reduced to `confidence` (always — silence about
/// provenance reads as measured) plus `stale` only when there is a warning.
/// No note excerpt, no access count: the text is one line, the detail lives
/// in `memory_search` / `task_view` / `full=true`.
fn compact_node_json(node: &aurelius_core::models::Node) -> serde_json::Value {
    let p = aurelius_core::provenance::Provenance::from_data(&node.data);
    let text = p.claim.clone().unwrap_or_else(|| node.label.clone());
    json!({
        "id": node.id.to_string(),
        "type": node.node_type,
        "date": node.created_at.format("%Y-%m-%d").to_string(),
        "text": text,
        "provenance": {
            "confidence": p.confidence_or_default().as_str(),
            "stale": p.staleness(node.created_at, chrono::Utc::now()).map(|s| s.note()),
        },
    })
}

/// Drops `provenance.stale` when it is null. Runs after `apply_probe_stale`,
/// which needs the slot present to write a probe note into it.
fn drop_null_stale(item: &mut serde_json::Value) {
    if let Some(prov) = item.get_mut("provenance").and_then(|p| p.as_object_mut()) {
        if prov.get("stale").is_some_and(|s| s.is_null()) {
            prov.remove("stale");
        }
    }
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

    let mut projects = graph::search_typed(conn, "*", &NodeType::Project, 10)?;
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

        // One probe read for every node this answer renders.
        let rendered: Vec<&aurelius_core::models::Node> = recent_decisions
            .iter()
            .chain(&problems)
            .chain(&recent_solutions)
            .chain(&recent_sessions)
            .collect();
        let probe_notes = probe_stale_notes(conn, &rendered);
        let detail = |n: &aurelius_core::models::Node| {
            let mut v = node_detail(n);
            apply_probe_stale(&mut v, "/provenance/stale", &probe_notes);
            v
        };

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
            "recent_decisions": recent_decisions.iter().map(detail).collect::<Vec<_>>(),
            "open_problems": problems.iter().map(detail).collect::<Vec<_>>(),
            "recent_solutions": recent_solutions.iter().map(detail).collect::<Vec<_>>(),
            "recent_sessions": recent_sessions.iter().map(detail).collect::<Vec<_>>(),
        }));
    }

    // Compact mode: same sections, padded fetches so the number of hidden
    // items is exact, one uniform shrink rule per item (id, type, date,
    // claim or label, provenance signal; task evidence summarized), and one honest truncation block.
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
    let hidden_skills = skills.len().saturating_sub(COMPACT_LIST_CAP);

    // With a project filter, other projects are noise in the answer.
    // Looked up by label, not filtered out of the top-10 list above: the
    // filtered project need not be among the ten.
    if let Some(p) = project_filter {
        projects = graph::find_project_by_label(conn, p)?.into_iter().collect();
    }

    let active_tasks_json: Vec<serde_json::Value> = active_tasks
        .iter()
        .take(COMPACT_LIST_CAP)
        .map(|t| {
            let fields = aurelius_core::tasks::TaskFields::from_data(&t.data);
            let mut v = compact_node_json(t);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("status".to_owned(), json!(t.data.get("status")));
                obj.insert("priority".to_owned(), json!(t.data.get("priority")));
                // The run-by-run journal stays with `task_view`; a status
                // answer needs "is there proof and is it green", which is
                // exactly what the summary says.
                // A task without runs carries no evidence object at all.
                let summary = aurelius_core::tasks::evidence_summary(&fields);
                if summary.total > 0 {
                    obj.insert("evidence".to_owned(), json!(summary));
                }
            }
            drop_null_stale(&mut v);
            v
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
            "skills": COMPACT_LIST_CAP,
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

    // One probe read for every node this answer renders.
    let rendered: Vec<&aurelius_core::models::Node> = recent_decisions
        .iter()
        .take(COMPACT_LIST_CAP)
        .chain(problems.iter().take(COMPACT_LIST_CAP))
        .chain(recent_solutions.iter().take(COMPACT_LIST_CAP))
        .chain(recent_sessions.iter().take(COMPACT_LIST_CAP))
        .collect();
    let probe_notes = probe_stale_notes(conn, &rendered);
    let compact = |n: &aurelius_core::models::Node| {
        let mut v = compact_node_json(n);
        apply_probe_stale(&mut v, "/provenance/stale", &probe_notes);
        drop_null_stale(&mut v);
        v
    };

    Ok(json!({
        "server": server_block(),
        "summary": {
            "total_nodes": total_nodes,
            "total_edges": total_edges,
        },
        "project_filter": project_filter,
        "projects": projects.iter().map(node_brief).collect::<Vec<_>>(),
        "crates": crates.iter().map(node_brief).collect::<Vec<_>>(),
        // Names only: the SessionStart hook already delivers the skill index
        // with triggers.
        "skills": skills.iter().take(COMPACT_LIST_CAP).map(|n| n.label.clone()).collect::<Vec<_>>(),
        "active_tasks": active_tasks_json,
        "recent_decisions": recent_decisions.iter().take(COMPACT_LIST_CAP).map(compact).collect::<Vec<_>>(),
        "open_problems": problems.iter().take(COMPACT_LIST_CAP).map(compact).collect::<Vec<_>>(),
        "recent_solutions": recent_solutions.iter().take(COMPACT_LIST_CAP).map(compact).collect::<Vec<_>>(),
        "recent_sessions": recent_sessions.iter().take(COMPACT_LIST_CAP).map(compact).collect::<Vec<_>>(),
        "truncation": truncation,
    }))
}

// ---------------------------------------------------------------------------
// memory_pickup, memory_eval, db_check, db_backup, db_reindex_embeddings —
// MCP doors onto `au pickup`/`au eval`/`au db check`/`au db backup`/
// `au db reindex-embeddings`, reusing the same `aurelius-core` functions the
// CLI calls rather than a second copy of their logic.
// ---------------------------------------------------------------------------

/// `memory_pickup` — MCP door onto `au pickup --project <p> --json`
/// (`graph::build_pickup`, `crates/aurelius-core/src/graph/pickup.rs`). A
/// project is required, same refusal as the CLI without `--project`: pickup
/// anchors on a project's own facets/tail/records, and an anchor without a
/// scope to compute it in is meaningless.
pub fn memory_pickup(params: &serde_json::Value) -> Result<serde_json::Value> {
    let project = params
        .get("project")
        .and_then(|p| p.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("missing 'project' parameter — pickup needs a project to anchor on")
        })?;

    let conn = open_db()?;
    let payload = graph::build_pickup(&conn, project)?;
    Ok(serde_json::to_value(payload)?)
}

/// Resolves a path relative to the nearest ancestor directory holding
/// `.git`, exactly like `au eval`'s own `eval_from_repo_root`
/// (`crates/au/src/commands.rs`): a call from any subdirectory of the repo
/// still finds the one contract file. No repo root found (an archive without
/// `.git`) — falls back to the bare relative path, same as the CLI.
fn eval_repo_root_join(relative: &str) -> PathBuf {
    let mut dir = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(_) => return PathBuf::from(relative),
    };
    loop {
        if dir.join(".git").exists() {
            return dir.join(relative);
        }
        if !dir.pop() {
            return PathBuf::from(relative);
        }
    }
}

/// Query vectors for the `recall_top5`/`crosslingual_top5` topics in `cases`,
/// through the same daemon-socket path every topic-taking tool uses
/// (`query_vector_for_topic`) — not a second model load like the CLI's
/// `--live` path (`eval_query_vectors` in `commands.rs`, which opens its own
/// copy of bge-m3): the daemon is the single owner of the model, and this
/// asks it exactly the way `memory_search`/`memory_recall`/`memory_context`
/// do. Returns the first degradation reason seen, if any, so the caller can
/// report it — never `Err`: a topic that fails to embed just judges without
/// a vector, same as the fixture path does when there is no vector table at
/// all.
fn eval_query_vectors(
    cases: &[aurelius_core::eval::EvalCase],
) -> (HashMap<String, Vec<f32>>, Option<String>) {
    use aurelius_core::eval::CaseBody;
    let mut out = HashMap::new();
    let mut notice = None;
    for case in cases {
        let topic = match &case.body {
            CaseBody::RecallTop5 { input, .. } | CaseBody::CrosslingualTop5 { input, .. } => {
                input.topic.as_str()
            }
            _ => continue,
        };
        if topic.trim().is_empty() || out.contains_key(topic) {
            continue;
        }
        let (vector, reason) = query_vector_for_topic(topic);
        match vector {
            Some(v) => {
                out.insert(topic.to_owned(), v);
            }
            None => {
                if notice.is_none() {
                    notice = reason;
                }
            }
        }
    }
    (out, notice)
}

/// `memory_eval` — MCP door onto `au eval` (`aurelius_core::eval::{load,
/// run}`, the same judge `au eval` calls, not a copy of it — see the module
/// doc on `eval` for why that separation is load-bearing): scores search
/// quality against the frozen fixture named in the cases file's own `meta`,
/// or — `live: true` — against the live database, using `now` (RFC3339) or
/// the system clock instead of `meta.as_of`.
///
/// A live run is never comparable to a frozen one — different data, no fixed
/// digest — and that is marked in the answer itself, not just in prose:
/// `"comparable": false` sits next to `"digest"`, the same field
/// `au eval --json` puts there (`eval_report_json` in `commands.rs`,
/// `"comparable": !head.live`), so a caller reading only the JSON still sees
/// it.
///
/// `cases`/`db` override the cases file and the fixture path, like
/// `au eval [CASES] --db <path>`. A `.zst`-packed fixture is refused rather
/// than silently unsupported: this crate does not depend on `zstd` (the CLI
/// does, only to unpack `au eval`'s archived fixtures) and adding that
/// dependency is outside this handler's reach — today's default fixture is a
/// plain `.db`, so this only bites a caller who deliberately points `db` at
/// an archive.
pub fn memory_eval(params: &serde_json::Value) -> Result<serde_json::Value> {
    let live = params
        .get("live")
        .and_then(|l| l.as_bool())
        .unwrap_or(false);
    let cases_path = match params.get("cases").and_then(|c| c.as_str()) {
        Some(p) => PathBuf::from(p),
        None => eval_repo_root_join("fixtures/eval/cases.jsonl"),
    };
    let (meta, cases) = aurelius_core::eval::load(&cases_path)?;

    let (moment, now_source) = match params.get("now").and_then(|n| n.as_str()) {
        Some(raw) => (
            chrono::DateTime::parse_from_rfc3339(raw)
                .map_err(|e| anyhow::anyhow!("'now' is not RFC3339: {raw} — {e}"))?
                .with_timezone(&chrono::Utc),
            "now",
        ),
        None if live => (chrono::Utc::now(), "system clock (live)"),
        None => (meta.as_of, "meta.as_of"),
    };

    let named = if live {
        db_path()
    } else {
        match params.get("db").and_then(|d| d.as_str()) {
            Some(p) => PathBuf::from(p),
            None if meta.fixture.trim().is_empty() => anyhow::bail!(
                "fixture not named: neither 'db' nor meta.fixture in {} — nothing to guess",
                cases_path.display()
            ),
            None => eval_repo_root_join(meta.fixture.trim()),
        }
    };
    if named.extension().is_some_and(|ext| ext == "zst") {
        anyhow::bail!(
            "{} is a packed .zst fixture — this tool does not unpack it (no zstd dependency \
             here, see the doc comment on memory_eval); run `au eval` from a shell for that one",
            named.display()
        );
    }

    let conn = aurelius_core::db::open_readonly(&named)?;
    let sha256 = if live {
        None
    } else {
        aurelius_core::eval::verify_fixture(&named, &meta.fixture_sha256)?;
        Some(meta.fixture_sha256.clone())
    };

    // Гибридный поиск (спека 011) только под `live: true` — та же причина,
    // что и у `au eval --live`: замороженная фикстура таблицы векторов не
    // несёт, и опрос демона на ней ничего не меняет в ответе.
    let (query_vectors, vector_notice) = if live {
        eval_query_vectors(&cases)
    } else {
        (HashMap::new(), None)
    };

    let report = aurelius_core::eval::run(&conn, &meta, &cases, moment, &query_vectors)?;

    let mut by_kind = serde_json::Map::new();
    for kind in aurelius_core::eval::CaseKind::ALL {
        let tally = report.tally(kind);
        by_kind.insert(
            kind.as_str().to_owned(),
            json!({
                "cases": tally.cases,
                "eligible": tally.eligible(),
                "passed": tally.passed,
                "failed": tally.failed,
                "skipped": tally.skipped,
            }),
        );
    }

    Ok(json!({
        "cases": cases_path.display().to_string(),
        "fixture": named.display().to_string(),
        "fixture_sha256": sha256,
        "as_of": report.now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "now_source": now_source,
        "comparable": !live,
        "total": report.total,
        "passed": report.passed,
        "failed": report.failed,
        "skipped": report.skipped,
        "mrr": report.mrr,
        "by_kind": by_kind,
        "digest": report.digest,
        "vector_notice": vector_notice,
    }))
}

/// `db_check` — MCP door onto `au db check` (`aurelius_core::db::check`):
/// read-only integrity report on the live database file — page geometry, WAL
/// size, node/edge counts, and a `quick_check` (or, `full: true`, a slower
/// whole-file `integrity_check`). Read-only, unlike `db_backup`: `db::check`
/// opens the file with `open_readonly` and never migrates or writes a page,
/// so it never risks the database it is reporting on.
///
/// Reports rather than fails when integrity is broken (`"ok": false` with
/// `"problems"` filled in) — the CLI exits non-zero on the same finding, but
/// an MCP caller is better served reading the report than parsing an error
/// string out of an `isError` result.
pub fn db_check(params: &serde_json::Value) -> Result<serde_json::Value> {
    let full = params
        .get("full")
        .and_then(|f| f.as_bool())
        .unwrap_or(false);
    let path = db_path();
    if !path.exists() {
        anyhow::bail!("no database at {}", path.display());
    }
    let report = aurelius_core::db::check(&path, full)?;
    Ok(json!({
        "path": path.display().to_string(),
        "ok": report.ok,
        "problems": report.problems,
        "page_size": report.page_size,
        "page_count": report.page_count,
        "file_bytes": report.file_bytes,
        "wal_bytes": report.wal_bytes,
        "nodes": report.nodes,
        "edges": report.edges,
        "mode": if full { "integrity_check" } else { "quick_check" },
    }))
}

/// `db_backup` — MCP door onto `au db backup` (`aurelius_core::db::backup_into`,
/// `VACUUM INTO`): writes a new, consistent snapshot file of the live
/// database. Unlike `db_check`, this one writes — a new file on disk, never
/// touching the source. Destination defaults to `aurelius-<timestamp>.db`
/// beside the source, same as the CLI; refuses to overwrite an existing file
/// rather than silently replace a previous backup.
pub fn db_backup(params: &serde_json::Value) -> Result<serde_json::Value> {
    let path = db_path();
    if !path.exists() {
        anyhow::bail!("no database at {}", path.display());
    }
    let dest = match params.get("out").and_then(|o| o.as_str()) {
        Some(p) => PathBuf::from(p),
        None => path.with_file_name(format!(
            "aurelius-{}.db",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        )),
    };
    if dest.exists() {
        anyhow::bail!("destination already exists: {}", dest.display());
    }

    let bytes = aurelius_core::db::backup_into(&path, &dest)?;
    Ok(json!({
        "source": path.display().to_string(),
        "dest": dest.display().to_string(),
        "bytes": bytes,
    }))
}

/// `db_reindex_embeddings` — MCP door onto `au db reindex-embeddings`. No
/// longer computes anything: since the phase-D rewrite (`commands.rs`,
/// `db_reindex_embeddings_cli`'s doc comment) it only finds live nodes
/// missing a vector in `node_embeddings` and queues their ids in
/// `embedding_queue`, plain SQL, no model call — it returns in milliseconds.
/// The daemon drains that queue on its own tick, with the model it already
/// holds (`drain_embedding_queue` in `commands.rs`); this call does not wait
/// for that and does not touch the model itself.
///
/// Same `INSERT ... ON CONFLICT DO NOTHING` as `db_reindex_embeddings_cli`,
/// duplicated rather than shared: this crate cannot depend on the `au`
/// binary crate (the dependency runs the other way — `au` depends on
/// `aurelius`), and the query is short enough that a second copy is the
/// honest choice over inventing a cross-crate export for one seven-line
/// statement.
pub fn db_reindex_embeddings(_params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    let now = chrono::Utc::now().to_rfc3339();
    let queued = conn.execute(
        "INSERT INTO embedding_queue (node_id, queued_at, attempts)
           SELECT n.id, ?1, 0
             FROM nodes n
            WHERE n.deleted_at IS NULL
              AND n.rowid NOT IN (SELECT rowid FROM node_embeddings)
         ON CONFLICT (node_id) DO NOTHING",
        rusqlite::params![now],
    )?;
    Ok(json!({
        "queued": queued,
        "note": "queued only, not computed — vectors are written by the daemon's background \
                 drain on its own tick, using the model it already holds",
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

        // (a) no note excerpt, no boilerplate: the label stands in for the
        // missing claim, provenance is reduced to its signal.
        let decision = &status["recent_decisions"][0];
        assert!(decision.get("note").is_none());
        assert!(decision.get("note_truncated").is_none());
        assert!(decision.get("access_count").is_none());
        assert!(decision.get("data").is_none());
        assert_eq!(
            decision["text"],
            json!("[aurelius] test decision with a long note")
        );
        assert_eq!(decision["date"].as_str().map(str::len), Some(10));
        let prov = decision["provenance"]
            .as_object()
            .expect("provenance object");
        assert!(prov
            .keys()
            .all(|k| ["confidence", "stale"].contains(&k.as_str())));

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
        // Boilerplate gone here too.
        assert!(task.get("created_by").is_none());
        assert!(task.get("note").is_none());
        // Skills are names only.
        assert!(status["skills"]
            .as_array()
            .expect("skills")
            .iter()
            .all(|s| s.is_string()));

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
        for i in 0..7 {
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
    fn compact_with_filter_lists_only_the_filtered_project() {
        let (_tmp, conn) = setup();
        graph::add_node(&conn, NodeType::Project, "other", None, "test", json!({}))
            .expect("seed other project");

        let status = memory_status_with_conn(&conn, &json!({"project": "aurelius"}))
            .expect("compact memory_status");

        let projects = status["projects"].as_array().expect("projects");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["label"], json!("aurelius"));
    }

    #[test]
    fn compact_item_with_claim_has_no_note_excerpt() {
        let (_tmp, conn) = setup();
        graph::add_node(
            &conn,
            NodeType::Decision,
            "[aurelius] decision with a claim",
            Some("a note that must not be echoed next to the claim"),
            "test",
            json!({"claim": "the claim itself"}),
        )
        .expect("seed decision");

        let status = memory_status_with_conn(&conn, &json!({})).expect("compact memory_status");

        let decision = &status["recent_decisions"][0];
        assert_eq!(decision["text"], json!("the claim itself"));
        assert!(decision.get("note").is_none());
        assert!(!serde_json::to_string(decision)
            .expect("serialize")
            .contains("must not be echoed"));
    }

    #[test]
    fn compact_task_without_runs_has_no_evidence_object() {
        let (_tmp, conn) = setup();
        graph::add_node(
            &conn,
            NodeType::Task,
            "[aurelius] task without evidence",
            None,
            "test",
            json!({"status": "active", "project": "aurelius"}),
        )
        .expect("seed task");

        let status = memory_status_with_conn(&conn, &json!({})).expect("compact memory_status");

        let task = &status["active_tasks"][0];
        assert_eq!(task["text"], json!("[aurelius] task without evidence"));
        assert!(task.get("evidence").is_none());
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

    fn seed_probe(conn: &Connection, node_id: &str, expr: &str, ok: bool, at: i64) {
        conn.execute(
            "INSERT INTO probes (node_id, kind, expr, last_ok, checked_at) VALUES (?1, 'file_exists', ?2, ?3, ?4)",
            rusqlite::params![node_id, expr, i64::from(ok), at],
        )
        .expect("seed probe");
    }

    /// An old volatile decision, so the age-based note fires on its own.
    fn seed_aged_decision(conn: &Connection, label: &str) -> String {
        graph::add_node(
            conn,
            NodeType::Decision,
            label,
            None,
            "test",
            json!({"volatility": "volatile", "measured_at": "2026-01-01T00:00:00Z"}),
        )
        .expect("seed decision")
        .id
        .to_string()
    }

    fn stale_of(status: &serde_json::Value, id: &str) -> serde_json::Value {
        status["recent_decisions"]
            .as_array()
            .expect("decisions")
            .iter()
            .find(|d| d["id"] == json!(id))
            .map(|d| d["provenance"]["stale"].clone())
            .expect("decision rendered")
    }

    #[test]
    fn failing_probe_note_outranks_age_note_and_passing_probe_keeps_age() {
        let (_tmp, conn) = setup();
        let failing = seed_aged_decision(&conn, "[aurelius] leans on a gone file");
        let passing = seed_aged_decision(&conn, "[aurelius] leans on a present file");
        // 2026-09-20T00:00:00Z
        seed_probe(&conn, &failing, "/gone/file.rs", false, 1_789_862_400);
        seed_probe(&conn, &passing, "/here/file.rs", true, 1_789_862_400);

        let expected = "проба не прошла 2026-09-20: file_exists /gone/file.rs — перепроверь, прежде чем опираться";
        for full in [false, true] {
            let status =
                memory_status_with_conn(&conn, &json!({"full": full})).expect("memory_status");
            assert_eq!(stale_of(&status, &failing), json!(expected), "full={full}");
            let age = stale_of(&status, &passing);
            let age = age.as_str().expect("age note present");
            assert!(age.starts_with("старше "), "full={full}: {age}");
        }
    }
}
