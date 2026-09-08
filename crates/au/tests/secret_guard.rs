//! Рубеж перед графом (`add_node_full`, subject `aurelius:write:secret-guard`):
//! текст, похожий на значение секрета, отказывается кодом возврата, а не
//! ложится узлом молча. Граф append-only — вычистить записанный секрет
//! нечем, `memory_forget` уносит узел вместе со знанием, поэтому проверяется
//! именно ОТКАЗ (узел не создан), а не последующая чистка.
//!
//! Проверяется запуском настоящего бинаря: код возврата и то, что реально
//! легло в узлы, существуют только у процесса и его базы.

// Интеграционный тест — весь файл рантайм-путём не является; unwrap/expect
// здесь и есть сам способ проверки.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Stdio};

/// Рубеж на секрет (`aurelius_core::secret::SecretLookalikeRefused`,
/// `au::main::exit::SECRET_LOOKALIKE`) — не ошибка вызова и не ошибка
/// хранилища: вызов был правильным, отказал именно этот текст.
const SECRET_LOOKALIKE: i32 = 13;

/// Валидная по форме строка GitHub personal access token: префикс `ghp_` +
/// 36 символов — ровно та форма, что `secret::KNOWN_KEY_PREFIXES` и
/// GitHub-документация задают для этого типа ключей.
const GH_TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
/// Отличительный хвост токена — по нему проверяется, что отказ НЕ печатает
/// сам секрет (тест 4).
const GH_TOKEN_TAIL: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// Изолированный домен данных: свой `AURELIUS_HOME` на тест, чтобы настоящая
/// база владельца не была задета и тесты не наступали друг другу на ноги.
struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn dir(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("au-secret-guard-{tag}-{}", std::process::id()));
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
    // Без Cargo.toml/package.json во временном доме — авто-индексатор не
    // тронет базу собственной записью раньше проверяемого вызова.
    cmd.env("AURELIUS_HOME", &home.0)
        .current_dir(&home.0)
        .args(args);
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

/// Число живых узлов в базе временного дома — считается напрямую по
/// sqlite, а не через `au`: возврат отказавшей команды не обязан ничего
/// печатать, а факт «ничего не создано» проверяется независимо от него.
fn node_count(home: &TmpHome) -> i64 {
    let conn = aurelius_core::db::open(&home.0.join("aurelius.db")).expect("открыть базу");
    conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )
    .expect("посчитать узлы")
}

/// Данные единственного живого узла — для проверки, что маркер обхода
/// (`secret::BYPASS_MARKER_KEY`) действительно лёг в `data`.
fn only_node_data(home: &TmpHome) -> serde_json::Value {
    let conn = aurelius_core::db::open(&home.0.join("aurelius.db")).expect("открыть базу");
    let raw: String = conn
        .query_row(
            "SELECT data FROM nodes WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("прочитать data единственного узла");
    serde_json::from_str(&raw).expect("data — валидный JSON")
}

/// Акцептанс 1: `au note` с валидной по форме строкой GitHub-токена в тексте
/// отказывает своим кодом, и узел не создаётся — а не код 0 и тихая запись,
/// как было измерено 07.09.2026 (subject `aurelius:write:secret-guard`).
#[test]
fn note_with_github_token_shaped_text_is_refused_and_writes_nothing() {
    let home = TmpHome::dir("basic");
    let text = format!("рабочий токен для интеграции: {GH_TOKEN}");

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "похожий на секрет текст обязан отказать кодом {SECRET_LOOKALIKE}: stdout={out} stderr={err}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "отказанная запись не должна была создать узел"
    );
}

/// Акцептанс 2: тот же вызов с `--allow-secret` — обратная сторона отказа.
/// Код 0, узел лёг, и обход НЕ невидим: маркер сидит в `data` узла, так что
/// «какие записи легли с выключенным рубежом» — вопрос к графу, а не к логам.
#[test]
fn allow_secret_flag_bypasses_the_refusal_and_marks_the_node() {
    let home = TmpHome::dir("bypass");
    let text = format!("рабочий токен для интеграции: {GH_TOKEN}");

    let (code, out, err) = run(&home, &["note", "--allow-secret", &text]);
    assert_eq!(code, 0, "--allow-secret обязан пропустить запись: {err}");
    assert!(
        !out.is_empty() || err.is_empty(),
        "успешная запись: {out}{err}"
    );
    assert_eq!(node_count(&home), 1, "узел обязан быть создан");

    let data = only_node_data(&home);
    assert_eq!(
        data["secret_guard_bypassed"],
        serde_json::Value::Bool(true),
        "обход обязан быть отмечен в data узла, иначе он невидим постфактум: {data}"
    );
}

