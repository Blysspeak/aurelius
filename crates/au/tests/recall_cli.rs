//! `au recall` — exact lookup, one record, printed whole.
//!
//! Checked by running the real binary, because the thing under test is what a
//! caller sees: the exit code, the one line on a miss, and the fact that the
//! note comes back the length it went in. Each test gets its own
//! `AURELIUS_HOME`, so the owner's real graph is never touched.

// Integration test — no part of this file is a runtime path; unwrap/expect
// here IS the check.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Stdio};

/// A miss is a usage error, the same code a mistyped argument gets: the caller
/// asked for something that is not there, and asking again the same way will
/// not help. See `main::classify`.
const USAGE: i32 = 1;

struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn dir(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("au-recall-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temporary home");
        Self(path)
    }
}

impl Drop for TmpHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run and return (exit code, stdout, stderr). stderr matters here: the
/// one-line miss message is written there, not to stdout.
fn run(home: &TmpHome, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_au"))
        .env("AURELIUS_HOME", &home.0)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run au");
    (
        out.status.code().expect("the process exited on its own"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Write a memo and return the id it reports.
fn note(home: &TmpHome, args: &[&str]) -> String {
    let mut argv = vec!["note", "--json"];
    argv.extend_from_slice(args);
    let (code, out, err) = run(home, &argv);
    assert_eq!(code, 0, "the memo had to land: {out}{err}");
    let saved: serde_json::Value = serde_json::from_str(out.trim()).expect("memo JSON");
    saved["id"]
        .as_str()
        .expect("id in the memo JSON")
        .to_owned()
}

/// A note long enough that any clipping formatter gives itself away: the tail
/// is what a 60-char label or a 200-char digest would eat.
const LONG_NOTE: &str = "the refund flag was read straight out of the deployed .env, \
not out of the repository copy, and the two have disagreed since the September \
rollout — which is the whole reason this record exists and the whole reason it \
must come back in one piece, tail included: END-OF-NOTE-MARKER";

/// The acceptance case: a memo written with a known subject comes back — that
/// one record and nothing else — by that subject, and by the id the write
/// reported, in one command each.
#[test]
fn a_memo_comes_back_whole_by_its_subject_and_by_its_id() {
    let home = TmpHome::dir("roundtrip");
    let subject = "xhub:.env:REFUND_REQUESTS_ENABLED";

    // A decoy that shares wording with the wanted record. Full-text search
    // would rank it; exact lookup must not see it at all.
    let decoy = note(
        &home,
        &["the refund flag was read straight out of the deployed .env"],
    );

    let wanted = note(
        &home,
        &[
            "--subject",
            subject,
            "--project",
            "recall-demo",
            "--session",
            "run-7",
            "--claim",
            "REFUND_REQUESTS_ENABLED=true",
            "--evidence",
            "cat /home/xhub/app/.env",
            "--confidence",
            "measured",
            "--volatility",
            "volatile",
            "--verify-with",
            "cat /home/xhub/app/.env",
            "--label",
            "refund flag on",
            LONG_NOTE,
        ],
    );

    let (code, out, err) = run(&home, &["recall", subject, "--json"]);
    assert_eq!(code, 0, "the subject had to resolve: {out}{err}");
    let by_subject: serde_json::Value = serde_json::from_str(out.trim()).expect("recall JSON");

    assert_eq!(by_subject["id"], wanted, "the wrong record came back");
    assert_eq!(by_subject["found_by"], "subject");
    assert!(
        !out.contains(&decoy),
        "an unrelated node leaked into an exact lookup: {out}"
    );

    // The whole record, not a digest of it.
    assert_eq!(
        by_subject["note"], LONG_NOTE,
        "the note came back clipped: {out}"
    );
    assert_eq!(by_subject["claim"], "REFUND_REQUESTS_ENABLED=true");
    assert_eq!(by_subject["project"], "recall-demo");
    assert_eq!(by_subject["session"], "run-7");
    assert_eq!(by_subject["memory_kind"], "semantic");
    assert!(
        by_subject["created_at"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "a record with no time on it is not a record: {out}"
    );
    let prov = &by_subject["provenance"];
    assert_eq!(prov["subject"], subject);
    assert_eq!(prov["confidence"], "measured");
    assert_eq!(prov["evidence"], "cat /home/xhub/app/.env");
    assert_eq!(
        prov["volatility"], "volatile",
        "how fast a fact rots is part of the fact: {out}"
    );
    assert_eq!(prov["verify_with"], "cat /home/xhub/app/.env");

    // The same record by the id the write handed back.
    let (code, out, err) = run(&home, &["recall", &wanted, "--json"]);
    assert_eq!(code, 0, "the id had to resolve: {out}{err}");
    let by_id: serde_json::Value = serde_json::from_str(out.trim()).expect("recall JSON");
    assert_eq!(by_id["id"], wanted);
    assert_eq!(by_id["found_by"], "id");
    assert_eq!(by_id["note"], LONG_NOTE);
    assert!(
        !out.contains(&decoy),
        "an unrelated node leaked into a lookup by id: {out}"
    );

    // And unclipped in the human-readable form too, which is the one a person
    // actually reads.
    let (code, out, err) = run(&home, &["recall", subject]);
    assert_eq!(code, 0, "the human-readable form had to print: {out}{err}");
    assert!(
        out.contains("END-OF-NOTE-MARKER"),
        "the printed note lost its tail: {out}"
    );
    assert!(out.contains(subject), "the subject was not printed: {out}");
    assert!(
        out.contains("cat /home/xhub/app/.env"),
        "the evidence was not printed: {out}"
    );
}

/// A subject nobody wrote is a miss — named, and non-zero. Not the nearest
/// full-text neighbour, however close it reads.
#[test]
fn a_subject_that_was_never_written_is_a_miss() {
    let home = TmpHome::dir("subject-miss");

    // Present in the graph, and worded almost exactly like the query below.
    note(
        &home,
        &["the refund requests flag lives in .env and is enabled"],
    );

    let missing = "xhub:.env:REFUND_REQUESTS_ENABLED";
    let (code, out, err) = run(&home, &["recall", missing]);
    assert_eq!(code, USAGE, "a miss must not exit zero: {out}{err}");
    assert!(
        err.contains(missing),
        "the message must name what was searched: {err}"
    );
    assert_eq!(
        err.lines().count(),
        1,
        "a miss is one line, not a report: {err}"
    );
    assert!(
        out.trim().is_empty(),
        "nothing found means nothing printed: {out}"
    );
}

/// A well-formed UUID that names nothing gets the same treatment, and the
/// message names the id rather than the subject it never was.
#[test]
fn an_id_that_names_nothing_is_a_miss() {
    let home = TmpHome::dir("id-miss");
    note(&home, &["some record that does exist"]);

    let missing = "00000000-0000-4000-8000-000000000000";
    let (code, out, err) = run(&home, &["recall", missing, "--json"]);
    assert_eq!(code, USAGE, "a miss must not exit zero: {out}{err}");
    assert!(
        err.contains(missing),
        "the message must name the id searched: {err}"
    );
    assert!(
        err.contains("id"),
        "a miss by id must not read as a miss by subject: {err}"
    );
    assert!(
        out.trim().is_empty(),
        "nothing found means nothing printed: {out}"
    );
}

/// A superseded subject resolves to the fact that replaced it, and says what
/// it replaced. Without the chain the answer is true but unaccountable: the
/// reader cannot tell a first record from a correction.
#[test]
fn a_superseded_subject_comes_back_with_its_chain() {
    let home = TmpHome::dir("supersede");
    let subject = "xhub:.env:REFUND_REQUESTS_ENABLED";

    let old = note(
        &home,
        &["--subject", subject, "--label", "flag on", "flag is on"],
    );
    let new = note(
        &home,
        &[
            "--subject",
            subject,
            "--resolution",
            "supersede",
            "--claim",
            "REFUND_REQUESTS_ENABLED=false",
            "--label",
            "flag off",
            "flag is off since the rollback",
        ],
    );

    let (code, out, err) = run(&home, &["recall", subject, "--json"]);
    assert_eq!(code, 0, "the subject had to resolve: {out}{err}");
    let rec: serde_json::Value = serde_json::from_str(out.trim()).expect("recall JSON");

    assert_eq!(
        rec["id"], new,
        "the newest fact about a subject is the answer: {out}"
    );
    let chain = rec["resolution"].as_array().expect("a resolution chain");
    assert_eq!(chain.len(), 1, "one link, to the fact replaced: {out}");
    assert_eq!(chain[0]["relation"], "supersedes");
    assert_eq!(
        chain[0]["direction"], "outgoing",
        "this record replaced that one, not the other way round: {out}"
    );
    assert_eq!(chain[0]["id"], old);

    // Already accounted for by the chain, so it must not also be reported as
    // an unrelated fact sharing the subject.
    assert_eq!(
        rec["also_with_subject"].as_array().map(Vec::len),
        Some(0),
        "a superseded fact must be listed once, not twice: {out}"
    );

    let (code, out, err) = run(&home, &["recall", subject]);
    assert_eq!(code, 0, "the human-readable form had to print: {out}{err}");
    assert!(
        out.contains("supersedes") && out.contains(&old),
        "the chain must be visible without --json too: {out}"
    );

    // The superseded record is still readable by its own id — superseding is
    // not deleting.
    let (code, out, err) = run(&home, &["recall", &old, "--json"]);
    assert_eq!(code, 0, "the replaced fact is still there: {out}{err}");
    let replaced: serde_json::Value = serde_json::from_str(out.trim()).expect("recall JSON");
    assert_eq!(replaced["id"], old);
    let chain = replaced["resolution"]
        .as_array()
        .expect("a resolution chain");
    assert_eq!(chain.len(), 1, "seen from the other end: {out}");
    assert_eq!(
        chain[0]["direction"], "incoming",
        "from here the edge points inward: {out}"
    );
    assert_eq!(chain[0]["id"], new);
}
