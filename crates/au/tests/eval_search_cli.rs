//! `au eval-search` без демона эмбеддингов: FTS считается, dense и RRF
//! пропускаются с причиной, код возврата 0 — отсутствие демона не провал.
//! Стенд — свой `AURELIUS_HOME` с одной заметкой, живая база не задета.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Stdio};

struct TmpHome(std::path::PathBuf);

impl Drop for TmpHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn au(home: &TmpHome, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_au"))
        .env("AURELIUS_HOME", &home.0)
        .current_dir(&home.0)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("запуск au");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().expect("код возврата"), text)
}

#[test]
fn without_daemon_fts_is_scored_and_vectors_are_skipped() {
    let path = std::env::temp_dir().join(format!("au-eval-search-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    let home = TmpHome(path);

    let (code, out) = au(
        &home,
        &[
            "note",
            "--json",
            "Пингвины зимуют колонией на льду Антарктиды",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let id = serde_json::from_str::<serde_json::Value>(out.lines().last().unwrap()).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let cases = home.0.join("cases.jsonl");
    let line =
        serde_json::json!({"id": "ru-01", "class": "ru", "query": "пингвины", "expect": [id]});
    std::fs::write(&cases, format!("{{\"meta\":{{}}}}\n{line}\n")).unwrap();

    let (code, out) = au(&home, &["eval-search", cases.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{out}");
    let report: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(report["board"]["fts"]["ru"]["at5"], 1, "{out}");
    assert!(report["board"].get("dense").is_none(), "{out}");
    assert!(report["vector_skipped"].is_string(), "{out}");
}