/// Акцептанс 3: токен, приклеенный к произвольному тексту БЕЗ разделителя ни
/// с одной стороны — дыра, найденная в `maskSecrets` замка (ulika): та
/// регулярка держится на `\b` и пропускает токен, склеенный со
/// словообразующим символом. Здесь такого якоря нет, и приклейка обязана
/// отказать точно так же, как токен сам по себе.
#[test]
fn token_glued_between_prefix_and_suffix_without_word_boundary_is_still_refused() {
    let home = TmpHome::dir("glued");
    let glued = format!("началоxyz{GH_TOKEN}zyxконец");

    let (code, out) = {
        let (c, o, e) = run(&home, &["note", &glued]);
        (c, format!("{o}{e}"))
    };
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "токен без границы слова обязан отказать тем же кодом: {out}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "приклеенный токен не должен был создать узел"
    );
}

/// Акцептанс 4, отдельным тестом по условию задачи: текст отказа НЕ содержит
/// подстроку самого токена. Отказ, печатающий в себе секрет, опровергает
/// собственное назначение — тем более что это сообщение уходит в stderr и
/// оттуда в журнал улик, откуда его, как и сам граф, не вычистить обратно.
#[test]
fn refusal_message_never_contains_the_token_substring() {
    let home = TmpHome::dir("no-leak");
    let text = format!("рабочий токен для интеграции: {GH_TOKEN}");

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(code, SECRET_LOOKALIKE, "предпосылка теста: отказ ожидается");
    assert!(
        !out.contains(GH_TOKEN_TAIL) && !err.contains(GH_TOKEN_TAIL),
        "отказ не должен печатать сам токен: stdout={out} stderr={err}"
    );
    assert!(
        !out.contains(GH_TOKEN) && !err.contains(GH_TOKEN),
        "отказ не должен печатать токен целиком: stdout={out} stderr={err}"
    );
}

// Ниже — пары дефектов 1 и 2 (07.09.2026, subject `aurelius:write:secret-guard`
// и `aurelius:write:secret-guard:false-positives`). Каждое поле, через
// которое текст попадает в узел, проверено ДВАЖДЫ: токен в нём обязан
// отказать, а обычный текст, на вид похожий на токен лишь коротким
// префиксом, обязан пройти. Все четыре старых акцептанс-теста выше проверяли
// только отказ — асимметрия, из-за которой ложные срабатывания на обычном
// английском ушли в установленный бинарь незамеченными.

/// Обычный позиционный текст без `--key`/провенанс-флагов: нейтральное тело
/// заметки для тестов полей `--claim`/`--evidence`/`--subject`/`--verify-with`,
/// чтобы токен, вложенный в проверяемое поле, не смешивался с тем, что и так
/// уже сканируется как `label`/`note`.
const BENIGN_BODY: &str = "ordinary status update about the rollout";

/// Пара 1а (`text`): токен прямо в позиционном тексте отказывает — то же, что
/// `note_with_github_token_shaped_text_is_refused_and_writes_nothing` выше,
/// здесь — для полноты пары рядом с её акцептансом.
#[test]
fn positional_text_with_token_is_refused() {
    let home = TmpHome::dir("field-text-refuse");
    let text = format!("token in the note body: {GH_TOKEN}");

    let (code, _, _) = run(&home, &["note", &text]);
    assert_eq!(code, SECRET_LOOKALIKE, "токен в тексте обязан отказать");
    assert_eq!(node_count(&home), 0, "отказанная запись не пишет узел");
}

/// Пара 1б (`text`): дефект 1. Три фразы, измеренные 07.09.2026 против
/// установленного бинаря — все три отказывали кодом 13, потому что `sk-`
/// ловился голой подстрокой внутри `mask-image`, `task-list`, `risk-free`.
#[test]
fn positional_text_resembling_token_is_accepted() {
    for text in [
        "CSS mask-image dropped silently",
        "the task-list rendering is off by one",
        "risk-free rollout plan",
    ] {
        let home = TmpHome::dir("field-text-accept");
        let (code, out, err) = run(&home, &["note", text]);
        assert_eq!(
            code, 0,
            "обычный текст не должен отказывать: {text}: stdout={out} stderr={err}"
        );
        assert_eq!(
            node_count(&home),
            1,
            "принятая запись обязана лечь узлом: {text}"
        );
    }
}

/// Пара 2а (`--claim`): дефект 2. Измерено 07.09.2026: `au note "тело"
/// --claim "<токен>"` уходило кодом 0, потому что рубеж судил только
/// `label`/`note`, а `claim` в узел попадает через `data`, минуя обе проверки.
#[test]
fn claim_with_token_is_refused() {
    let home = TmpHome::dir("field-claim-refuse");
    let claim = format!("leaked: {GH_TOKEN}");

    let (code, _, _) = run(&home, &["note", BENIGN_BODY, "--claim", &claim]);
    assert_eq!(code, SECRET_LOOKALIKE, "токен в --claim обязан отказать");
    assert_eq!(node_count(&home), 0, "отказанная запись не пишет узел");
}

/// Пара 2б (`--claim`): обычный текст, похожий на токен только коротким
/// префиксом, обязан лечь в `data.claim` без отказа.
#[test]
fn claim_resembling_token_is_accepted() {
    let home = TmpHome::dir("field-claim-accept");
    let claim = "the task-list rendering is off by one";

    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--claim", claim]);
    assert_eq!(code, 0, "обычный --claim не должен отказывать: {out}{err}");
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_data(&home)["claim"],
        serde_json::Value::String(claim.to_owned()),
        "claim обязан дойти до data без изменений"
    );
}

