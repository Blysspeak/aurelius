//! `au search --json` (задача 9b06c9da): тот же гибридный путь, что и
//! человеческий вывод; без демона — полнотекстовая выдача плюс явный
//! `vector_notice`, код выхода 0; `--type` фильтрует; многострочная заметка
//! не рвёт JSON.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Output, Stdio};

struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("au-searchjson-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TmpHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn au(home: &TmpHome, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_au"))
        .env("AURELIUS_HOME", &home.0)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Переводы строк, кавычки и таб — всё, что рвёт наивно склеенный JSON.
const MULTILINE: &str = "зебрафлокс падает\nвторая строка \"в кавычках\"\n\tтаб";

fn seeded(tag: &str) -> TmpHome {
    let home = TmpHome::new(tag);
    for (ty, text) in [
        ("problem", MULTILINE),
        ("solution", "зебрафлокс починен"),
        ("concept", "зебрафлокс устроен так"),
    ] {
        let out = au(&home, &["note", "--type", ty, text]);
        assert!(out.status.success(), "{out:?}");
    }
    home
}

fn search_json(home: &TmpHome, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["search", "зебрафлокс", "--json", "--limit", "10"];
    args.extend_from_slice(extra);
    let out = au(home, &args);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        text.trim_end().lines().count(),
        1,
        "одна строка JSON: {text}"
    );
    serde_json::from_str(&text).unwrap()
}

#[test]
fn without_daemon_answers_fts_only_with_notice() {
    let home = seeded("nodaemon");
    let v = search_json(&home, &[]);
    assert_eq!(v["vectors"], false);
    assert!(v["vector_notice"].as_str().is_some_and(|s| !s.is_empty()));
    let hits = v["results"].as_array().unwrap();
    assert_eq!(hits.len(), 3, "{v}");
    for hit in hits {
        assert_eq!(hit["origin"], "fts");
        assert!(hit["score"].is_number(), "{hit}");
        assert_eq!(hit["created_at"].as_str().unwrap().len(), 10);
        assert!(hit["id"].as_str().is_some_and(|s| s.len() == 36));
        assert!(hit.get("claim").is_some() && hit.get("subject").is_some());
    }
    assert!(
        hits.iter().any(|h| h["note"] == MULTILINE),
        "многострочная заметка обязана дойти целой: {v}"
    );
}

#[test]
fn type_filter_keeps_only_listed_types() {
    let home = seeded("types");
    let v = search_json(&home, &["--type", "problem,solution"]);
    let mut types: Vec<_> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["type"].as_str().unwrap().to_owned())
        .collect();
    types.sort();
    assert_eq!(types, ["problem", "solution"], "{v}");
    let one = search_json(&home, &["--type", "concept"]);
    assert_eq!(one["count"], 1, "{one}");
    let repeated = search_json(&home, &["--type", "concept", "--type", "solution"]);
    assert_eq!(repeated["count"], 2, "{repeated}");
}

#[test]
fn unknown_type_is_refused_not_answered_empty() {
    let home = seeded("typo");
    let out = au(
        &home,
        &["search", "зебрафлокс", "--json", "--type", "problems"],
    );
    assert!(!out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("problems"));
}

#[test]
fn json_and_human_output_agree_on_count() {
    let home = seeded("agree");
    let v = search_json(&home, &[]);
    let human = au(&home, &["search", "зебрафлокс", "--limit", "10"]);
    let text = String::from_utf8_lossy(&human.stdout);
    let n = v["count"].as_u64().unwrap();
    assert!(text.starts_with(&format!("{n} results:")), "{text}");
    for hit in v["results"].as_array().unwrap() {
        let label = hit["label"].as_str().unwrap();
        assert!(
            text.contains(label),
            "метка {label} обязана быть и в человеческом выводе"
        );
    }
}
