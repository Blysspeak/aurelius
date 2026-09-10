//! MCP surface over `aurelius_core::reminders` — pure request/response reads
//! and writes against the same `reminders`/`reminder_events` tables `au
//! remind` and the session-delivery hook already use. Nothing in this file
//! owns time.
//!
//! Hard rule, decided 2026-09-10 (`aurelius:reminders:clock-owner`): the MCP
//! server is a process spawned once per Claude Code session, so anything
//! periodic living inside it runs once per open session, not once total —
//! open three sessions and it fires three times. This file MUST NOT start a
//! timer, a thread, `tokio::spawn`, an `interval`, or any other background
//! task. If a tool below ever seems to need one — "wake up on its own and
//! check what's due" — the answer is the always-on daemon (wave 2's `au
//! daemon`), which is not part of this file set. Every tool here reads or
//! writes exactly once per call and returns.

use anyhow::Result;
use aurelius_core::reminders;
use chrono::Utc;
use rusqlite::Connection;
use serde_json::json;

use super::{open_db, resolve_task_node};

/// One reminder by full id or unique prefix. `reminders`'s public surface
/// deliberately has no "get one" query of its own (see the doc comment on
/// `resolve_id` in `aurelius-core`) — `au remind show`/`done`/`snooze`
/// resolve the same way, via a scan over `list`, and this mirrors that
/// rather than inventing a second lookup shape.
fn resolve_reminder(conn: &Connection, prefix: &str) -> Result<reminders::Reminder> {
    let id = reminders::resolve_id(conn, prefix)?
        .ok_or_else(|| anyhow::anyhow!("reminder not found or ambiguous prefix: {prefix}"))?;
    reminders::list(conn, None, None, true, usize::MAX)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| anyhow::anyhow!("reminder vanished: {id}"))
}

/// `reminder` serialized plus whatever extra top-level keys the caller adds
/// (`created`, `done`, ...) — one place so every tool below reports the same
/// field set for "here is a reminder" instead of five hand-picked subsets.
fn reminder_json(r: &reminders::Reminder) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(r)?)
}

pub fn reminder_add(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    reminder_add_with_conn(&conn, params)
}

/// Body of `reminder_add`, connection explicit — same testability trick as
/// `task_create_with_conn`.
fn reminder_add_with_conn(
    conn: &Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let text = params
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'text' parameter"))?;

    let now = Utc::now();
    let due_in = params.get("due_in").and_then(|v| v.as_str());
    let due_at = params.get("due_at").and_then(|v| v.as_str());
    // Both or neither is a call error naming which — not a silent pick of
    // one, the same shape `au remind add`'s `one_moment` uses for `--at`/`--in`.
    let due_at = match (due_in, due_at) {
        (Some(_), Some(_)) => {
            anyhow::bail!("reminder_add: due_in and due_at both given — pass exactly one")
        }
        (None, None) => {
            anyhow::bail!("reminder_add: neither due_in nor due_at given — pass exactly one")
        }
        (Some(delay), None) => {
            now + reminders::parse_delay(delay)
                .ok_or_else(|| anyhow::anyhow!("reminder_add: due_in: could not parse '{delay}'"))?
        }
        (None, Some(spec)) => reminders::parse_moment(spec, now)
            .ok_or_else(|| anyhow::anyhow!("reminder_add: due_at: could not parse '{spec}'"))?,
    };

    let owner = match params.get("owner").and_then(|o| o.as_str()) {
        Some(o) => reminders::Owner::parse(o)
            .ok_or_else(|| anyhow::anyhow!("reminder_add: unknown owner '{o}' (me | ai | both)"))?,
        None => reminders::Owner::Both,
    };

    let explicit_project = params
        .get("project")
        .and_then(|p| p.as_str())
        .map(str::to_owned);

    // `task` is resolved the same id-or-label way `task_update`/`task_log`
    // do (`resolve_task_node`). Attached to a task, the reminder inherits
    // ITS project unless the caller names one explicitly.
    let (task_id, project) = match params.get("task").and_then(|t| t.as_str()) {
        Some(t) => {
            let node = resolve_task_node(conn, t)?;
            let inherited = explicit_project.clone().or_else(|| {
                node.data
                    .get("project")
                    .and_then(|p| p.as_str())
                    .map(str::to_owned)
            });
            (Some(node.id.to_string()), inherited)
        }
        None => (None, explicit_project),
    };

    let repeat_spec = params
        .get("repeat")
        .and_then(|r| r.as_str())
        .map(str::to_owned);

    let reminder = reminders::add(
        conn,
        reminders::NewReminder {
            text: text.to_owned(),
            due_at,
            owner,
            task_id,
            project,
            repeat_spec,
        },
    )?;

    let mut out = reminder_json(&reminder)?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("created".to_owned(), json!(true));
    }
    Ok(out)
}

