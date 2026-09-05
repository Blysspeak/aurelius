//! `--hook` support for `au touch`, `au reindex`, `au db backup` — replaces
//! three bash wrappers under `contrib/claude-code/` (`aurelius-track-edit.sh`,
//! `aurelius-reindex.sh`, `aurelius-backup.sh`) that needed `bash` and
//! `python3`, neither of which the owner's Windows machine has. The plugin
//! (spec 009) calls `au` directly instead.
//!
//! Contract: `specs/009-claude-code-plugin/contracts/au-cli-hooks.md`. Common
//! rule for all three: the exit code is always 0 — an internal failure
//! (no database, no permissions, no network, no such file) ends the process
//! silently. Claude Code shows a hook's stderr to the user only on a
//! non-zero exit, so the only way to see a reason is `AURELIUS_HOOK_DEBUG=1`,
//! which prints one line to stderr per failure.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::commands;

/// One Claude Code hook JSON payload from stdin. Empty input or anything
/// that isn't valid JSON is not an error worth reporting — it just means
/// "no payload", the same as an interactive run with nothing piped in.
fn read_payload() -> Option<Value> {
    serde_json::from_reader(std::io::stdin().lock()).ok()
}

/// `tool_input.file_path`, falling back to `tool_input.path` — the two
/// fields `aurelius-track-edit.sh` read from Claude Code's PostToolUse
/// payload.
fn file_path_of(payload: &Value) -> Option<PathBuf> {
    let tool_input = payload.get("tool_input")?;
    tool_input
        .get("file_path")
        .or_else(|| tool_input.get("path"))?
        .as_str()
        .map(PathBuf::from)
}

/// The `cwd` field of a Claude Code hook payload, used by `reindex --hook`
/// to find the project root.
fn cwd_of(payload: &Value) -> Option<PathBuf> {
    payload.get("cwd")?.as_str().map(PathBuf::from)
}

/// One line on stderr, gated behind `AURELIUS_HOOK_DEBUG=1` so a hook that
/// fires on every tool call doesn't spam a session nobody is debugging.
fn debug(cmd: &str, reason: &str) {
    if std::env::var("AURELIUS_HOOK_DEBUG").as_deref() == Ok("1") {
        eprintln!("au {cmd} --hook: {reason}");
    }
}

/// True when a snapshot exists and is younger than `min_hours` — the
/// throttle that folds several Claude Code sessions in one day into a
/// single backup. `min_hours == 0` disables the throttle (every call takes
/// a fresh snapshot); it is not an error value, just the fastest cadence.
pub fn throttled(newest: Option<SystemTime>, now: SystemTime, min_hours: u64) -> bool {
    let Some(newest) = newest else {
        return false;
    };
    let elapsed = now.duration_since(newest).unwrap_or_default();
    elapsed < Duration::from_secs(min_hours.saturating_mul(3600))
}

/// Every snapshot except the `keep` newest by mtime — what rotation removes
/// after a fresh backup. Ordered by mtime, not by filename: the
/// UTC-timestamp name happens to sort the same way today, but mtime is the
/// contract.
pub fn to_delete(snapshots: &[(PathBuf, SystemTime)], keep: usize) -> Vec<PathBuf> {
    let mut by_age: Vec<&(PathBuf, SystemTime)> = snapshots.iter().collect();
    by_age.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));
    by_age
        .into_iter()
        .skip(keep)
        .map(|(path, _)| path.clone())
        .collect()
}

/// `au touch --hook` — replaces `aurelius-track-edit.sh`. Increments
/// `access_count` on an existing File node; creates no new nodes, exactly
/// like the wrapper it replaces.
pub async fn touch_hook() {
    let Some(payload) = read_payload() else {
        debug("touch", "no JSON payload on stdin");
        return;
    };
    let Some(path) = file_path_of(&payload) else {
        debug("touch", "no tool_input.file_path or tool_input.path");
        return;
    };
    if !path.is_file() {
        debug("touch", &format!("not a regular file: {}", path.display()));
        return;
    }
    if let Err(e) = commands::touch(&path.to_string_lossy()).await {
        debug("touch", &format!("{e:#}"));
    }
}