/// Пара 2в (`--evidence`): та же дыра, что и `--claim`, но для поля, куда
/// карточка `agent-checkpoint` инструктирует класть ДОСЛОВНУЮ команду —
/// самый вероятный носитель настоящего секрета среди всех полей.
#[test]
fn evidence_with_token_is_refused() {
    let home = TmpHome::dir("field-evidence-refuse");
    let evidence = format!("curl -H 'Authorization: Bearer {GH_TOKEN}' https://api.example.com");

    let (code, _, _) = run(&home, &["note", BENIGN_BODY, "--evidence", &evidence]);
    assert_eq!(code, SECRET_LOOKALIKE, "токен в --evidence обязан отказать");
    assert_eq!(node_count(&home), 0, "отказанная запись не пишет узел");
}

/// Пара 2г (`--evidence`): команда без токена, похожая на него лишь коротким
/// префиксом внутри обычного слова, обязана пройти.
#[test]
fn evidence_resembling_token_is_accepted() {
    let home = TmpHome::dir("field-evidence-accept");
    let evidence = "risk-free rollout plan";

    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--evidence", evidence]);
    assert_eq!(
        code, 0,
        "обычный --evidence не должен отказывать: {out}{err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_data(&home)["evidence"],
        serde_json::Value::String(evidence.to_owned()),
        "evidence обязан дойти до data без изменений"
    );
}

/// Пара 2д (`--subject`): тот же непроверенный канал `data`, что и у
/// `--claim`/`--evidence`.
#[test]
fn subject_with_token_is_refused() {
    let home = TmpHome::dir("field-subject-refuse");
    let subject = format!("integration:{GH_TOKEN}");

    let (code, _, _) = run(&home, &["note", BENIGN_BODY, "--subject", &subject]);
    assert_eq!(code, SECRET_LOOKALIKE, "токен в --subject обязан отказать");
    assert_eq!(node_count(&home), 0, "отказанная запись не пишет узел");
}

/// Пара 2е (`--subject`): предмет утверждения, похожий на токен лишь
/// коротким префиксом, обязан пройти.
#[test]
fn subject_resembling_token_is_accepted() {
    let home = TmpHome::dir("field-subject-accept");
    let subject = "CSS mask-image dropped silently";

    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--subject", subject]);
    assert_eq!(
        code, 0,
        "обычный --subject не должен отказывать: {out}{err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_data(&home)["subject"],
        serde_json::Value::String(subject.to_owned()),
        "subject обязан дойти до data без изменений"
    );
}

/// Пара 2ж (`--verify-with`): то же самое, что `--evidence` — тоже команда,
/// дословно; в задаче отдельно подчёркнуто, что этому полю нельзя быть вторым
/// сортом рядом с `evidence`.
#[test]
fn verify_with_token_is_refused() {
    let home = TmpHome::dir("field-verify-refuse");
    let verify_with = format!("curl -H 'Authorization: Bearer {GH_TOKEN}' https://api.example.com");

    let (code, _, _) = run(&home, &["note", BENIGN_BODY, "--verify-with", &verify_with]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "токен в --verify-with обязан отказать"
    );
    assert_eq!(node_count(&home), 0, "отказанная запись не пишет узел");
}

/// Пара 2з (`--verify-with`): команда без токена, похожая на него лишь
/// коротким префиксом, обязана пройти.
#[test]
fn verify_with_resembling_token_is_accepted() {
    let home = TmpHome::dir("field-verify-accept");
    let verify_with = "the task-list rendering is off by one";

    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--verify-with", verify_with]);
    assert_eq!(
        code, 0,
        "обычный --verify-with не должен отказывать: {out}{err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_data(&home)["verify_with"],
        serde_json::Value::String(verify_with.to_owned()),
        "verify_with обязан дойти до data без изменений"
    );
}

/// Дефект 2, обратная сторона: `data` целиком не сканируется, только четыре
/// именованных провенанс-поля. `--key` кладёт своё значение в `data.key` тем
/// же путём (`add_node_full`, вызванным из `upsert_node_by_key`) — здесь оно
/// сорокасимвольное значение формы AWS secret access key (base64-алфавит,
/// оба регистра, цифра), которое отказало бы, будь `data` просканирована
/// вслепую. Машинное поле — не текст, вписанный человеком, и не обязано
/// проходить проверку формы, придуманную для читаемого текста.
#[test]
fn machine_value_living_in_data_is_not_rejected() {
    let home = TmpHome::dir("machine-data");
    let machine_value = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--key", machine_value]);
    assert_eq!(
        code, 0,
        "машинное поле data.key не должно отказывать: {out}{err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_data(&home)["key"],
        serde_json::Value::String(machine_value.to_owned()),
        "машинное значение обязано дойти до data без изменений"
    );
}
