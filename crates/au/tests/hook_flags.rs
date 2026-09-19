//! Real-binary tests for `--hook` on `au touch` and `au db backup` (spec 009,
//! `contracts/au-cli-hooks.md`). Each test gets its own `AURELIUS_HOME`, so
//! the owner's real database is never touched.
//!
//! Follows the same binary-location and `AURELIUS_HOME` pattern as
//! `exit_codes.rs`'s `TmpHome`/`au`/`run` helpers.

// Integration test — the whole file is not a runtime path; unwrap/expect
// here is the verification method itself.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// A throwaway data/config directory for one test, removed on drop.
struct TmpHome(PathBuf);

impl TmpHome {
    fn dir(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("au-hook-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp home");
        Self(path)
    }
}

impl Drop for TmpHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn au(home: &TmpHome, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_au"));
    cmd.env("AURELIUS_HOME", &home.0).args(args);
    cmd
}

/// Run and return (exit code, stdout).
fn run(home: &TmpHome, args: &[&str], stdin: Option<&str>) -> (i32, String) {
    let mut cmd = au(home, args);
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn au");
    if let Some(text) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin connected")
            .write_all(text.as_bytes())
            .expect("write to stdin");
    }
    let out = child.wait_with_output().expect("wait for au");
    (
        out.status.code().expect("process exited on its own"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Every `aurelius-*.db` snapshot currently in `<home>/backups`.
fn backup_files(home: &TmpHome) -> Vec<PathBuf> {
    let dir = home.0.join("backups");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("aurelius-") && name.ends_with(".db")
        })
        .collect()
}

/// The single path present in `after` but not in `before`.
fn new_file(before: &[PathBuf], after: &[PathBuf]) -> PathBuf {
    after
        .iter()
        .find(|p| !before.contains(p))
        .cloned()
        .expect("exactly one new snapshot file")
}

#[test]
fn touch_hook_ignores_non_json_stdin_and_exits_zero() {
    let home = TmpHome::dir("touch-not-json");
    let (code, out) = run(&home, &["touch", "--hook"], Some("not-json"));
    assert_eq!(code, 0, "non-JSON stdin must not fail the hook");
    assert!(out.is_empty(), "the hook must not write to stdout: {out:?}");
}

/// (b) two calls right after `init` leave exactly one snapshot (default
/// 24h throttle). (c) `--min-hours 0` after a >1s pause lifts the throttle
/// and adds a second file. (d) one more `--min-hours 0` snapshot, then
/// `--keep 2` rotates down to the two newest — proven by tracking which
/// file each call actually created, not just counting.
#[test]
fn db_backup_hook_throttles_keeps_and_rotates() {
    let home = TmpHome::dir("db-backup");

    let (code, out) = run(&home, &["init"], None);
    assert_eq!(code, 0, "au init: {out}");

    let (code, out) = run(&home, &["db", "backup", "--hook"], None);
    assert_eq!(code, 0);
    assert!(
        out.is_empty(),
        "first call takes a real snapshot and must not write to stdout: {out:?}"
    );
    let after1 = backup_files(&home);
    assert_eq!(
        after1.len(),
        1,
        "first call must create a snapshot: {after1:?}"
    );

    // Immediate repeat: default 24h throttle must block a second snapshot.
    let (code, out) = run(&home, &["db", "backup", "--hook"], None);
    assert_eq!(code, 0);
    assert!(
        out.is_empty(),
        "throttled call must not write to stdout: {out:?}"
    );
    assert_eq!(
        backup_files(&home),
        after1,
        "throttle must leave the same single file"
    );

    // `--min-hours 0` lifts the throttle. The sleep keeps the UTC-second
    // timestamp in the file name from colliding with the previous snapshot.
    std::thread::sleep(Duration::from_millis(1100));
    let (code, out) = run(&home, &["db", "backup", "--hook", "--min-hours", "0"], None);
    assert_eq!(code, 0);
    assert!(
        out.is_empty(),
        "second real snapshot must not write to stdout: {out:?}"
    );
    let after2 = backup_files(&home);
    assert_eq!(
        after2.len(),
        2,
        "--min-hours 0 must lift the throttle: {after2:?}"
    );
    std::thread::sleep(Duration::from_millis(1100));
    let (code, out) = run(&home, &["db", "backup", "--hook", "--min-hours", "0"], None);
    assert_eq!(code, 0);
    assert!(
        out.is_empty(),
        "third real snapshot must not write to stdout: {out:?}"
    );
    let after3 = backup_files(&home);
    assert_eq!(after3.len(), 3, "third snapshot must land: {after3:?}");
    let snap3 = new_file(&after2, &after3);

    // Fourth call creates one more snapshot and rotates to the two newest —
    // this snapshot plus snap3, not the two oldest (snap1, snap2).
    std::thread::sleep(Duration::from_millis(1100));
    let (code, out) = run(
        &home,
        &["db", "backup", "--hook", "--min-hours", "0", "--keep", "2"],
        None,
    );
    assert_eq!(code, 0);
    assert!(
        out.is_empty(),
        "fourth real snapshot (with rotation) must not write to stdout: {out:?}"
    );
    let after4 = backup_files(&home);
    assert_eq!(
        after4.len(),
        2,
        "--keep 2 must leave exactly two files: {after4:?}"
    );
    let snap4 = new_file(&after3, &after4);

    let mut remaining = after4;
    remaining.sort();
    let mut expected = vec![snap3, snap4];
    expected.sort();
    assert_eq!(
        remaining, expected,
        "the two newest snapshots must survive rotation, not the two oldest"
    );
}

