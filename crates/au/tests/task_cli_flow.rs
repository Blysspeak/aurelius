//! Находка 9 (адверсариальный разбор спеки 007): вся проводка CLI-стороны
//! круга «задача — работа — улика — закрытие» (`au task evidence`, `au task
//! done`, `au task activate`, `au task ripe`/`--decline`, `au judge --hook`,
//! `au secret add/list/rm`) была покрыта ТОЛЬКО двумя негативными тестами —
//! ни один тест не гонял бинарь по успешному пути. Доказательство дыры:
//! перестановка местами позиционных аргументов `commit`/`pull_request` в
//! вызове `build_resolution` внутри `TaskAction::Done` не роняла ни одного
//! теста `cargo test --workspace`.
//!
//! Здесь — тесты именно на проводку: порядок аргументов, разбор флагов clap,
//! форматирование вывода человеку. Каждый тест — свой `AURELIUS_HOME`
//! (`TmpHome`), настоящая база пользователя не задета.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::process::{Command, Stdio};

/// Временный домен данных — свой на тест, как и в `exit_codes.rs`.
struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn dir(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("au-task-flow-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("создать временный дом");
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

/// Запустить и вернуть (код возврата, stdout, stderr).
fn run(home: &TmpHome, args: &[&str]) -> (i32, String, String) {
    let out = au(home, args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("запустить au");
    (
        out.status.code().expect("процесс завершился сам"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Запустить с указанным рабочим каталогом подпроцесса — нужно там, где
/// поведение зависит от имени текущей папки (`current_dir_name()`), как
/// `au trace --hook`.
fn run_in(home: &TmpHome, cwd: &std::path::Path, args: &[&str]) -> (i32, String) {
    let out = au(home, args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("запустить au");
    (
        out.status.code().expect("процесс завершился сам"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// То же, что `run_in`, но и stderr возвращается: нужно там, где команда
/// обязана что-то СКАЗАТЬ в stderr (`au task done` без `--commit`), не
/// смешивая это с результатом в stdout.
fn run_in_full(home: &TmpHome, cwd: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = au(home, args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("запустить au");
    (
        out.status.code().expect("процесс завершился сам"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Запустить со stdin — нужно `au trace --hook`, который читает JSON хука
/// оттуда.
fn run_with_stdin_in(
    home: &TmpHome,
    cwd: &std::path::Path,
    args: &[&str],
    stdin: &str,
) -> (i32, String) {
    let mut cmd = au(home, args);
    cmd.current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("запустить au");
    child
        .stdin
        .as_mut()
        .expect("stdin подключён")
        .write_all(stdin.as_bytes())
        .expect("записать в stdin");
    let out = child.wait_with_output().expect("дождаться au");
    (
        out.status.code().expect("процесс завершился сам"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// `au task new` печатает `✓ Task created: [<uuid>]` первой строкой —
/// достать id оттуда.
fn created_task_id(stdout: &str) -> String {
    let line = stdout.lines().next().expect("хотя бы одна строка вывода");
    let start = line.find('[').expect("id в квадратных скобках");
    let end = line.find(']').expect("закрывающая скобка");
    line[start + 1..end].to_owned()
}

/// Находка 9, воспроизведение конкретного дефекта: проверяющий поменял
/// местами позиционные аргументы `commit`/`pull_request` в вызове
/// `build_resolution` внутри `TaskAction::Done` — `au task done --commit X`
/// записал бы `X` в `pull_request`, а не в `commit`. Ни один существующий
/// тест это не ловил, потому что ни один не гонял `au task done` по
/// успешному пути вовсе. Тест падает при переставленных аргументах и
/// проходит на текущем коде.
#[test]
fn task_done_records_commit_and_pull_request_without_swapping_them() {
    let home = TmpHome::dir("done-swap");

    let (code, out, err) = run(
        &home,
        &[
            "task",
            "new",
            "починить проводку done",
            "--project",
            "proj-done",
        ],
    );
    assert_eq!(code, 0, "создание задачи: stdout={out} stderr={err}");
    let id = created_task_id(&out);

    let (code, out, err) = run(&home, &["task", "activate", &id]);
    assert_eq!(code, 0, "активация: stdout={out} stderr={err}");

    let commit = "deadbeef0123";
    let pr_url = "https://example.invalid/pulls/42";
    let (code, out, err) = run(
        &home,
        &["task", "done", &id, "--commit", commit, "--pr", pr_url],
    );
    assert_eq!(code, 0, "закрытие задачи: stdout={out} stderr={err}");
    // Закрытие с явно указанными commit/pr обязано быть подтверждённым —
    // предупреждения о неподтверждённом закрытии тут быть не должно.
    assert!(
        !out.contains("без подтверждения"),
        "закрытие с явным commit/pr не обязано быть неподтверждённым: {out}"
    );

    let (code, show_out, err) = run(&home, &["task", "show", &id]);
    assert_eq!(code, 0, "показ задачи: stdout={show_out} stderr={err}");

    assert!(
        show_out.contains(&format!("коммит: {commit}")),
        "commit обязан лечь в поле коммита, а не быть перепутан с PR:\n{show_out}"
    );
    assert!(
        show_out.contains(&format!("PR: {pr_url}")),
        "pull_request обязан лечь в поле PR, а не быть перепутан с коммитом:\n{show_out}"
    );
    // Асимметрия: перепутанные аргументы дали бы коммит-строку со значением
    // PR — явно проверяем, что этого НЕ произошло.
    assert!(
        !show_out.contains(&format!("коммит: {pr_url}")),
        "коммит не обязан содержать значение PR (проводка перепутана):\n{show_out}"
    );
    assert!(
        !show_out.contains(&format!("PR: {commit}")),
        "PR не обязан содержать значение коммита (проводка перепутана):\n{show_out}"
    );
}

/// `au task activate` вытесняет прежнюю активную задачу того же проекта в
/// `backlog` и обязана сказать об этом вслух (T009) — молчаливое вытеснение
/// выглядит как потеря задачи.
#[test]
fn task_activate_evicts_previous_active_and_reports_it() {
    let home = TmpHome::dir("activate-evict");

    let (code, out, _) = run(
        &home,
        &["task", "new", "первая активная", "--project", "proj-evict"],
    );
    assert_eq!(code, 0);
    let first_id = created_task_id(&out);

    let (code, out, _) = run(
        &home,
        &["task", "new", "вторая активная", "--project", "proj-evict"],
    );
    assert_eq!(code, 0);
    let second_id = created_task_id(&out);

    let (code, out, err) = run(&home, &["task", "activate", &first_id]);
    assert_eq!(code, 0, "первая активация: stdout={out} stderr={err}");
    assert!(
        out.contains("Task activated"),
        "первая активация не должна упоминать вытеснение: {out}"
    );

    let (code, out, err) = run(&home, &["task", "activate", &second_id]);
    assert_eq!(code, 0, "вторая активация: stdout={out} stderr={err}");
    assert!(
        out.contains("вытеснена в backlog"),
        "вторая активация обязана сообщить о вытеснении первой: {out}"
    );
    assert!(
        out.contains("первая активная"),
        "сообщение обязано назвать именно вытесненную задачу: {out}"
    );

    // Первая реально ушла в backlog — не только текст сообщения.
    let (code, show_out, _) = run(&home, &["task", "show", &first_id]);
    assert_eq!(code, 0);
    assert!(
        show_out.contains("Status:   backlog"),
        "вытесненная задача обязана реально стать backlog:\n{show_out}"
    );
}

/// Сквозной успешный путь «улика → созревание → отказ»: `au task evidence`
/// (привязка через `--project`, без явного id — путь, которым пользуется
/// хук ulika), `au task ripe` (видит созревшую задачу) и `au task ripe
/// --decline` (снимает предъявление, не трогая саму задачу).
#[test]
fn task_ripe_shows_task_and_decline_removes_it_from_the_list() {
    let home = TmpHome::dir("ripe-decline");
    let project = "proj-ripe-flow";
    let project_dir = home.0.join(project);
    std::fs::create_dir_all(&project_dir).expect("рабочий каталог проекта");

    let (code, out, _) = run(
        &home,
        &["task", "new", "задача для созревания", "--project", project],
    );
    assert_eq!(code, 0);
    let id = created_task_id(&out);

    let (code, _, _) = run(&home, &["task", "activate", &id]);
    assert_eq!(code, 0);

    // Правка файла — из каталога проекта, чтобы `current_dir_name()` увидел
    // тот же проект и привязал `last_edit_at` к активной задаче (T012).
    let hook_payload = serde_json::json!({
        "session_id": "test-session",
        "tool_name": "Edit",
        "tool_input": { "file_path": "src/lib.rs" },
    })
    .to_string();
    let (code, _) = run_with_stdin_in(&home, &project_dir, &["trace", "--hook"], &hook_payload);
    assert_eq!(code, 0, "трейс правки обязан пройти без ошибки");

    // Улика без явного id — привязывается активной задаче названного
    // проекта (FR-008).
    let (code, evidence_out) = run_in(
        &home,
        &project_dir,
        &[
            "task",
            "evidence",
            "--project",
            project,
            "--command",
            "cargo test --workspace",
            "--exit",
            "0",
            "--json",
        ],
    );
    assert_eq!(code, 0, "улика: {evidence_out}");
    let evidence: serde_json::Value =
        serde_json::from_str(evidence_out.trim()).expect("JSON улики");
    assert_eq!(evidence["id"], id, "улика обязана уйти именно этой задаче");
    // 28.09.2026: улика живёт в задаче, зеркальный узел прогона не заводится.
    assert!(evidence.get("run_id").is_none(), "{evidence_out}");
    let conn = aurelius_core::db::open(&home.0.join("aurelius.db")).expect("открыть базу");
    let runs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE node_type = '\"run\"' AND deleted_at IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("счёт прогонов");
    assert_eq!(runs, 0, "узел прогона не пишется");

    let (code, ripe_out, err) = run(&home, &["task", "ripe", "--project", project, "--json"]);
    assert_eq!(code, 0, "ripe: {ripe_out} {err}");
    let ripe: serde_json::Value = serde_json::from_str(ripe_out.trim()).expect("JSON ripe");
    let ripe_arr = ripe.as_array().expect("ripe — массив");
    assert_eq!(
        ripe_arr.len(),
        1,
        "созревшая задача обязана быть предъявлена: {ripe_out}"
    );
    assert_eq!(ripe_arr[0]["id"], id);

    // Отказ — задача больше не предъявляется, но не удалена и не изменена
    // по статусу.
    let (code, decline_out, err) = run(&home, &["task", "ripe", "--decline", &id]);
    assert_eq!(code, 0, "decline: {decline_out} {err}");
    assert!(
        decline_out.contains("Отказ зафиксирован"),
        "decline обязан подтвердить действие текстом: {decline_out}"
    );

    let (code, ripe_out_after, _) = run(&home, &["task", "ripe", "--project", project, "--json"]);
    assert_eq!(code, 0);
    let ripe_after: serde_json::Value =
        serde_json::from_str(ripe_out_after.trim()).expect("JSON ripe после отказа");
    assert!(
        ripe_after.as_array().expect("массив").is_empty(),
        "после отказа задача не обязана предъявляться снова: {ripe_out_after}"
    );

    let (code, show_out, _) = run(&home, &["task", "show", &id]);
    assert_eq!(code, 0);
    assert!(
        show_out.contains("Status:   active"),
        "отказ от предъявления не обязан менять статус задачи:\n{show_out}"
    );
}

/// `au judge --hook` печатает блок созревших задач в режиме хука (T019,
/// FR-012) — тот самый путь, которым созревание доходит до ассистента, у
/// которого нет терминала для `au task ripe`.
#[test]
fn judge_hook_prints_ripe_block_for_ripe_task() {
    let home = TmpHome::dir("judge-hook");
    let project = "proj-judge-flow";
    let project_dir = home.0.join(project);
    std::fs::create_dir_all(&project_dir).expect("рабочий каталог проекта");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&project_dir)
            .args(args)
            .output()
            .expect("git fixture");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "--allow-empty",
        "-qm",
        "fixture",
    ]);

    let (code, out, _) = run(
        &home,
        &["task", "new", "задача под судью", "--project", project],
    );
    assert_eq!(code, 0);
    let id = created_task_id(&out);
    let (code, _, _) = run(&home, &["task", "activate", &id]);
    assert_eq!(code, 0);

    let hook_payload = serde_json::json!({
        "session_id": "test-session",
        "tool_name": "Edit",
        "tool_input": { "file_path": "src/lib.rs" },
    })
    .to_string();
    let (code, _) = run_with_stdin_in(&home, &project_dir, &["trace", "--hook"], &hook_payload);
    assert_eq!(code, 0);

    let (code, _) = run_in(
        &home,
        &project_dir,
        &[
            "task",
            "evidence",
            "--project",
            project,
            "--command",
            "cargo test --workspace",
            "--exit",
            "0",
            "--json",
        ],
    );
    assert_eq!(code, 0);

    let (code, judge_out) = run_in(&home, &project_dir, &["judge", "--hook"]);
    assert_eq!(code, 0, "judge --hook: {judge_out}");
    assert!(
        judge_out.contains("Созревшие задачи"),
        "блок созревших задач обязан появиться в выводе хука: {judge_out}"
    );
    assert!(
        judge_out.contains("задача под судью"),
        "блок обязан назвать именно эту задачу: {judge_out}"
    );
    assert!(judge_out.contains("готова к закрытию"));
    assert!(!judge_out.contains("src/lib.rs"));
    assert!(!judge_out.contains("cargo test --workspace"));
    let (code, detailed, _) = run(&home, &["task", "ripe", "--json"]);
    assert_eq!(code, 0);
    assert!(detailed.contains("cargo test --workspace"));
    assert!(detailed.contains("src/lib.rs"));
    let (code, from_payload) = run_with_stdin_in(
        &home,
        &home.0,
        &["judge", "--hook"],
        &serde_json::json!({"cwd": project_dir}).to_string(),
    );
    assert_eq!(code, 0);
    assert_eq!(from_payload, judge_out, "hook cwd wins over process cwd");
    let (code, from_ag) = run_with_stdin_in(
        &home,
        &home.0,
        &["judge", "--hook"],
        &serde_json::json!({"workspacePaths": [project_dir]}).to_string(),
    );
    assert_eq!(code, 0);
    assert_eq!(from_ag, judge_out);
    let nested = project_dir.join("src/nested");
    std::fs::create_dir_all(&nested).expect("nested cwd");
    let worktree = home.0.join("unrelated-worktree-name");
    git(&[
        "worktree",
        "add",
        "--detach",
        "-q",
        worktree.to_str().expect("fixture path"),
    ]);
    for cwd in [&nested, &worktree] {
        let (code, scoped) = run_with_stdin_in(
            &home,
            &home.0,
            &["judge", "--hook"],
            &serde_json::json!({"cwd": cwd}).to_string(),
        );
        assert_eq!(code, 0);
        assert_eq!(
            scoped, judge_out,
            "nested and worktree cwd share project identity"
        );
        let (code, snapshot) = run_with_stdin_in(
            &home,
            &home.0,
            &["snapshot", "--hook"],
            &serde_json::json!({"cwd": cwd}).to_string(),
        );
        assert_eq!(code, 0);
        assert!(snapshot.contains("задача под судью"));
    }
    for payload in [
        "{}".to_owned(),
        serde_json::json!({"cwd": home.0}).to_string(),
    ] {
        let (code, quiet) = run_with_stdin_in(&home, &project_dir, &["judge", "--hook"], &payload);
        assert_eq!(code, 0);
        assert!(
            quiet.is_empty(),
            "no global queue for missing or foreign project"
        );
        // Снапшот, в отличие от очереди судьи, без проекта не молчит: пустой
        // ответ хука будил сессию без памяти. Он отдаёт глобальный срез, так
        // и говорит, и не подставляет репозиторий каталога процесса.
        let (code, snapshot) =
            run_with_stdin_in(&home, &project_dir, &["snapshot", "--hook"], &payload);
        assert_eq!(code, 0);
        let json: serde_json::Value = serde_json::from_str(&snapshot).expect("hook JSON");
        let md = json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext");
        assert!(md.starts_with("# Память · глобально · "), "{md}");
        assert!(!md.contains("Репозиторий"), "{md}");
    }
}

#[test]
fn reminder_hook_does_not_consume_foreign_project_queue() {
    let home = TmpHome::dir("reminder-scope");
    let project = "reminder-current";
    let cwd = home.0.join(project);
    std::fs::create_dir_all(&cwd).expect("repo dir");
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&cwd)
        .status()
        .expect("git init")
        .success());
    for (name, project) in [
        ("foreign one", "other"),
        ("foreign two", "other"),
        ("foreign three", "other"),
        ("foreign four", "other"),
        ("own reminder", project),
    ] {
        let (code, out, err) = run(
            &home,
            &[
                "remind",
                "add",
                name,
                "--project",
                project,
                "--for",
                "ai",
                "--at",
                "2020-01-01T00:00:00Z",
            ],
        );
        assert_eq!(code, 0, "{out} {err}");
    }
    let (code, out) = run_with_stdin_in(
        &home,
        &home.0,
        &["remind", "--hook"],
        &serde_json::json!({"cwd": cwd, "hook_event_name":"UserPromptSubmit"}).to_string(),
    );
    assert_eq!(code, 0);
    assert!(out.contains("own reminder"));
    assert!(!out.contains("foreign"));
    let (code, pending, _) = run(&home, &["remind", "list", "--state", "pending", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(pending.matches("foreign").count(), 4);
    assert!(!pending.contains("own reminder"));
}

/// `au secret add/list/rm` — полный успешный путь: запись координаты,
/// чтение списка (без единого значения секрета) и удаление по имени.
#[test]
fn secret_add_list_rm_round_trip() {
    let home = TmpHome::dir("secret-flow");
    let project = "proj-secret-flow";
    let location = "1password://Private/Stripe/api-key";

    let (code, out, err) = run(
        &home,
        &[
            "secret",
            "add",
            "--name",
            "STRIPE_SECRET_KEY",
            "--where",
            location,
            "--purpose",
            "оплата подписки",
            "--project",
            project,
        ],
    );
    assert_eq!(code, 0, "запись координаты: {out} {err}");
    assert!(
        out.contains("Координата записана"),
        "успешная запись обязана подтвердиться текстом: {out}"
    );

    let (code, list_out, err) = run(&home, &["secret", "list", "--project", project, "--json"]);
    assert_eq!(code, 0, "список: {list_out} {err}");
    let refs: serde_json::Value = serde_json::from_str(list_out.trim()).expect("JSON списка");
    let arr = refs.as_array().expect("список — массив");
    assert_eq!(arr.len(), 1, "ровно одна координата: {list_out}");
    assert_eq!(arr[0]["name"], "STRIPE_SECRET_KEY");
    assert_eq!(arr[0]["location"], location);
    assert_eq!(arr[0]["purpose"], "оплата подписки");

    let (code, rm_out, err) = run(
        &home,
        &["secret", "rm", "STRIPE_SECRET_KEY", "--project", project],
    );
    assert_eq!(code, 0, "удаление: {rm_out} {err}");
    assert!(
        rm_out.contains("Координата удалена"),
        "успешное удаление обязано подтвердиться текстом: {rm_out}"
    );

    let (code, list_out_after, _) = run(&home, &["secret", "list", "--project", project, "--json"]);
    assert_eq!(code, 0);
    let refs_after: serde_json::Value =
        serde_json::from_str(list_out_after.trim()).expect("JSON списка после удаления");
    assert!(
        refs_after.as_array().expect("массив").is_empty(),
        "после удаления координат не обязано остаться: {list_out_after}"
    );
}
/// Временный НАСТОЯЩИЙ git-репозиторий под автоподстановку коммита: init,
/// минимальный cargo-проект (Cargo.toml обязателен — `ensure_indexed`
/// пропускает каталог без него, проект не индексируется, `project_root`
/// не находит путь и автодетект молчит), первый коммит и перевод HEAD на
/// ветку `feat/cli-probe`. Возвращает каталог и SHA первого коммита.
///
/// Имя каталога — короткий слаг БЕЗ «task» и без uuid: имя проекта ложится
/// в метку задачи `[<проект>] …`, которую рубеж на запись судит как один
/// кандидат до пробела — длинное имя (uuid сам по себе 36 символов) с
/// цифрами отклоняется как «случайный токен», а `sk-` прячется внутри
/// обычного слова «ta*sk-*…». Найдено 14.09.2026 на этом же тесте.
fn temp_git_repo(tag: &str) -> (std::path::PathBuf, String) {
    let repo = std::env::temp_dir().join(format!("au-probe-repo-{tag}"));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).expect("mkdir repo");
    let run_git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .expect("запустить git")
    };
    assert!(run_git(&["init", "-q"]).status.success());
    assert!(run_git(&["config", "user.email", "test@example.com"])
        .status
        .success());
    assert!(run_git(&["config", "user.name", "test"]).status.success());
    std::fs::create_dir_all(repo.join("src")).expect("mkdir src");
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"auto-probe\"\nversion = \"0.0.0\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(repo.join("src/lib.rs"), "").expect("write lib.rs");
    assert!(run_git(&["add", "."]).status.success());
    assert!(run_git(&["commit", "-q", "-m", "init"]).status.success());
    let expected_sha = String::from_utf8_lossy(&run_git(&["rev-parse", "--short", "HEAD"]).stdout)
        .trim()
        .to_owned();
    assert!(run_git(&["checkout", "-q", "-b", "feat/cli-probe"])
        .status
        .success());
    (repo, expected_sha)
}

/// Сессия сидит на чужой ветке — `au task done` без `--commit` молча записал
/// бы неуловимо чужой коммит. Автоподстановка остаётся (FR-004..006), но
/// обязана стать видимой: одна строка в stderr с SHA и веткой, а в записи —
/// имя ветки рядом с автоподставленным коммитом (`au task show` его
/// показывает).
#[test]
fn task_done_without_commit_prints_notice_and_stores_branch() {
    let home = TmpHome::dir("done-notice");
    let (repo, expected_sha) = temp_git_repo("notice");
    let project = repo
        .file_name()
        .and_then(|n| n.to_str())
        .expect("имя каталога");

    let (code, out, err) = run_in_full(
        &home,
        &repo,
        &["task", "new", "probe auto detection", "--project", project],
    );
    assert_eq!(code, 0, "создание задачи: stdout={out} stderr={err}");
    let id = created_task_id(&out);

    let (code, out, err) = run_in_full(&home, &repo, &["task", "activate", &id]);
    assert_eq!(code, 0, "активация: stdout={out} stderr={err}");

    let (code, out, err) = run_in_full(&home, &repo, &["task", "done", &id]);
    assert_eq!(code, 0, "закрытие задачи: stdout={out} stderr={err}");
    assert!(
        err.contains("определён автоматически"),
        "автоподстановка обязана быть названа в stderr: {err}"
    );
    assert!(
        err.contains(&expected_sha),
        "в примечании обязан быть сам SHA {expected_sha}: {err}"
    );
    assert!(
        err.contains("feat/cli-probe"),
        "в примечании обязана быть ветка HEAD: {err}"
    );

    let (code, show_out, err) = run_in_full(&home, &repo, &["task", "show", &id]);
    assert_eq!(code, 0, "показ задачи: stdout={show_out} stderr={err}");
    assert!(
        show_out.contains(&format!("    коммит: {expected_sha}")),
        "автоподставленный коммит обязан быть в записи:\n{show_out}"
    );
    assert!(
        show_out.contains("    ветка: feat/cli-probe"),
        "ветка автоподстановки обязана быть в записи и в show:\n{show_out}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Явно названный коммит — не автоподстановка: ни примечания в stderr, ни
/// ветки в записи (человек назвал сам, позиция HEAD ни при чём). Семантика
/// закрепляется тестом намеренно, это не случайность.
#[test]
fn task_done_with_explicit_commit_prints_no_notice() {
    let home = TmpHome::dir("done-explicit");
    let (repo, _expected_sha) = temp_git_repo("explicit");
    let project = repo
        .file_name()
        .and_then(|n| n.to_str())
        .expect("имя каталога");

    let (code, out, err) = run_in_full(
        &home,
        &repo,
        &["task", "new", "probe auto detection", "--project", project],
    );
    assert_eq!(code, 0, "создание задачи: stdout={out} stderr={err}");
    let id = created_task_id(&out);

    let (code, out, err) = run_in_full(&home, &repo, &["task", "activate", &id]);
    assert_eq!(code, 0, "активация: stdout={out} stderr={err}");

    let (code, out, err) = run_in_full(&home, &repo, &["task", "done", &id, "--commit", "deadbee"]);
    assert_eq!(code, 0, "закрытие задачи: stdout={out} stderr={err}");
    assert!(
        !err.contains("определён автоматически"),
        "при явном коммите примечания об автоподстановке быть не должно: {err}"
    );

    let (code, show_out, err) = run_in_full(&home, &repo, &["task", "show", &id]);
    assert_eq!(code, 0, "показ задачи: stdout={show_out} stderr={err}");
    assert!(
        show_out.contains("    коммит: deadbee"),
        "явный коммит обязан лечь в запись как есть:\n{show_out}"
    );
    assert!(
        !show_out.contains("    ветка:"),
        "явному коммиту ветка не положена:\n{show_out}"
    );

    std::fs::remove_dir_all(&repo).ok();
}
