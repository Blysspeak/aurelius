//! `au hint --hook` — PreToolUse hook on Edit/Write/MultiEdit/NotebookEdit.
//! On the first edit of a file in a session it looks up at most three live
//! decision/problem/solution nodes that mention the file's repository path
//! and hands them to the model as `additionalContext`. An event-driven
//! insertion, not a per-turn one (decision 2026-09-24: per-turn background
//! thoughts were rejected). Never blocks the edit: any failure prints
//! nothing and the process exits 0.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use serde_json::Value;

use crate::hooks;

const SEEN_DIR: &str = "hint-seen";
const SEEN_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_NODES: usize = 3;

/// One node found for the hint.
#[derive(Debug, Clone, PartialEq)]
pub struct HintNode {
    pub id: String,
    pub node_type: String,
    pub date: String,
    pub text: String,
}

/// The edited file's path for the tools this hook handles, `None` otherwise.
pub fn edited_path(payload: &Value) -> Option<PathBuf> {
    let field = match payload.get("tool_name")?.as_str()? {
        "Edit" | "Write" | "MultiEdit" => "file_path",
        "NotebookEdit" => "notebook_path",
        _ => return None,
    };
    payload
        .get("tool_input")?
        .get(field)?
        .as_str()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// Repository-relative path with `/` separators, or `None` when the file
/// is outside `root` or under a skipped directory.
pub fn relative_path(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<_>>()?;
    if parts.is_empty() {
        return None;
    }
    let dirs = &parts[..parts.len() - 1];
    if dirs
        .iter()
        .any(|d| matches!(*d, "target" | "node_modules" | ".git"))
    {
        return None;
    }
    Some(parts.join("/"))
}

/// `git rev-parse --show-toplevel` from the file's directory (the file may
/// not exist yet for Write, so walk up to the first existing ancestor).
fn repo_root(file: &Path) -> Option<PathBuf> {
    let dir = file.ancestors().skip(1).find(|d| d.is_dir())?;
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    if root.is_empty() {
        return None;
    }
    // Canonicalize both sides so symlinked paths still strip cleanly.
    std::fs::canonicalize(root).ok()
}

/// Session id reduced to a safe file name.
fn session_file(dir: &Path, session: &str) -> Option<PathBuf> {
    let safe: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe.is_empty() {
        return None;
    }
    Some(dir.join(format!("{safe}.txt")))
}

/// Records `rel` for the session; `true` when it was not seen before.
pub fn mark_seen(dir: &Path, session: &str, rel: &str) -> std::io::Result<bool> {
    use std::io::Write;
    let Some(file) = session_file(dir, session) else {
        return Ok(false);
    };
    std::fs::create_dir_all(dir)?;
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    if existing.lines().any(|l| l == rel) {
        return Ok(false);
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)?;
    writeln!(f, "{rel}")?;
    Ok(true)
}

/// Deletes session files older than seven days.
pub fn prune_seen(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > SEEN_MAX_AGE);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Read-only lookup: live decision/problem/solution nodes whose label, note
/// or data.claim contains `rel`, scoped to `project` the way
/// `graph::project_scope_sql` scopes (label prefix, the project node itself,
/// or an edge to it in either direction), newest `created_at` first.
pub fn lookup(
    conn: &Connection,
    rel: &str,
    project: Option<&str>,
) -> rusqlite::Result<Vec<HintNode>> {
    let scope = if project.is_some() {
        format!(" AND {}", aurelius_core::graph::project_scope_sql("n", 2))
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT n.id, n.node_type, n.created_at, n.label, json_extract(n.data, '$.claim')
           FROM nodes n
          WHERE n.deleted_at IS NULL
            AND n.node_type IN ('\"decision\"', '\"problem\"', '\"solution\"',
                                'decision', 'problem', 'solution')
            AND (instr(n.label, ?1) > 0
                 OR instr(COALESCE(n.note, ''), ?1) > 0
                 OR instr(COALESCE(json_extract(n.data, '$.claim'), ''), ?1) > 0){scope}
          ORDER BY n.created_at DESC
          LIMIT {MAX_NODES}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        let node_type: String = row.get(1)?;
        let created: String = row.get(2)?;
        let label: String = row.get(3)?;
        let claim: Option<String> = row.get(4).ok().flatten();
        Ok(HintNode {
            id: row.get(0)?,
            node_type: node_type.trim_matches('"').to_owned(),
            date: created.chars().take(10).collect(),
            text: claim.filter(|c| !c.trim().is_empty()).unwrap_or(label),
        })
    };
    let rows = match project {
        Some(p) => stmt.query_map(rusqlite::params![rel, p], map)?,
        None => stmt.query_map(rusqlite::params![rel], map)?,
    };
    rows.collect()
}

/// The hook's stdout: one JSON object with `additionalContext`, or `None`
/// when nothing was found.
pub fn render(rel: &str, nodes: &[HintNode]) -> Option<String> {
    if nodes.is_empty() {
        return None;
    }
    let mut text = format!("aurelius: в памяти есть записи про {rel}:");
    for n in nodes {
        let short: String = n.id.chars().take(8).collect();
        text.push_str(&format!(
            "\n- [{} {}] {} ({short})",
            n.node_type, n.date, n.text
        ));
    }
    Some(
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": text,
            }
        })
        .to_string(),
    )
}

