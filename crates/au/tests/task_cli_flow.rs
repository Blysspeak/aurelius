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

    let (code, judge_out, err) = run(&home, &["judge", "--hook"]);
    assert_eq!(code, 0, "judge --hook: {judge_out} {err}");
    assert!(
        judge_out.contains("Созревшие задачи"),
        "блок созревших задач обязан появиться в выводе хука: {judge_out}"
    );
    assert!(
        judge_out.contains("задача под судью"),
        "блок обязан назвать именно эту задачу: {judge_out}"
    );
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
