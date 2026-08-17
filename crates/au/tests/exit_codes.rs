//! Договор о кодах возврата и сквозной путь «сессия → узел → связь».
//!
//! Проверяется запуском настоящего бинаря, а не вызовом функций: код возврата
//! существует только у процесса. Вызывающий обязан различать «я позвал
//! неправильно» (1) и «база недоступна» (2) — первое чинится другим вызовом,
//! второе руками, и повторять его бессмысленно.
//!
//! Каждый тест работает в своём `AURELIUS_HOME`, поэтому настоящая база
//! владельца не задета.

use std::io::Write;
use std::process::{Command, Stdio};

const USAGE: i32 = 1;
const STORAGE: i32 = 2;

/// Временный домен данных: каталог для рабочих тестов, файл — для теста
/// недоступного хранилища (`<файл>/aurelius.db` открыть нельзя).
struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn dir(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("au-exit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("создать временный дом");
        Self(path)
    }

    fn file(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("au-exit-{tag}-{}.not-a-dir", std::process::id()));
        std::fs::write(&path, "это файл, а не каталог").expect("создать файл-заглушку");
        Self(path)
    }
}

impl Drop for TmpHome {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = std::fs::remove_dir_all(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn au(home: &TmpHome, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_au"));
    cmd.env("AURELIUS_HOME", &home.0).args(args);
    cmd
}

/// Запустить и вернуть (код возврата, stdout).
fn run(home: &TmpHome, args: &[&str], stdin: Option<&str>) -> (i32, String) {
    let mut cmd = au(home, args);
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("запустить au");
    if let Some(text) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin подключён")
            .write_all(text.as_bytes())
            .expect("записать в stdin");
    }
    let out = child.wait_with_output().expect("дождаться au");
    (
        out.status.code().expect("процесс завершился сам"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn unknown_node_type_is_a_usage_error() {
    let home = TmpHome::dir("type");
    let (code, _) = run(&home, &["note", "--type", "ерунда", "текст"], None);
    assert_eq!(
        code, USAGE,
        "опечатка в --type — ошибка вызова, не хранилища"
    );
}

#[test]
fn unknown_relation_is_a_usage_error() {
    let home = TmpHome::dir("rel");
    let (code, _) = run(&home, &["relate", "a", "b", "--type", "ерунда"], None);
    assert_eq!(code, USAGE);
}

#[test]
fn help_is_not_a_failure() {
    let home = TmpHome::dir("help");
    let (code, _) = run(&home, &["--help"], None);
    assert_eq!(code, 0, "запрошенная справка — не сбой");
}

#[test]
fn unreachable_storage_is_its_own_code() {
    let home = TmpHome::file("storage");
    let (code, _) = run(&home, &["note", "запись в недоступную базу"], None);
    assert_eq!(
        code, STORAGE,
        "недоступное хранилище обязано отличаться от ошибки вызова"
    );
}

/// Сквозной путь механики: записать сессию из stdin, получить её id, привязать
/// к ней узел ребром и увидеть сессию в слое 4 снапшота — а не в слое 5.
#[test]
fn session_lands_in_layer_four_and_relate_links_to_it() {
    let home = TmpHome::dir("flow");
    let project = "тестпроект-au";
    let summary = "снимок перед компакцией из хука";

    let payload = format!(
        r#"{{"summary":"{summary}","decisions":["коды возврата разведены"],"next_steps":["дописать README"]}}"#
    );
    let (code, out) = run(
        &home,
        &["session", "--stdin", "--project", project, "--json"],
        Some(&payload),
    );
    assert_eq!(code, 0, "запись сессии: {out}");
    let session: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON сессии");
    assert_eq!(session["created"], true);
    let session_id = session["id"].as_str().expect("id сессии").to_owned();

    // Повтор той же записи (хук срабатывает и на авто-, и на ручную компакцию).
    let (code, out) = run(
        &home,
        &["session", "--stdin", "--project", project, "--json"],
        Some(&payload),
    );
    assert_eq!(code, 0);
    let repeat: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON повтора");
    assert_eq!(
        repeat["duplicate"], true,
        "повтор не должен плодить близнеца"
    );
    assert_eq!(repeat["id"].as_str(), Some(session_id.as_str()));

    // Узел из «механики» и ребро к сессии — без него граф остаётся кучей.
    let (code, out) = run(
        &home,
        &[
            "note",
            "--type",
            "concept",
            "--project",
            project,
            "--json",
            "у au появился session",
        ],
        None,
    );
    assert_eq!(code, 0, "запись узла: {out}");
    let note: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON узла");
    let note_id = note["id"].as_str().expect("id узла").to_owned();

    let (code, out) = run(
        &home,
        &[
            "relate",
            &note_id,
            &session_id,
            "--type",
            "part-of",
            "--json",
        ],
        None,
    );
    assert_eq!(code, 0, "связывание: {out}");
    let edge: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON ребра");
    assert_eq!(edge["created"], true);
    assert_eq!(
        edge["relation"], "belongs_to",
        "part-of — написание belongs_to, а не новая связь"
    );

    // Повтор той же связи не создаёт второго ребра и честно об этом говорит.
    let (code, out) = run(
        &home,
        &[
            "relate",
            &note_id,
            &session_id,
            "--type",
            "part-of",
            "--json",
        ],
        None,
    );
    assert_eq!(code, 0);
    let again: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON повтора ребра");
    assert_eq!(again["created"], false);
    assert_eq!(again["id"], edge["id"]);

    // Главная проверка: сессия читается слоем 4, а не слоем 5.
    let (code, snapshot) = run(&home, &["snapshot", "--project", project], None);
    assert_eq!(code, 0, "снапшот: {snapshot}");
    let layer4 = section(&snapshot, "4 · Последние сессии");
    let layer5 = section(&snapshot, "5 · Решения и знания");
    assert!(
        layer4.contains(summary),
        "снимок обязан быть в слое 4; снапшот:\n{snapshot}"
    );
    assert!(
        !layer5.contains(summary),
        "снимок не должен лежать среди решений; снапшот:\n{snapshot}"
    );
    assert!(
        layer5.contains("коды возврата разведены"),
        "решение сессии — как раз слой 5; снапшот:\n{snapshot}"
    );
}

/// Запустить с дополнительными переменными окружения.
fn run_env(home: &TmpHome, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = au(home, args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .expect("запустить au с окружением");
    (
        out.status.code().expect("процесс завершился сам"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Ради чего метка заводилась: хук конца сессии обязан собрать СВОИ записи, а
/// не все записи проекта. Проверяем, что чужой прогон и непомеченная запись в
/// выборку не попадают.
#[test]
fn journal_returns_only_what_this_run_wrote() {
    let home = TmpHome::dir("journal");
    let project = "тестпроект-журнал";

    let (code, out) = run(
        &home,
        &[
            "session",
            "--project",
            project,
            "--session",
            "run-one",
            "--json",
            "итог первого прогона",
        ],
        None,
    );
    assert_eq!(code, 0, "запись сессии: {out}");
    let session: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON сессии");
    assert_eq!(
        session["session"], "run-one",
        "ответ обязан подтвердить метку, а не молчать о ней"
    );

    for (text, session_arg) in [
        ("заметка того же прогона", Some("run-one")),
        ("заметка чужого прогона", Some("run-two")),
        ("заметка без метки", None),
    ] {
        let mut args = vec!["note", "--project", project, "--json", text];
        if let Some(id) = session_arg {
            args.extend_from_slice(&["--session", id]);
        }
        let (code, out) = run(&home, &args, None);
        assert_eq!(code, 0, "запись заметки: {out}");
    }

    let (code, out) = run(&home, &["journal", "--session", "run-one", "--json"], None);
    assert_eq!(code, 0, "журнал: {out}");
    let journal: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON журнала");
    assert_eq!(
        journal["count"], 2,
        "сессия и её заметка — и ничего чужого: {journal}"
    );
    let labels = journal["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|e| e["label"].as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        !labels.contains("чужого") && !labels.contains("без метки"),
        "в журнал прогона попало лишнее: {labels}"
    );

    // Переменная окружения — второй канал для хука, у которого stdin занят.
    let (code, from_env) = run_env(
        &home,
        &["journal", "--json"],
        &[("AURELIUS_SESSION_ID", "run-one")],
    );
    assert_eq!(code, 0, "журнал через окружение: {from_env}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(from_env.trim()).expect("JSON")["count"],
        2
    );

    // Без идентификатора выбирать нечего — это ошибка вызова, а не пустой ответ.
    let (code, _) = run_env(
        &home,
        &["journal", "--json"],
        &[("AURELIUS_SESSION_ID", "")],
    );
    assert_eq!(code, USAGE, "журнал без сессии — ошибка вызова");
}

/// Происхождение: заявленное «измерено» обязано предъявить, чем измерено, а
/// незнакомое слово в `--confidence` — остановить запись, а не лечь молча.
#[test]
fn a_measured_claim_must_show_what_measured_it() {
    let home = TmpHome::dir("provenance");

    let (code, out) = run(
        &home,
        &[
            "note",
            "--json",
            "--confidence",
            "measured",
            "флаги выключены",
        ],
        None,
    );
    assert_eq!(code, USAGE, "measured без evidence обязан отказать: {out}");

    let (code, out) = run(
        &home,
        &["note", "--json", "--confidence", "измерено", "что-то"],
        None,
    );
    assert_eq!(code, USAGE, "неизвестная уверенность — ошибка: {out}");

    let (code, out) = run(
        &home,
        &[
            "note",
            "--json",
            "--confidence",
            "measured",
            "--evidence",
            "cat /home/xhub/app/.env",
            "--claim",
            "REFUND_REQUESTS_ENABLED=true",
            "--volatility",
            "volatile",
            "флаг возвратов включён",
        ],
        None,
    );
    assert_eq!(code, 0, "измеренный факт с уликой обязан лечь: {out}");
    let saved: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON заметки");
    assert_eq!(saved["confidence"], "measured");

    // Умолчание — не «наверное измерено», а «не проверено».
    let (code, out) = run(&home, &["note", "--json", "просто заметка"], None);
    assert_eq!(code, 0, "заметка без происхождения: {out}");
    let plain: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON заметки");
    assert_eq!(plain["confidence"], "unverified");
}

/// Противоречие: второе утверждение о том же предмете не ложится рядом молча.
#[test]
fn a_second_claim_about_the_same_subject_needs_resolving() {
    let home = TmpHome::dir("subject");
    let subject = "xhub:.env:REFUND_REQUESTS_ENABLED";

    let (code, out) = run(
        &home,
        &["note", "--json", "--subject", subject, "выключено"],
        None,
    );
    assert_eq!(code, 0, "первый факт о предмете: {out}");

    let (code, out) = run(
        &home,
        &["note", "--json", "--subject", subject, "включено"],
        None,
    );
    assert_eq!(
        code, USAGE,
        "второй факт обязан потребовать разрешения: {out}"
    );

    let (code, out) = run(
        &home,
        &[
            "note",
            "--json",
            "--subject",
            subject,
            "--resolution",
            "supersede",
            "включено",
        ],
        None,
    );
    assert_eq!(code, 0, "с разрешением — ложится: {out}");
    let saved: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON заметки");
    assert_eq!(
        saved["resolved_against"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        1,
        "ребро к вытесненному факту обязано быть в ответе: {saved}"
    );

    let (code, out) = run(
        &home,
        &[
            "note",
            "--json",
            "--subject",
            subject,
            "--resolution",
            "заменяет",
            "ещё раз",
        ],
        None,
    );
    assert_eq!(code, USAGE, "неизвестное разрешение — ошибка: {out}");
}

/// Тело секции markdown-снапшота по её заголовку.
fn section<'a>(snapshot: &'a str, title: &str) -> &'a str {
    let Some(start) = snapshot.find(&format!("## {title}")) else {
        return "";
    };
    let rest = &snapshot[start..];
    match rest[1..].find("\n## ") {
        Some(end) => &rest[..=end],
        None => rest,
    }
}