/// The temp home is not a git repository and has no sync-enabled projects,
/// so this exercises the `git rev-parse` fallback to `cwd`, `index_project`
/// on an arbitrary directory, and the "no targets" push branch — none of
/// which may write to stdout (contract: `au-cli-hooks.md`, common rule 3).
#[test]
fn reindex_hook_is_silent_and_exits_zero() {
    let home = TmpHome::dir("reindex-hook");

    let (code, out) = run(&home, &["init"], None);
    assert_eq!(code, 0, "au init: {out}");

    let payload = serde_json::json!({"cwd": home.0.to_string_lossy()}).to_string();
    let (code, out) = run(&home, &["reindex", "--hook"], Some(&payload));
    assert_eq!(code, 0, "reindex --hook must exit 0 outside a git repo");
    assert!(out.is_empty(), "the hook must not write to stdout: {out:?}");
}

/// `snapshot --hook` from inside a repository opens with the repository
/// layer: name, branch, dirt and the last commit, all in the payload the
/// waking model reads. Outside any repository there is no project to derive
/// (`hook_project`), and the hook used to answer with nothing — a session
/// woke with no memory at all. Now it falls back to the global snapshot,
/// says so under the header, and keeps the repository layer empty.
#[test]
fn snapshot_hook_leads_with_the_repository_it_stands_in() {
    let home = TmpHome::dir("snapshot-hook");
    let repo = home.0.join("wakedemo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(&repo)
            .status()
            .expect("run git")
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "trunk"]);
    std::fs::write(repo.join("a.txt"), "a").expect("write");
    git(&["add", "a.txt"]);
    git(&["commit", "-q", "-m", "wake up here"]);
    std::fs::write(repo.join("dirty.txt"), "d").expect("write");

    let payload = serde_json::json!({"cwd": repo.to_string_lossy()}).to_string();
    let (code, out) = run(&home, &["snapshot", "--hook"], Some(&payload));
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&out).expect("hook JSON");
    let md = json["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext");
    assert!(md.starts_with("# Память · wakedemo · "), "{md}");
    assert!(
        md.contains("## 1 · Репозиторий\n- wakedemo on trunk, no upstream\n"),
        "{md}"
    );
    assert!(md.contains("- 1 changed: dirty.txt\n"), "{md}");
    assert!(md.contains(" wake up here\n"), "{md}");
    assert!(
        out.len() <= 4000,
        "payload over the ceiling: {} bytes",
        out.len()
    );

    let outside = home.0.join("plain");
    std::fs::create_dir_all(&outside).expect("create plain dir");
    let payload = serde_json::json!({"cwd": outside.to_string_lossy()}).to_string();
    let (code, out) = run(&home, &["snapshot", "--hook"], Some(&payload));
    assert_eq!(code, 0);
    assert!(
        !out.is_empty(),
        "outside a repository the hook must not answer with nothing"
    );
    let json: serde_json::Value = serde_json::from_str(&out).expect("hook JSON");
    let md = json["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext");
    assert!(md.starts_with("# Память · глобально · "), "{md}");
    assert!(md.contains("Проект не определён"), "scope unsaid:\n{md}");
    assert!(md.contains("\n## 1 · Репозиторий\n— пусто\n"), "{md}");
    assert!(
        out.len() <= 4000,
        "payload over the ceiling: {} bytes",
        out.len()
    );
}

/// A snapshot that cannot be built is not silence either: the hook still
/// exits 0, and the payload names the failure instead of looking like an
/// empty memory.
#[test]
fn snapshot_hook_names_a_failure_instead_of_answering_with_nothing() {
    let home = TmpHome::dir("snapshot-hook-broken");
    std::fs::write(home.0.join("aurelius.db"), vec![0x5a; 8192]).expect("write junk db");
    let payload = serde_json::json!({"cwd": home.0.to_string_lossy()}).to_string();
    let (code, out) = run(&home, &["snapshot", "--hook"], Some(&payload));
    assert_eq!(code, 0, "a hook never fails the session start");
    let json: serde_json::Value = serde_json::from_str(&out).expect("hook JSON");
    let md = json["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext");
    assert!(md.starts_with("# Память недоступна\n"), "{md}");
}
