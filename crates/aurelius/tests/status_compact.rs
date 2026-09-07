//! `memory_status` compact-vs-full contract, end to end.
//!
//! The MCP handler module is private (only `node_detail` is re-exported), so
//! this test takes the public door instead: it seeds a fixture database under
//! a uuid-named temp home and drives the real server binary over stdio with
//! `AURELIUS_HOME` pointed at it. The live database is never touched.

// Integration test — the whole file is test code; unwrap/expect here are the
// assertion mechanism itself.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use aurelius_core::models::NodeType;
use aurelius_core::{db, graph};
use serde_json::{json, Value};

/// A temp home directory (uuid-named under `std::env::temp_dir()`). The
/// database inside it is the file `db_path()` resolves to for that home;
/// dropping the home removes the database together with its -wal/-shm.
struct TempHome(PathBuf);

impl TempHome {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("aurelius-status-compact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp home");
        Self(path)
    }

    fn db_path(&self) -> PathBuf {
        self.0.join("aurelius.db")
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        // Removes the db and its -wal/-shm sidecars with the directory.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The server under test, talking JSON-RPC over stdio exactly like a client.
struct Server {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
}

impl Server {
    fn start(home: &TempHome) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_aurelius"))
            .env("AURELIUS_HOME", &home.0)
            // An empty cwd: `ensure_indexed` sees no Cargo.toml and skips, so
            // nothing from this repository leaks into the fixture database.
            .current_dir(&home.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn aurelius server");
        let stdin = child.stdin.take().expect("server stdin");
        let stdout = BufReader::new(child.stdout.take().expect("server stdout"));
        Self {
            stdin,
            stdout,
            child,
        }
    }

    /// One `tools/call` and the parsed tool result for it.
    fn call(&mut self, id: u64, arguments: &str) -> Value {
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"memory_status","arguments":{arguments}}}}}"#
        );
        self.stdin
            .write_all(request.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .expect("write request");

        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read response");
            assert!(read > 0, "server closed stdout before answering id {id}");
            // The daemon's tracing logs share stdout with the protocol, so a
            // non-JSON line here is a log record, not a parse failure.
            let Ok(response) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if response["id"] == json!(id) {
                assert!(
                    response["result"]["isError"].is_null(),
                    "server reported a tool error: {}",
                    response["result"]["content"][0]["text"]
                );
                let text = response["result"]["content"][0]["text"]
                    .as_str()
                    .expect("text content");
                return serde_json::from_str(text).expect("tool result is json");
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One decision with a note well past the clip budget, and one active task
/// with three evidence runs (red, green, green) and the same long note.
/// Returns the note text so assertions can compare against it directly.
fn seed_fixture(conn: &rusqlite::Connection) -> String {
    let long_note = format!(
        "fixture note that must survive whole in full mode and be clipped to an excerpt in \
         compact mode; padded well past the two hundred character budget with filler: {}",
        "filler ".repeat(40)
    );
    graph::add_node(
        conn,
        NodeType::Decision,
        "[fixture] decision with a long note",
        Some(&long_note),
        "test",
        json!({}),
    )
    .expect("seed decision");
    graph::add_node_full(
        conn,
        NodeType::Task,
        "[fixture] active task with three evidence runs",
        Some(&long_note),
        "test",
        json!({
            "status": "active",
            "priority": "high",
            "project": "fixture",
            "evidence": [
                {"command": "cargo fmt --check", "exit_code": 1, "at": "2026-09-07T09:00:00Z"},
                {"command": "cargo clippy", "exit_code": 0, "at": "2026-09-07T09:10:00Z"},
                {"command": "cargo test", "exit_code": 0, "at": "2026-09-07T09:20:00Z"},
            ],
        }),
        aurelius_core::models::MemoryKind::Semantic,
        None,
    )
    .expect("seed task");
    long_note
}

#[test]
fn memory_status_compact_vs_full_end_to_end() {
    let home = TempHome::new();
    let long_note = {
        let conn = db::open(&home.db_path()).expect("open fixture database");
        let note = seed_fixture(&conn);
        // Release the fixture lock before the server opens it.
        drop(conn);
        note
    };

    let mut server = Server::start(&home);

    let compact = server.call(1, r#"{"project":"fixture"}"#);

    // (a) The long note comes back as a word-boundary excerpt near 200
    // chars, flagged with note_truncated.
    let decision = &compact["recent_decisions"][0];
    let note = decision["note"].as_str().expect("note is a string");
    assert!(
        note.chars().count() <= 220,
        "compact note must be an excerpt, got {} chars",
        note.chars().count()
    );
    assert!(note.ends_with('…'), "excerpt must end with the ellipsis");
    assert_eq!(decision["note_truncated"], json!(true));
    // The raw `data` object stays out of compact items — everything it said
    // is already surfaced as claim and provenance.
    assert!(decision.get("data").is_none());

    // (b) Evidence is the summary object with total 3 — total/green and a
    // last_green of command/exit_code/at only, no per-run array.
    let evidence = compact["active_tasks"][0]["evidence"]
        .as_object()
        .expect("evidence is the summary object")
        .clone();
    let keys: Vec<&str> = evidence.keys().map(String::as_str).collect();
    assert!(
        keys.iter()
            .all(|k| ["total", "green", "last_green"].contains(k)),
        "unexpected keys in the evidence summary: {keys:?}"
    );
    assert_eq!(evidence["total"], json!(3));
    assert_eq!(evidence["green"], json!(2));
    let last_green = evidence["last_green"].as_object().expect("last_green");
    assert!(last_green.contains_key("command"));
    assert!(last_green.contains_key("exit_code"));
    assert!(last_green.contains_key("at"));
    assert!(last_green.len() <= 3, "last_green carries run-only fields");

    // (d) The truncation block is present in compact mode.
    assert!(compact["truncation"].is_object());
    assert!(compact["truncation"]["hidden"].is_object());
    assert!(compact["truncation"]["how_to_see_more"].is_string());

    let full = server.call(2, r#"{"project":"fixture","full":true}"#);

    // (c) Full mode returns the unclipped note — and keeps today's exact
    // shape (no truncation block, no evidence summary; the per-run array
    // stays reachable through task_view, as it always has).
    assert_eq!(full["recent_decisions"][0]["note"], json!(long_note));
    assert_eq!(full["active_tasks"][0]["note"], json!(long_note));
    assert!(full["recent_decisions"][0].get("data").is_some());
    assert!(full["active_tasks"][0].get("evidence").is_none());
    assert!(full["active_tasks"][0].get("created_by").is_some());
    assert!(full.get("truncation").is_none());

    // Payload bound on the fixture database: compact must stay far below the
    // tool-result ceiling that pushed the full shape out of the window.
    let compact_len = compact.to_string().len();
    let full_len = full.to_string().len();
    assert!(
        compact_len <= 10_000,
        "compact status ballooned on a tiny fixture: {compact_len} chars"
    );
    assert!(
        compact_len < full_len,
        "compact ({compact_len}) must be smaller than full ({full_len})"
    );
}