/// `au hint --hook`.
pub fn hint_hook() {
    let Some(payload) = hooks::read_payload() else {
        hooks::debug("hint", "no JSON payload on stdin");
        return;
    };
    let Some(file) = edited_path(&payload) else {
        return;
    };
    let file = if file.is_absolute() {
        file
    } else {
        match hooks::cwd_of(&payload) {
            Some(cwd) => cwd.join(file),
            None => return,
        }
    };
    let Some(root) = repo_root(&file) else {
        return;
    };
    // Canonicalize the parent (the file itself may not exist yet).
    let file = match (
        file.parent().and_then(|p| std::fs::canonicalize(p).ok()),
        file.file_name(),
    ) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => return,
    };
    let Some(rel) = relative_path(&root, &file) else {
        return;
    };
    let Some(session) = payload.get("session_id").and_then(Value::as_str) else {
        return;
    };

    let db = aurelius_core::db::db_path();
    let Some(data_dir) = db.parent() else {
        return;
    };
    let seen_dir = data_dir.join(SEEN_DIR);
    prune_seen(&seen_dir, SystemTime::now());
    match mark_seen(&seen_dir, session, &rel) {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            hooks::debug("hint", &format!("hint-seen: {e}"));
            return;
        }
    }

    let conn = match aurelius_core::db::open(&db) {
        Ok(conn) => conn,
        Err(e) => {
            hooks::debug("hint", &format!("{e}"));
            return;
        }
    };
    let project = hooks::hook_project(Some(&payload));
    match lookup(&conn, &rel, project.as_deref()) {
        Ok(nodes) => {
            if let Some(out) = render(&rel, &nodes) {
                println!("{out}");
            }
        }
        Err(e) => hooks::debug("hint", &format!("lookup: {e}")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use aurelius_core::models::{NodeType, Relation};
    use serde_json::json;

    #[test]
    fn path_per_tool() {
        for tool in ["Edit", "Write", "MultiEdit"] {
            let p = json!({"tool_name": tool, "tool_input": {"file_path": "/a/b.rs"}});
            assert_eq!(edited_path(&p), Some(PathBuf::from("/a/b.rs")));
        }
        let p = json!({"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "/a/n.ipynb", "file_path": "/x"}});
        assert_eq!(edited_path(&p), Some(PathBuf::from("/a/n.ipynb")));
        let p = json!({"tool_name": "Bash", "tool_input": {"file_path": "/a/b.rs"}});
        assert_eq!(edited_path(&p), None);
        let p = json!({"tool_name": "Edit", "tool_input": {}});
        assert_eq!(edited_path(&p), None);
    }

    #[test]
    fn skip_rules() {
        let root = Path::new("/repo");
        assert_eq!(
            relative_path(root, Path::new("/repo/crates/x/src/db.rs")).as_deref(),
            Some("crates/x/src/db.rs")
        );
        assert_eq!(relative_path(root, Path::new("/other/db.rs")), None);
        assert_eq!(
            relative_path(root, Path::new("/repo/target/debug/x.rs")),
            None
        );
        assert_eq!(
            relative_path(root, Path::new("/repo/ui/node_modules/a.js")),
            None
        );
        assert_eq!(relative_path(root, Path::new("/repo/.git/config")), None);
        // A file merely named like a skipped dir is not skipped.
        assert_eq!(
            relative_path(root, Path::new("/repo/target")).as_deref(),
            Some("target")
        );
        // Outside any git repository: no root at all.
        assert_eq!(repo_root(Path::new("/definitely/not/here/x.rs")), None);
    }

    #[test]
    fn once_per_session_and_prune() {
        let dir = std::env::temp_dir().join(format!("hint-seen-{}", uuid::Uuid::new_v4()));
        assert!(mark_seen(&dir, "s1", "a.rs").unwrap());
        assert!(!mark_seen(&dir, "s1", "a.rs").unwrap());
        assert!(mark_seen(&dir, "s1", "b.rs").unwrap());
        assert!(mark_seen(&dir, "s2", "a.rs").unwrap());
        prune_seen(&dir, SystemTime::now());
        assert!(dir.join("s1.txt").exists());
        prune_seen(&dir, SystemTime::now() + Duration::from_secs(8 * 24 * 3600));
        assert!(!dir.join("s1.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lookup_types_project_and_limit() {
        let path = std::env::temp_dir().join(format!("hint-{}.db", uuid::Uuid::new_v4()));
        let conn = aurelius_core::db::open(&path).unwrap();
        use aurelius_core::graph::{add_edge, add_node};
        let proj = add_node(
            &conn,
            NodeType::Project,
            "aurelius",
            None,
            "manual",
            json!({}),
        )
        .unwrap();
        let other = add_node(&conn, NodeType::Project, "other", None, "manual", json!({})).unwrap();
        let rel = "crates/x/src/db.rs";
        for i in 0..4 {
            let n = add_node(
                &conn,
                NodeType::Decision,
                &format!("d{i}"),
                Some(&format!("see {rel}")),
                "manual",
                json!({}),
            )
            .unwrap();
            add_edge(&conn, n.id, proj.id, Relation::Uses, 1.0).unwrap();
        }
        let wrong_type = add_node(
            &conn,
            NodeType::Concept,
            "c",
            Some(rel),
            "manual",
            json!({}),
        )
        .unwrap();
        add_edge(&conn, wrong_type.id, proj.id, Relation::Uses, 1.0).unwrap();
        let foreign = add_node(
            &conn,
            NodeType::Problem,
            "p",
            None,
            "manual",
            json!({"claim": rel}),
        )
        .unwrap();
        add_edge(&conn, foreign.id, other.id, Relation::Uses, 1.0).unwrap();
        let prefixed = add_node(
            &conn,
            NodeType::Solution,
            "[aurelius] s",
            None,
            "manual",
            json!({"claim": format!("fix in {rel}")}),
        )
        .unwrap();

        let found = lookup(&conn, rel, Some("aurelius")).unwrap();
        assert_eq!(found.len(), 3);
        assert!(found
            .iter()
            .all(|n| n.node_type == "decision" || n.node_type == "solution"));
        assert!(found
            .iter()
            .all(|n| n.id != foreign.id.to_string() && n.id != wrong_type.id.to_string()));
        let all = lookup(&conn, rel, Some("aurelius")).unwrap();
        assert_eq!(all, found);
        let only = lookup(&conn, rel, Some("other")).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].node_type, "problem");
        assert_eq!(only[0].text, rel);
        let _ = prefixed;
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn output_shape() {
        assert_eq!(render("a.rs", &[]), None);
        let out = render(
            "a.rs",
            &[HintNode {
                id: "0123456789abcdef".into(),
                node_type: "decision".into(),
                date: "2026-09-25".into(),
                text: "claim".into(),
            }],
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert!(v["hookSpecificOutput"].get("permissionDecision").is_none());
        assert_eq!(
            v["hookSpecificOutput"]["additionalContext"],
            "aurelius: в памяти есть записи про a.rs:\n- [decision 2026-09-25] claim (01234567)"
        );
    }
}
