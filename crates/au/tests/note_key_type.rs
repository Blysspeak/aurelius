//! Задача af49ce11: `au note --key` без `--type` брал умолчание `decision` и
//! молча переписывал под ним узел другого типа. Смена типа — только явным
//! `--type`; умолчание не выбор, и совпадение ключа с чужим типом — отказ.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Output, Stdio};

struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("au-notekey-{tag}-{}", std::process::id()));
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

/// Id из строки `✓ Saved: [id] ...`.
fn saved_id(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find('[').unwrap() + 1;
    let end = text[start..].find(']').unwrap() + start;
    text[start..end].to_owned()
}

fn recall_type(home: &TmpHome, id: &str) -> String {
    let out = au(home, &["recall", id]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines()
        .find_map(|l| l.strip_prefix("type"))
        .map(|t| t.trim().to_owned())
        .unwrap_or_else(|| panic!("нет строки type: {text}"))
}

#[test]
fn default_type_does_not_retype_a_foreign_node() {
    let home = TmpHome::new("default");
    let key = "test:af49:shared";
    let first = au(&home, &["note", "--type", "concept", "--key", key, "факт"]);
    assert!(first.status.success(), "{first:?}");
    let id = saved_id(&first);

    let second = au(&home, &["note", "--key", key, "чужая запись"]);
    assert!(!second.status.success(), "обязан отказать: {second:?}");
    let err = String::from_utf8_lossy(&second.stderr);
    assert!(
        err.contains("concept") && err.contains("decision"),
        "отказ называет оба типа: {err}"
    );
    assert_eq!(recall_type(&home, &id), "concept", "узел не тронут");
}

#[test]
fn explicit_type_still_retypes() {
    let home = TmpHome::new("explicit");
    let key = "test:af49:retype";
    let first = au(&home, &["note", "--type", "concept", "--key", key, "факт"]);
    assert!(first.status.success());
    let id = saved_id(&first);
    let out = au(
        &home,
        &["note", "--type", "problem", "--key", key, "теперь проблема"],
    );
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("тип узла изменён"));
    assert_eq!(recall_type(&home, &id), "problem");
}