pub fn reminder_list(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    reminder_list_with_conn(&conn, params)
}

fn reminder_list_with_conn(
    conn: &Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let project = params.get("project").and_then(|p| p.as_str());
    let state_filter = params
        .get("state")
        .and_then(|s| s.as_str())
        .map(|s| {
            reminders::State::parse(s).ok_or_else(|| {
                anyhow::anyhow!(
                    "reminder_list: unknown state '{s}' (pending | delivered | done | cancelled)"
                )
            })
        })
        .transpose()?;
    let include_terminal = params
        .get("include_terminal")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;

    let items = reminders::list(conn, project, state_filter, include_terminal, limit)?;
    Ok(json!({
        "reminders": items,
        "count": items.len(),
    }))
}

pub fn reminder_show(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    reminder_show_with_conn(&conn, params)
}

/// The reminder together with its full journal — the postponement history
/// is the point of this tool, not an afterthought squeezed in beside it:
/// `snooze_count`/`original_due_at` on the reminder say how many times and
/// from when, the journal says exactly when each move happened.
fn reminder_show_with_conn(
    conn: &Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;
    let reminder = resolve_reminder(conn, id)?;
    let journal = reminders::events(conn, &reminder.id)?;

    Ok(json!({
        "reminder": reminder,
        "journal": journal,
        "postponements": {
            "count": reminder.snooze_count,
            "original_due_at": reminder.original_due_at.to_rfc3339(),
        },
    }))
}

pub fn reminder_done(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    reminder_done_with_conn(&conn, params)
}

fn reminder_done_with_conn(
    conn: &Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;
    let full_id = reminders::resolve_id(conn, id)?
        .ok_or_else(|| anyhow::anyhow!("reminder not found or ambiguous prefix: {id}"))?;
    if !reminders::done(conn, &full_id, Utc::now())? {
        anyhow::bail!("reminder is already done or cancelled: {full_id}");
    }
    let reminder = resolve_reminder(conn, &full_id)?;
    let mut out = reminder_json(&reminder)?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("done".to_owned(), json!(true));
    }
    Ok(out)
}

pub fn reminder_snooze(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    reminder_snooze_with_conn(&conn, params)
}

