//! `au recall --prefix` — facet lookup: every live node whose `subject`
//! starts with a facet, one entry per distinct subject, newest family first.
//!
//! Same reasoning as `exit_codes.rs`: the exit code is the contract (a rule
//! in ulika keys off it, not off printed text), so every test here runs the
//! real binary and reads its exit status, not a function's return value.
//! Helpers below are the same shape as `exit_codes.rs`'s — a fresh
//! `AURELIUS_HOME` per test, `run()` returning (exit code, stdout) — rather
//! than a second harness invented for one file.

// Integration test — no part of this file is a runtime path; unwrap/expect
// here IS the check.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Stdio};

const USAGE: i32 = 1;

struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn dir(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("au-recall-prefix-{tag}-{}", std::process::id()));
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

fn au(home: &TmpHome, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_au"));
    cmd.env("AURELIUS_HOME", &home.0).args(args);
    cmd
}

/// Run and return (exit code, stdout, stderr). stderr matters for the two
/// clap-rejection tests: the message is what tells a bad combination from a
/// bad flag.
fn run(home: &TmpHome, args: &[&str]) -> (i32, String, String) {
    let out = au(home, args)
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

/// Write a fact under a subject with a given claim, `--claim` carrying the
/// whole assertion so no positional text is needed (`note`'s
/// `required_unless_present_any = ["stdin", "claim"]`).
///
/// `--resolution coexist` is passed unconditionally, including on a
/// subject's first write: `note`'s conflict guard only fires when a live
/// node already carries the subject (`guard_subject`'s `existing.is_empty()`
/// short-circuit), so on a first write this resolves nothing and leaves no
/// edge — the same "several live facts, no edge between them" shape the
/// task's own `xhub:bank131:refunds` example describes.
fn note_with_subject(home: &TmpHome, subject: &str, claim: &str) {
    let (code, out, err) = run(
        home,
        &[
            "note",
            "--json",
            "--subject",
            subject,
            "--claim",
            claim,
            "--resolution",
            "coexist",
        ],
    );
    assert_eq!(code, 0, "setup note had to land: {out}{err}");
}

/// The acceptance case: several subjects share a prefix, the family with the
/// most recently written member comes first, and `count` counts nodes, not
/// distinct claim text. The `xhub:bank131:refunds` family below has three
/// nodes but only two distinct claim strings (the first two repeat "flag
/// toggled") — `count` must read 3, which is only possible if it counts
/// nodes; a distinct-claims count would read 2.
#[test]
fn prefix_matching_several_subjects_exits_zero_and_lists_newest_first() {
    let home = TmpHome::dir("hit");

    // Three nodes under one subject, two sharing a claim string, written in
    // order so the last is unambiguously the newest member.
    note_with_subject(&home, "xhub:bank131:refunds", "flag toggled");
    note_with_subject(&home, "xhub:bank131:refunds", "flag toggled");
    note_with_subject(&home, "xhub:bank131:refunds", "third and newest note");
    // A second subject under the same prefix, written last — its single node
    // is newer than every refunds node, so this family must sort first.
    note_with_subject(&home, "xhub:bank131:chargebacks", "chargeback spike");
    // Same subject text, different prefix — must not appear in the result.
    note_with_subject(
        &home,
        "xhub:antifraud:research:data-layer",
        "unrelated facet",
    );

    let (code, out, err) = run(&home, &["recall", "--prefix", "xhub:bank131", "--json"]);
    assert_eq!(code, 0, "a non-empty prefix result must exit 0: {out}{err}");

    let rec: serde_json::Value = serde_json::from_str(out.trim()).expect("recall --json output");
    assert_eq!(rec["prefix"], "xhub:bank131");
    assert_eq!(rec["count"], 2, "two distinct subjects under this prefix");

    let families = rec["families"].as_array().expect("families array");
    assert_eq!(families.len(), 2);

    assert_eq!(
        families[0]["subject"], "xhub:bank131:chargebacks",
        "the family whose newest member was written last sorts first: {out}"
    );
    assert_eq!(families[0]["count"], 1);
    assert_eq!(families[0]["claim"], "chargeback spike");

    assert_eq!(families[1]["subject"], "xhub:bank131:refunds");
    assert_eq!(
        families[1]["count"], 3,
        "count is nodes carrying the subject (3), not distinct claim text (2): {out}"
    );
    assert_eq!(
        families[1]["claim"], "third and newest note",
        "the reported claim is the newest node's, not the first written: {out}"
    );
    assert!(
        families[1]["id"].as_str().is_some_and(|s| !s.is_empty()),
        "the newest node's id must be reported: {out}"
    );
    assert!(
        families[1]["created_at"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "the newest node's created_at must be reported: {out}"
    );

    // Human-readable form: same facts, `line()`-shaped output, no --json.
    let (code, out, err) = run(&home, &["recall", "--prefix", "xhub:bank131"]);
    assert_eq!(code, 0, "human-readable form must also exit 0: {out}{err}");
    assert!(out.contains("xhub:bank131:chargebacks"), "{out}");
    assert!(out.contains("xhub:bank131:refunds"), "{out}");
    assert!(out.contains("third and newest note"), "{out}");
    assert!(
        !out.contains("xhub:antifraud"),
        "an unrelated facet leaked into the prefix result: {out}"
    );
}

/// A facet nobody wrote is a miss — same code the exact lookup already uses
/// on a miss, so the ulika lock can key its rule off this exit code alone.
#[test]
fn prefix_matching_nothing_exits_one() {
    let home = TmpHome::dir("miss");
    note_with_subject(
        &home,
        "xhub:bank131:refunds",
        "unrelated to the query below",
    );

    let missing_prefix = "xhub:no-such-facet";
    let (code, out, err) = run(&home, &["recall", "--prefix", missing_prefix, "--json"]);
    assert_eq!(
        code, USAGE,
        "an empty prefix result must not exit 0: {out}{err}"
    );
    assert!(
        out.trim().is_empty(),
        "nothing found means nothing printed on stdout: {out}"
    );
    assert!(
        err.contains(missing_prefix),
        "the message must name what was searched: {err}"
    );
}

/// Positional query and `--prefix` are mutually exclusive; clap rejects the
/// combination before the handler ever runs, so no node needs to exist.
#[test]
fn positional_query_and_prefix_together_is_rejected() {
    let home = TmpHome::dir("both");
    let (code, _out, err) = run(&home, &["recall", "some-query", "--prefix", "xhub:bank131"]);
    assert_eq!(
        code, USAGE,
        "query and --prefix together must be a usage error: {err}"
    );
}

/// Neither the positional query nor `--prefix` is also rejected — exactly one
/// of the two is required.
#[test]
fn neither_query_nor_prefix_is_rejected() {
    let home = TmpHome::dir("neither");
    let (code, _out, err) = run(&home, &["recall"]);
    assert_eq!(
        code, USAGE,
        "recall with neither a query nor --prefix must be a usage error: {err}"
    );
}
