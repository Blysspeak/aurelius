//! Which Claude Code session a process belongs to, without a schema change.
//!
//! The SessionStart hook (`au session-hook --hook`) records the session id
//! under the pid of its parent — the Claude Code process — as one small file
//! `<data dir>/sessions/<pid>`. Any descendant (the MCP server, `au task
//! evidence` run by the verify hook) finds it by walking its own parent chain.
//! `/clear` fires SessionStart again and overwrites the file.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Files older than this are removed on every write.
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// How many ancestors `resolve_session` inspects.
const MAX_DEPTH: usize = 6;

/// `<data dir>/sessions`, beside the database.
pub fn registry_dir() -> PathBuf {
    let db = crate::db::db_path();
    db.parent()
        .map_or_else(std::env::temp_dir, Path::to_path_buf)
        .join("sessions")
}

/// Record `session_id` for `pid` and prune stale entries.
pub fn record_in(dir: &Path, pid: u32, session_id: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    prune(dir);
    let now = chrono::Utc::now().timestamp();
    std::fs::write(dir.join(pid.to_string()), format!("{session_id}\n{now}\n"))?;
    Ok(())
}

fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > MAX_AGE);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Session recorded for exactly `pid`, if any.
fn lookup(dir: &Path, pid: u32) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join(pid.to_string())).ok()?;
    let id = raw.lines().next()?.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// Walk `start`'s ancestors (at most `MAX_DEPTH`, starting with its parent)
/// and return the session of the first registered one.
pub fn resolve_in(
    dir: &Path,
    start: u32,
    parent_of: impl Fn(u32) -> Option<u32>,
) -> Option<String> {
    let mut pid = start;
    for _ in 0..MAX_DEPTH {
        pid = parent_of(pid)?;
        if pid <= 1 {
            return None;
        }
        if let Some(id) = lookup(dir, pid) {
            return Some(id);
        }
    }
    None
}

/// Parent pid from `/proc/<pid>/stat` (fourth field). `None` off Linux.
pub fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name (field 2) may contain spaces and parentheses: the
    // fields after the last ')' are state, then ppid.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Record `session_id` for the parent of the current process (the SessionStart
/// hook's parent is Claude Code).
pub fn record_for_parent(session_id: &str) -> Result<()> {
    let ppid =
        parent_pid(std::process::id()).ok_or_else(|| anyhow::anyhow!("parent pid unavailable"))?;
    record_in(&registry_dir(), ppid, session_id)
}

/// Session of the Claude Code process this one descends from, read fresh.
pub fn resolve_session() -> Option<String> {
    resolve_in(&registry_dir(), std::process::id(), parent_pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp() -> TmpDir {
        TmpDir(std::env::temp_dir().join(format!("aurelius-sessions-{}", uuid::Uuid::new_v4())))
    }

    #[test]
    fn write_then_resolve_through_fake_chain() {
        let d = tmp();
        // 500 (mcp) -> 400 (claude) -> 300 -> 1
        let chain: HashMap<u32, u32> = [(500, 400), (400, 300), (300, 1), (600, 500)].into();
        let parent = |p: u32| chain.get(&p).copied();
        assert_eq!(resolve_in(&d.0, 500, parent), None);
        record_in(&d.0, 400, "sess-a").unwrap();
        assert_eq!(resolve_in(&d.0, 500, parent).as_deref(), Some("sess-a"));
        // Grandchild (verify hook -> au) finds the same session.
        assert_eq!(resolve_in(&d.0, 600, parent).as_deref(), Some("sess-a"));
        // /clear overwrites.
        record_in(&d.0, 400, "sess-b").unwrap();
        assert_eq!(resolve_in(&d.0, 600, parent).as_deref(), Some("sess-b"));
    }

    #[test]
    fn depth_is_bounded() {
        let d = tmp();
        record_in(&d.0, 100, "deep").unwrap();
        // 200 -> 199 -> ... -> 100: 100 is 100 levels up, beyond MAX_DEPTH.
        let parent = |p: u32| (p > 100).then(|| p - 1);
        assert_eq!(resolve_in(&d.0, 200, parent), None);
        assert_eq!(resolve_in(&d.0, 104, parent).as_deref(), Some("deep"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parent_pid_reads_proc_on_linux() {
        assert_eq!(
            parent_pid(std::process::id()),
            Some(std::os::unix::process::parent_id())
        );
    }
}