fn reminder_snooze_with_conn(
    conn: &Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let id = params
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

    let now = Utc::now();
    let in_ = params.get("in").and_then(|v| v.as_str());
    let at = params.get("at").and_then(|v| v.as_str());
    let until = match (in_, at) {
        (Some(_), Some(_)) => {
            anyhow::bail!("reminder_snooze: in and at both given — pass exactly one")
        }
        (None, None) => {
            anyhow::bail!("reminder_snooze: neither in nor at given — pass exactly one")
        }
        (Some(delay), None) => {
            now + reminders::parse_delay(delay)
                .ok_or_else(|| anyhow::anyhow!("reminder_snooze: in: could not parse '{delay}'"))?
        }
        (None, Some(spec)) => reminders::parse_moment(spec, now)
            .ok_or_else(|| anyhow::anyhow!("reminder_snooze: at: could not parse '{spec}'"))?,
    };

    let full_id = reminders::resolve_id(conn, id)?
        .ok_or_else(|| anyhow::anyhow!("reminder not found or ambiguous prefix: {id}"))?;
    if !reminders::snooze(conn, &full_id, until, now)? {
        anyhow::bail!("reminder cannot be snoozed (already done or cancelled): {full_id}");
    }
    let reminder = resolve_reminder(conn, &full_id)?;
    let mut out = reminder_json(&reminder)?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("snoozed".to_owned(), json!(true));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-rem-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        aurelius_core::db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn add_requires_exactly_one_of_due_in_or_due_at() {
        let conn = test_conn();

        let neither = reminder_add_with_conn(&conn, &json!({"text": "x"}))
            .expect_err("neither due_in nor due_at must fail");
        assert!(
            format!("{neither}").contains("neither"),
            "message must name which: {neither}"
        );

        let both = reminder_add_with_conn(
            &conn,
            &json!({"text": "x", "due_in": "1h", "due_at": "2030-01-01"}),
        )
        .expect_err("both due_in and due_at must fail");
        assert!(
            format!("{both}").contains("both"),
            "message must name which: {both}"
        );
    }

    #[test]
    fn add_defaults_owner_to_both_and_round_trips_state() {
        let conn = test_conn();
        let result = reminder_add_with_conn(&conn, &json!({"text": "call back", "due_in": "1h"}))
            .expect("reminder_add");
        assert_eq!(result["owner"], json!("both"));
        assert_eq!(result["state"], json!("pending"));
        assert_eq!(result["created"], json!(true));
    }

    #[test]
    fn add_rejects_unknown_owner() {
        let conn = test_conn();
        let err = reminder_add_with_conn(
            &conn,
            &json!({"text": "x", "due_in": "1h", "owner": "someone"}),
        )
        .expect_err("unknown owner must fail");
        assert!(format!("{err}").contains("someone"));
    }

    #[test]
    fn show_returns_the_reminder_with_its_journal() {
        let conn = test_conn();
        let added = reminder_add_with_conn(&conn, &json!({"text": "x", "due_in": "1h"}))
            .expect("reminder_add");
        let id = added["id"].as_str().expect("id").to_owned();

        reminder_snooze_with_conn(&conn, &json!({"id": id, "in": "2h"})).expect("snooze");

        let shown = reminder_show_with_conn(&conn, &json!({"id": id})).expect("reminder_show");
        assert_eq!(shown["postponements"]["count"], json!(1));
        let journal = shown["journal"].as_array().expect("journal array");
        assert!(journal.iter().any(|e| e["kind"] == json!("created")));
        assert!(journal.iter().any(|e| e["kind"] == json!("snoozed")));
    }

    #[test]
    fn done_on_an_already_done_reminder_is_refused() {
        let conn = test_conn();
        let added = reminder_add_with_conn(&conn, &json!({"text": "x", "due_in": "1h"}))
            .expect("reminder_add");
        let id = added["id"].as_str().expect("id").to_owned();

        reminder_done_with_conn(&conn, &json!({"id": id})).expect("first done");
        let err = reminder_done_with_conn(&conn, &json!({"id": id}))
            .expect_err("second done must be refused");
        assert!(format!("{err}").contains("already done"));
    }

    #[test]
    fn snooze_requires_exactly_one_of_in_or_at() {
        let conn = test_conn();
        let added = reminder_add_with_conn(&conn, &json!({"text": "x", "due_in": "1h"}))
            .expect("reminder_add");
        let id = added["id"].as_str().expect("id").to_owned();

        let neither = reminder_snooze_with_conn(&conn, &json!({"id": id}))
            .expect_err("neither in nor at must fail");
        assert!(format!("{neither}").contains("neither"));

        let both =
            reminder_snooze_with_conn(&conn, &json!({"id": id, "in": "1h", "at": "2030-01-01"}))
                .expect_err("both in and at must fail");
        assert!(format!("{both}").contains("both"));
    }

    /// Fixture task written directly into the test connection — not through
    /// `task_create`, which opens the real `AURELIUS_HOME` via `open_db()`
    /// and would make this test depend on (and pollute) a live database.
    fn make_task(conn: &Connection, project: &str) -> aurelius_core::models::Node {
        aurelius_core::graph::add_node(
            conn,
            aurelius_core::models::NodeType::Task,
            &format!("[{project}] test task"),
            None,
            "test",
            json!({"status": "backlog", "project": project}),
        )
        .expect("add fixture task node")
    }

    #[test]
    fn add_with_task_inherits_its_project() {
        let conn = test_conn();
        let task = make_task(&conn, "aurelius");

        let result = reminder_add_with_conn(
            &conn,
            &json!({"text": "follow up", "due_in": "1h", "task": task.id.to_string()}),
        )
        .expect("reminder_add with task");
        assert_eq!(result["project"], json!("aurelius"));
        assert_eq!(result["task_id"], json!(task.id.to_string()));
    }

    #[test]
    fn add_with_task_and_explicit_project_keeps_the_explicit_one() {
        let conn = test_conn();
        let task = make_task(&conn, "aurelius");

        let result = reminder_add_with_conn(
            &conn,
            &json!({
                "text": "follow up",
                "due_in": "1h",
                "task": task.id.to_string(),
                "project": "other-project"
            }),
        )
        .expect("reminder_add with task and explicit project");
        assert_eq!(result["project"], json!("other-project"));
    }
}
