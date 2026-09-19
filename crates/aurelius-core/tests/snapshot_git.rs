//! Слой «Репозиторий» и область слоя «Владелец» в снапшоте — на настоящем
//! git-репозитории во временном каталоге и на временной базе через
//! `db::open`, живая база не трогается.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use aurelius_core::db;
use aurelius_core::git;
use aurelius_core::graph;
use aurelius_core::models::{NodeType, Relation};

/// Временный каталог, удаляется при выходе из теста.
struct TmpDir(PathBuf);

impl TmpDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir()
            .join(format!("aurelius-snapshot-git-{}", uuid::Uuid::new_v4()))
            .join(name);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

fn conn_in(dir: &Path) -> rusqlite::Connection {
    db::open(&dir.join("test.db")).expect("open test db")
}

/// git без глобальных хуков, подписи и имени владельца машины.
fn git_in(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
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
        .current_dir(dir)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

/// Репозиторий с двумя коммитами и одним неотслеживаемым файлом.
fn repo(name: &str) -> TmpDir {
    let dir = TmpDir::new(name);
    git_in(&dir.0, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.0.join("a.txt"), "a").expect("write");
    git_in(&dir.0, &["add", "a.txt"]);
    git_in(&dir.0, &["commit", "-q", "-m", "first commit"]);
    std::fs::write(dir.0.join("b.txt"), "b").expect("write");
    git_in(&dir.0, &["add", "b.txt"]);
    git_in(&dir.0, &["commit", "-q", "-m", "second commit"]);
    std::fs::write(dir.0.join("scratch.patch"), "x").expect("write");
    dir
}

/// Снапшот без слоя «Репозиторий» и без номеров слоёв — для сравнения выдачи
/// внутри репозитория и вне него. Номера слоёв теперь постоянны (скелет
/// фиксирован), но нормализация всё равно полезна: тест проверяет содержимое,
/// а не очерёдность.
fn without_repo_layer(md: &str) -> String {
    md.split("\n## ")
        .filter(|section| !section.contains(" · Репозиторий\n"))
        .map(|section| match section.split_once(" · ") {
            Some((n, rest)) if n.chars().all(|c| c.is_ascii_digit()) => rest.to_owned(),
            _ => section.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n## ")
}

fn seed_graph(conn: &rusqlite::Connection, project: &str) {
    let p = graph::add_node(
        conn,
        NodeType::Project,
        project,
        None,
        "test",
        serde_json::json!({}),
    )
    .expect("add project");
    let d = graph::add_node(
        conn,
        NodeType::Decision,
        "взяли sqlite",
        Some("решение: sqlite, потому что память локальная"),
        "test",
        serde_json::json!({}),
    )
    .expect("add decision");
    graph::add_edge(conn, d.id, p.id, Relation::BelongsTo, 1.0).expect("link");
    graph::add_node(
        conn,
        NodeType::UserFact,
        "Отвечать коротко",
        None,
        "test",
        serde_json::json!({}),
    )
    .expect("add user fact");
}

/// (a) Вне репозитория слоя нет, и остальной снапшот не меняется.
#[test]
fn repository_layer_vanishes_outside_a_repository_and_nothing_else_changes() {
    let name = format!("demo{}", std::process::id());
    let repo_dir = repo(&name);
    let elsewhere = TmpDir::new("not-a-repo");
    let conn = conn_in(&elsewhere.0);
    seed_graph(&conn, &name);

    let located = git::locate(&conn, Some(&repo_dir.0), Some(&name)).expect("repo found from cwd");
    let state = git::read(&located).expect("git status answered");
    assert_eq!(state.head, git::Head::Branch("main".into()));
    assert_eq!(state.upstream, None);
    assert_eq!(state.changed, 1);
    assert_eq!(state.paths, vec!["scratch.patch"]);
    assert_eq!(state.commits.len(), 2);
    assert_eq!(state.commits[0].subject, "second commit");

    let inside = graph::build_snapshot_in(&conn, Some(&name), Some(&state)).expect("inside");
    assert!(inside.contains("## 1 · Репозиторий\n"), "{inside}");
    assert!(
        inside.contains(&format!("- {name} on main, no upstream\n")),
        "{inside}"
    );
    assert!(inside.contains("- 1 changed: scratch.patch\n"), "{inside}");
    assert!(inside.contains(" second commit\n"), "{inside}");

    // Вне репозитория и без записанного корня проекта — `None`, без заглушки.
    assert_eq!(git::locate(&conn, Some(&elsewhere.0), Some(&name)), None);
    let outside = graph::build_snapshot_in(&conn, Some(&name), None).expect("outside");
    // Скелет постоянен: вне репозитория слой остаётся первым и с тем же
    // номером, но пустым. Исчезающий заголовок сдвигал бы номера всех слоёв
    // ниже, и «1 · Владелец» в одной сессии значило бы не то же, что в другой.
    assert!(
        outside.contains("## 1 · Репозиторий\n— пусто\n"),
        "вне репозитория слой обязан остаться пустым:\n{outside}"
    );
    assert!(
        outside.contains("## 2 · Владелец\n"),
        "владелец — второй слой постоянного скелета:\n{outside}"
    );

    assert_eq!(without_repo_layer(&inside), without_repo_layer(&outside));
}

/// Каталог сессии вне репозитория и проект не определён: хук пробуждения
/// отдаёт глобальный срез. Он обязан назвать свою область, держать слой
/// «Репозиторий» пустым, а скелет — тем же 1..N без дыр, и не терять знание.
#[test]
fn global_snapshot_outside_a_repository_names_its_scope_and_keeps_the_skeleton() {
    let name = format!("scoped{}", std::process::id());
    let elsewhere = TmpDir::new("not-a-repo");
    let conn = conn_in(&elsewhere.0);
    seed_graph(&conn, &name);

    assert_eq!(git::locate(&conn, Some(&elsewhere.0), None), None);
    let global = graph::build_snapshot_in(&conn, None, None).expect("global");
    let scoped = graph::build_snapshot_in(&conn, Some(&name), None).expect("scoped");

    assert!(global.starts_with("# Память · глобально · "), "{global}");
    assert!(
        global
            .lines()
            .nth(1)
            .is_some_and(|l| l.starts_with("Проект не определён")),
        "глобальный срез обязан сказать, что он не проектный:\n{global}"
    );
    assert!(
        !scoped.contains("Проект не определён"),
        "проектный срез не несёт строки глобального:\n{scoped}"
    );
    assert!(
        global.contains("\n## 1 · Репозиторий\n— пусто\n"),
        "{global}"
    );

    let titles = |md: &str| -> Vec<String> {
        md.lines()
            .filter_map(|l| l.strip_prefix("## "))
            .map(|l| l.split(" · ").nth(1).unwrap_or_default().to_owned())
            .collect()
    };
    assert_eq!(
        titles(&global),
        titles(&scoped),
        "скелет не зависит от области"
    );
    for (i, line) in global.lines().filter(|l| l.starts_with("## ")).enumerate() {
        assert!(
            line.starts_with(&format!("## {} · ", i + 1)),
            "номера слоёв 1..N без дыр: {line}\n{global}"
        );
    }
    assert!(
        global.contains("sqlite"),
        "знание пропало из среза:\n{global}"
    );
}

/// Шаг 2: каталог сессии не репозиторий, но граф знает корень проекта с тем же
/// именем — слой находится по нему.
#[test]
fn repository_is_found_through_the_recorded_project_root() {
    let name = format!("rooted{}", std::process::id());
    let repo_dir = repo(&name);
    let elsewhere = TmpDir::new("not-a-repo");
    let conn = conn_in(&elsewhere.0);
    graph::add_node(
        &conn,
        NodeType::Project,
        &name,
        None,
        "indexer",
        serde_json::json!({ "path": repo_dir.0.to_string_lossy() }),
    )
    .expect("add project with root");

    let located = git::locate(&conn, Some(&elsewhere.0), Some(&name)).expect("found by root");
    assert_eq!(located.root, repo_dir.0);
    assert_eq!(located.name, name);
}

/// (b) Строка владельца, связанная с другим проектом, снимается — и по
/// префиксу метки, и по ребру. Своё идёт первым, глобальное добирает до трёх.
#[test]
fn owner_rows_tied_to_another_project_are_dropped() {
    let dir = TmpDir::new("owner");
    let conn = conn_in(&dir.0);
    let demo = graph::add_node(
        &conn,
        NodeType::Project,
        "demo",
        None,
        "test",
        serde_json::json!({}),
    )
    .expect("add demo");
    let xhub = graph::add_node(
        &conn,
        NodeType::Project,
        "xhub",
        None,
        "test",
        serde_json::json!({}),
    )
    .expect("add xhub");

    let fact = |label: &str| {
        graph::add_node(
            &conn,
            NodeType::UserFact,
            label,
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add user fact")
    };
    fact("[xhub] не спрашивать банк про антифрод");
    let linked = fact("демо-проект банка чисто карточный");
    graph::add_edge(&conn, linked.id, xhub.id, Relation::BelongsTo, 1.0).expect("link xhub");
    let own = fact("в demo релиз только через PR");
    graph::add_edge(&conn, own.id, demo.id, Relation::BelongsTo, 1.0).expect("link demo");
    for i in 0..4 {
        fact(&format!("глобальное правило {i}"));
    }

    let md = graph::build_snapshot(&conn, Some("demo")).expect("snapshot");
    let owner: Vec<&str> = md
        .split("\n## ")
        .find(|s| s.contains("· Владелец\n"))
        .expect("owner layer")
        .lines()
        .skip(1)
        .collect();

    assert!(
        !md.contains("антифрод"),
        "строка с префиксом [xhub] протекла:\n{md}"
    );
    assert!(
        !md.contains("карточный"),
        "строка, связанная с xhub ребром, протекла:\n{md}"
    );
    assert_eq!(owner.len(), 3, "не больше трёх строк:\n{md}");
    assert_eq!(owner[0], "- в demo релиз только через PR");
    assert!(
        owner[1..]
            .iter()
            .all(|l| l.starts_with("- глобальное правило")),
        "{owner:?}"
    );
}