/// `au reindex --hook` — replaces `aurelius-reindex.sh`. The project root
/// comes from the hook payload's `cwd`, else the process's own working
/// directory, then either way is raised to the git toplevel when one
/// exists. The share-push step runs regardless of whether the reindex
/// succeeded — the wrapper chained both commands with `|| true`, never
/// `&&`, so one failing was never allowed to skip the other.
pub async fn reindex_hook() {
    let payload = read_payload();
    let start = payload
        .as_ref()
        .and_then(cwd_of)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let root = git_toplevel(&start).unwrap_or(start);

    // Without a database neither the index nor the push can do anything.
    let conn = match aurelius_core::db::open(&aurelius_core::db::db_path()) {
        Ok(conn) => conn,
        Err(e) => {
            debug("reindex", &format!("{e}"));
            return;
        }
    };
    if let Err(e) = aurelius_core::indexer::index_project(&conn, &root) {
        debug("reindex", &format!("{e:#}"));
    }
    match commands::push_targets(None).await {
        Ok(results) => {
            for (label, result) in &results {
                if let Err(e) = result {
                    debug("reindex", &format!("share push to {label}: {e:#}"));
                }
            }
        }
        Err(e) => debug("reindex", &format!("share push: {e:#}")),
    }
}

/// `git rev-parse --show-toplevel` run from `from`, or `None` on any
/// failure (not a repo, `git` missing, non-UTF8 output) — the caller keeps
/// `from` itself in that case, matching the wrapper's `|| pwd` fallback.
fn git_toplevel(from: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// `au db backup --hook [--keep N] [--min-hours H]` — replaces
/// `aurelius-backup.sh`. `keep`/`min_hours` resolve flag → env var →
/// default, the same precedence as the wrapper's `${VAR:-default}`.
///
/// `keep == 0` is refused outright: rotating to zero snapshots would delete
/// the one just taken, so nothing is written in the first place.
/// `min_hours == 0` is a valid "never throttle" value, not an error —
/// [`throttled`] already treats it that way, and it is the only way to force
/// a second snapshot inside the same day (used by the integration test).
pub fn db_backup_hook(keep: Option<usize>, min_hours: Option<u64>) {
    let keep = keep.unwrap_or_else(|| env_num("AURELIUS_BACKUP_KEEP").unwrap_or(7));
    let min_hours = min_hours.unwrap_or_else(|| env_num("AURELIUS_BACKUP_MIN_HOURS").unwrap_or(24));
    if keep == 0 {
        debug(
            "db backup",
            "--keep resolved to 0 — refusing to back up just to delete it",
        );
        return;
    }

    let db = aurelius_core::db::db_path();
    if !db.exists() {
        debug("db backup", "no database yet");
        return;
    }
    let Some(dir) = db.parent() else {
        debug("db backup", "database path has no parent directory");
        return;
    };
    let backups_dir = dir.join("backups");
    if let Err(e) = std::fs::create_dir_all(&backups_dir) {
        debug("db backup", &format!("could not create backups dir: {e}"));
        return;
    }

    let newest = list_snapshots(&backups_dir)
        .into_iter()
        .map(|(_, t)| t)
        .max();
    if throttled(newest, SystemTime::now(), min_hours) {
        debug("db backup", "newest snapshot is within --min-hours");
        return;
    }

    let dest = backups_dir.join(format!(
        "aurelius-{}.db",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    if dest.exists() {
        debug(
            "db backup",
            &format!("destination already exists: {}", dest.display()),
        );
        return;
    }
    if let Err(e) = aurelius_core::db::backup_into(&db, &dest) {
        debug("db backup", &format!("{e:#}"));
        return;
    }

    let check_failure = match aurelius_core::db::check(&dest, false) {
        Err(e) => Some(e.to_string()),
        Ok(report) if !report.ok => Some(report.problems.join("; ")),
        Ok(_) => None,
    };
    if let Some(reason) = check_failure {
        debug(
            "db backup",
            &format!("snapshot failed integrity check: {reason}"),
        );
        let failed = with_suffix(&dest, ".FAILED-CHECK");
        let _ = std::fs::rename(&dest, &failed);
        let _ = std::fs::remove_file(with_suffix(&dest, "-wal"));
        let _ = std::fs::remove_file(with_suffix(&dest, "-shm"));
        return;
    }

    // `dest` is now a real file on disk, so re-listing the directory already
    // picks it up with its real mtime — no need to fabricate an entry.
    for old in to_delete(&list_snapshots(&backups_dir), keep) {
        let _ = std::fs::remove_file(old);
    }
}

/// `path` with `suffix` appended to its file name (`aurelius-x.db` + `-wal`
/// = `aurelius-x.db-wal`, `aurelius-x.db` + `.FAILED-CHECK` =
/// `aurelius-x.db.FAILED-CHECK`) — plain string concatenation, since these
/// suffixes are not real extensions `Path` would recognize.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn env_num<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.parse().ok()
}

/// `aurelius-*.db` files directly under `dir`, with their mtimes. A failed
/// snapshot renamed to `<name>.FAILED-CHECK` never matches this pattern
/// (`.db.FAILED-CHECK` does not end in `.db`), so it is never listed,
/// counted, or rotated away by [`to_delete`].
fn list_snapshots(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with("aurelius-") || !name.ends_with(".db") {
                return None;
            }
            let mtime = entry.metadata().ok()?.modified().ok()?;
            Some((path, mtime))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_path_of_prefers_file_path_over_path() {
        let payload = serde_json::json!({
            "tool_input": {"file_path": "/tmp/a.rs", "path": "/tmp/b.rs"}
        });
        assert_eq!(file_path_of(&payload), Some(PathBuf::from("/tmp/a.rs")));
    }

    #[test]
    fn file_path_of_falls_back_to_path() {
        let payload = serde_json::json!({"tool_input": {"path": "/tmp/b.rs"}});
        assert_eq!(file_path_of(&payload), Some(PathBuf::from("/tmp/b.rs")));
    }

    #[test]
    fn file_path_of_neither_field_is_none() {
        let payload = serde_json::json!({"tool_input": {"command": "ls"}});
        assert_eq!(file_path_of(&payload), None);
    }

    #[test]
    fn file_path_of_non_object_payload_is_none() {
        let payload = serde_json::json!(["not", "an", "object"]);
        assert_eq!(file_path_of(&payload), None);
    }

    #[test]
    fn throttled_with_no_prior_snapshot_never_blocks() {
        assert!(!throttled(None, SystemTime::now(), 24));
    }

    #[test]
    fn throttled_fresh_snapshot_blocks() {
        let now = SystemTime::now();
        let newest = now - Duration::from_secs(60);
        assert!(throttled(Some(newest), now, 24));
    }

    #[test]
    fn throttled_old_snapshot_does_not_block() {
        let now = SystemTime::now();
        let newest = now - Duration::from_secs(25 * 3600);
        assert!(!throttled(Some(newest), now, 24));
    }

    #[test]
    fn to_delete_keeps_everything_when_fewer_than_keep() {
        let now = SystemTime::now();
        let snapshots = vec![
            (PathBuf::from("a"), now),
            (PathBuf::from("b"), now - Duration::from_secs(10)),
        ];
        assert!(to_delete(&snapshots, 5).is_empty());
    }

    #[test]
    fn to_delete_drops_everything_past_keep() {
        let now = SystemTime::now();
        let snapshots = vec![
            (PathBuf::from("newest"), now),
            (PathBuf::from("middle"), now - Duration::from_secs(3600)),
            (PathBuf::from("oldest"), now - Duration::from_secs(7200)),
        ];
        assert_eq!(to_delete(&snapshots, 2), vec![PathBuf::from("oldest")]);
    }

    #[test]
    fn to_delete_orders_by_mtime_not_by_name() {
        let now = SystemTime::now();
        // "a-oldest" sorts first alphabetically but is the oldest by mtime —
        // it must be the one dropped, not "z-newest".
        let snapshots = vec![
            (PathBuf::from("a-oldest"), now - Duration::from_secs(3600)),
            (PathBuf::from("z-newest"), now),
        ];
        assert_eq!(to_delete(&snapshots, 1), vec![PathBuf::from("a-oldest")]);
    }
}
