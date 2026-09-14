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

/// Текст `note` единственного живого узла — сверяется, что полная заметка
/// легла без потерь независимо от того, как обрезалась метка (`label`).
fn only_node_note(home: &TmpHome) -> String {
    let conn = aurelius_core::db::open(&home.0.join("aurelius.db")).expect("открыть базу");
    conn.query_row(
        "SELECT note FROM nodes WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT 1",
        [],
        |r| r.get(0),
    )
    .expect("прочитать note единственного узла")
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

// Дефект 3 (найдено 11.09.2026, измерено на установленном бинаре с
// изолированным AURELIUS_HOME): git-хэш, UUID в скобках и квалифицированный
// идентификатор кода ложно отказывали в свободном тексте `au note`.

/// Репро 3а: git-хэш (SHA-1, 40 hex) в тексте заметки — не секрет, обязан
/// пройти без `--allow-secret`.
#[test]
fn note_with_git_sha1_hash_is_accepted() {
    for text in [
        "commit 7b86a7d98517479bbcd10998e74b292d763159dd fixed it",
        "see ff202359c32a6819358a9e9636b2284b98387c5f for the diff",
    ] {
        let home = TmpHome::dir("git-hash");
        let (code, out, err) = run(&home, &["note", text]);
        assert_eq!(
            code, 0,
            "git-хэш не должен отказывать: {text}: stdout={out} stderr={err}"
        );
        assert_eq!(
            node_count(&home),
            1,
            "принятая запись обязана лечь узлом: {text}"
        );
    }
}

/// Репро 3б: UUID, приклеенный к скобкам без пробела, — та же форма, что уже
/// проходит голой, но обёртка ломает распознавание.
#[test]
fn note_with_parenthesized_uuid_is_accepted() {
    let home = TmpHome::dir("uuid-paren");
    let text = "session id (449adf4b-26f9-4273-9e18-e16e638185f3) attached";

    let (code, out, err) = run(&home, &["note", text]);
    assert_eq!(
        code, 0,
        "UUID в скобках не должен отказывать: stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
}

/// Репро 3в: квалифицированный идентификатор кода (`Тип.метод`, опционально
/// с `()`), измерено как `SocketBusClient.request`.
#[test]
fn note_with_qualified_code_identifier_is_accepted() {
    for text in [
        "SocketBusClient.request failed with exit13",
        "SocketBusClient.request() failed with exit13",
    ] {
        let home = TmpHome::dir("qualified-id");
        let (code, out, err) = run(&home, &["note", text]);
        assert_eq!(
            code, 0,
            "идентификатор не должен отказывать: {text}: stdout={out} stderr={err}"
        );
        assert_eq!(
            node_count(&home),
            1,
            "принятая запись обязана лечь узлом: {text}"
        );
    }
}

/// Асимметрия: обёрточная пунктуация сама по себе поблажки не даёт — обычный
/// случайный токен под скобками отказывает так же, как без них. Фикстура
/// собрана `concat!` из двух частей, а не как цельный литерал.
#[test]
fn note_with_wrapped_generic_random_token_is_still_refused() {
    const GENERIC_RANDOM_TOKEN: &str = concat!("aZ9bQ7mK2xR5vN8p", "L1wT4Q9zK3");
    let home = TmpHome::dir("wrapped-random");
    let text = format!("leaked: ({GENERIC_RANDOM_TOKEN}) rotate it");

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "случайный токен в скобках обязан отказать: stdout={out} stderr={err}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "отказанная запись не должна была создать узел"
    );
}

/// Асимметрия: обёрнутый известный префикс по-прежнему ловится — поиск
/// префикса подстрокой по всему тексту обёрткой не задет.
#[test]
fn note_with_wrapped_known_prefix_is_still_refused() {
    let home = TmpHome::dir("wrapped-prefix");
    let text = format!("token ({GH_TOKEN}) leaked");

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "известный префикс в скобках обязан отказать: stdout={out} stderr={err}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "отказанная запись не должна была создать узел"
    );
}

// Приёмка 2 (11.09.2026, родитель): бэктики, пунктуация конца предложения на
// уже распознанных формах, и простой вызов `callee(arg)` всё ещё ложно
// отказывали в позиционном тексте и в провенанс-полях.

/// Репро: git-хэш, обёрнутый в бэктики (markdown-стиль code span).
#[test]
fn note_with_backtick_wrapped_hash_is_accepted() {
    let home = TmpHome::dir("backtick-hash");
    let text = "see `7b86a7d98517479bbcd10998e74b292d763159dd` for the diff";

    let (code, out, err) = run(&home, &["note", text]);
    assert_eq!(
        code, 0,
        "хэш в бэктиках не должен отказывать: stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
}

/// Репро: обычная пунктуация конца предложения на уже распознанных формах —
/// запятая после идентификатора, точка после UUID в скобках и после хэша.
#[test]
fn note_with_sentence_punctuation_after_safe_shapes_is_accepted() {
    for text in [
        "SocketBusClient.request, and it failed",
        "session id (449adf4b-26f9-4273-9e18-e16e638185f3).",
        "commit 7b86a7d98517479bbcd10998e74b292d763159dd.",
    ] {
        let home = TmpHome::dir("sentence-punct");
        let (code, out, err) = run(&home, &["note", text]);
        assert_eq!(
            code, 0,
            "форма с пунктуацией не должна отказывать: {text}: stdout={out} stderr={err}"
        );
        assert_eq!(
            node_count(&home),
            1,
            "принятая запись обязана лечь узлом: {text}"
        );
    }
}

/// Репро: `callee(arg)` — простой вызов, живая формулировка из отказанного
/// провенанс-поля (`createMapper(claude)`, 20 символов).
#[test]
fn note_with_simple_call_is_accepted() {
    let home = TmpHome::dir("simple-call");
    let text = "failed inside createMapper(claude) during init";

    let (code, out, err) = run(&home, &["note", text]);
    assert_eq!(
        code, 0,
        "простой вызов не должен отказывать: stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
}

/// Асимметрия: длинный/случайный callee или аргумент — уже не «простой
/// вызов», обязан отказать.
#[test]
fn note_with_simple_call_carrying_a_long_random_part_is_still_refused() {
    const GENERIC_RANDOM_TOKEN: &str = concat!("aZ9bQ7mK2xR5vN8p", "L1wT4Q9zK3");
    let home = TmpHome::dir("simple-call-random");
    let text = format!("failed inside createMapper({GENERIC_RANDOM_TOKEN}) during init");

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "вызов со случайным аргументом обязан отказать: stdout={out} stderr={err}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "отказанная запись не должна была создать узел"
    );
}

/// Репро в провенанс-поле, не только в теле заметки: бэктик-хэш в
/// `--claim` обязан пройти так же, как в позиционном тексте.
#[test]
fn claim_with_backtick_wrapped_hash_is_accepted() {
    let home = TmpHome::dir("claim-backtick-hash");
    let claim = "see `7b86a7d98517479bbcd10998e74b292d763159dd` for the diff";

    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--claim", claim]);
    assert_eq!(
        code, 0,
        "хэш в бэктиках в --claim не должен отказывать: stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
}

/// Репро в провенанс-поле: простой вызов `callee(arg)` в `--evidence`
/// обязан пройти так же, как в позиционном тексте.
#[test]
fn evidence_with_simple_call_is_accepted() {
    let home = TmpHome::dir("evidence-simple-call");
    let evidence = "failed inside createMapper(claude) during init";

    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--evidence", evidence]);
    assert_eq!(
        code, 0,
        "простой вызов в --evidence не должен отказывать: stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
}

// Дефект 4 (найдено 11.09.2026, subject
// `aurelius:write:secret-guard:label-truncation`): рубеж судит `label`
// независимо от `note` (`add_node_full`), а авто-метка резала текст слепо —
// `text.chars().take(60)`. Безопасная форма (git-хэш, UUID), пересечённая
// границей в 60 символов, превращалась в НЕузнаваемый обрубок, и рубеж
// отказывал по метке даже там, где полный текст был чист. Измерено: текст
// "word ".repeat(6) + 40-hex git-хэш + " fixed" отказывал кодом 13 — фикс
// режет метку по границе слова, а не по счётчику символов
// (`aurelius_core::graph::label_preview`).

const GIT_SHA1_HASH: &str = "7b86a7d98517479bbcd10998e74b292d763159dd";

/// Репро 4а: хэш, пересекающий старую границу метки в 60 символов на разных
/// смещениях (30/35/40 символов филлера перед хэшем — все три ложно
/// отказывали при старой слепой резке ровно потому, что обрубок хэша длиной
/// 20-30 символов был не короче `RANDOM_TOKEN_MIN_LEN`). Полный текст заметки
/// обязан остаться нетронутым, а маркер обхода — отсутствовать: фикс не
/// пропускает секреты мимо рубежа, он лишь чинит метку.
#[test]
fn note_with_hash_crossing_the_old_label_boundary_is_accepted() {
    for filler_words in [6usize, 7, 8] {
        let home = TmpHome::dir("hash-boundary");
        let text = format!("{}{GIT_SHA1_HASH} fixed", "word ".repeat(filler_words));

        let (code, out, err) = run(&home, &["note", &text]);
        assert_eq!(
            code, 0,
            "хэш, пересекающий старую границу метки, не должен отказывать \
             ({filler_words} слов филлера): stdout={out} stderr={err}"
        );
        assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
        assert_eq!(
            only_node_note(&home),
            text,
            "полный текст заметки обязан остаться нетронутым"
        );
        assert_eq!(
            only_node_data(&home).get("secret_guard_bypassed"),
            None,
            "рубеж обязан быть пройден честно, а не в обход"
        );
    }
}

/// Репро 4б: UUID в скобках, пересекающий старую границу метки — тот же
/// обрубок-без-закрывающей-скобки, что и у хэша, только с формой
/// `looks_like_uuid`, которую ломает уже сама незакрытая скобка.
#[test]
fn note_with_wrapped_uuid_crossing_the_old_label_boundary_is_accepted() {
    let home = TmpHome::dir("uuid-boundary");
    let text = format!(
        "{}(449adf4b-26f9-4273-9e18-e16e638185f3) attached",
        "word ".repeat(9)
    );

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, 0,
        "UUID в скобках, пересекающий старую границу метки, не должен отказывать: \
         stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_note(&home),
        text,
        "полный текст заметки обязан остаться нетронутым"
    );
}

/// Репро 4в: «голый» (не обёрнутый) SHA-256 — сам длиннее бюджета метки
/// (64 > 60) — посреди предложения длиннее 60 символов. Собран из
/// печатного hex-алфавита повтором, а не как хэш реального коммита.
#[test]
fn note_with_standalone_sha256_longer_than_the_label_budget_is_accepted() {
    let sha256 = "0123456789abcdef".repeat(4);
    assert_eq!(sha256.len(), 64, "предпосылка теста: ровно длина SHA-256");
    let home = TmpHome::dir("sha256-standalone");
    let text = format!("investigating regression: {sha256} across the board");

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, 0,
        "голый SHA-256 длиннее бюджета метки не должен отказывать: \
         stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_note(&home),
        text,
        "полный текст заметки обязан остаться нетронутым"
    );
}

/// Репро 4г: та же граница, но филлер — кириллица (многобайтовые символы в
/// UTF-8, один символ на `char`). Резка обязана считать `char`, а не байт, и
/// по-прежнему не разрубать хэш.
#[test]
fn note_with_cyrillic_filler_crossing_the_old_label_boundary_is_accepted() {
    let home = TmpHome::dir("cyrillic-boundary");
    let text = format!("{}{GIT_SHA1_HASH} готово", "слово ".repeat(7));

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, 0,
        "кириллический филлер вокруг старой границы метки не должен отказывать: \
         stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_note(&home),
        text,
        "полный текст заметки обязан остаться нетронутым, включая кириллицу"
    );
}

/// Асимметрия дефекта 4: чинится ДЕРИВАЦИЯ метки, а не рубеж. Настоящий
/// токен, лежащий в тексте ПОСЛЕ той точки, где новая (куда более короткая,
/// режущая по словам) метка уже оборвалась, обязан по-прежнему отказать —
/// потому что рубеж сканирует `note` целиком независимо от того, что попало
/// в `label`. Если бы это перестало работать, укорачивание метки само стало
/// бы дырой, через которую секрет проходит незамеченным.
#[test]
fn note_with_real_token_after_the_new_shorter_label_still_refused() {
    let home = TmpHome::dir("token-after-shorter-label");
    let text = format!("{}{GH_TOKEN}", "word ".repeat(15));

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "токен после укороченной метки обязан отказать так же, как и раньше: \
         stdout={out} stderr={err}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "отказанная запись не должна была создать узел"
    );
}

/// Репро 4д (найдено родителем 11.09.2026, приёмка ранее принятого фикса):
/// текст заметки — ЦЕЛИКОМ голый 64-hex, ни единого слова вокруг, ни единого
/// пробела. Первое (и единственное) слово само длиннее бюджета метки — тот
/// самый крайний случай, где предыдущая версия резала слово посимвольно
/// (`take(58)`) и получала 58-символьный обрубок, который уже не совпадает с
/// `HEX_HASH_LENS` (40/64) и ловится как случайный токен. Измерено родителем
/// на `'a1'.repeat(32)` (64 hex-валидных символа) — отказ на смещении 0.
/// Метка теперь не режет слово вовсе, отдаёт нейтральное многоточие.
#[test]
fn note_with_bare_64_hex_text_and_nothing_else_is_accepted() {
    let sha256 = "0123456789abcdef".repeat(4);
    assert_eq!(sha256.len(), 64, "предпосылка теста: ровно длина SHA-256");
    let home = TmpHome::dir("bare-64hex");

    let (code, out, err) = run(&home, &["note", &sha256]);
    assert_eq!(
        code, 0,
        "голый 64-hex без единого слова вокруг не должен отказывать: \
         stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_note(&home),
        sha256,
        "полный текст заметки обязан остаться нетронутым"
    );
    assert_eq!(
        only_node_data(&home).get("secret_guard_bypassed"),
        None,
        "рубеж обязан быть пройден честно, а не в обход"
    );
}

/// Асимметрия репро 4д: когда единственное «слово» текста — НАСТОЯЩИЙ похожий
/// на секрет токен (известный префикс `sk-` + длинный алфанумерик-хвост с
/// цифрой), длиннее бюджета метки, рубеж обязан отказать так же, как и до
/// фикса. Метка для такого текста становится голым многоточием и сама по
/// себе безобидна — отказ здесь может дать только скан `note` целиком, не
/// зависящий от того, что попало в `label`. Если бы этот тест прошёл кодом 0,
/// укорачивание метки само стало бы дырой для обхода рубежа.
#[test]
fn note_with_bare_long_secret_shaped_word_and_nothing_else_is_still_refused() {
    let secret_shaped = format!("sk-{}", "a1".repeat(35));
    assert!(
        secret_shaped.chars().count() > 60,
        "предпосылка теста: единственное слово длиннее бюджета метки"
    );
    let home = TmpHome::dir("bare-long-secret-word");

    let (code, out, err) = run(&home, &["note", &secret_shaped]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "голый секретоподобный токен без единого слова вокруг обязан отказать: \
         stdout={out} stderr={err}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "отказанная запись не должна была создать узел"
    );
}

// Заказ 14.09.2026 (subject
// `aurelius:crates/aurelius-core/src/secret.rs:camelcase-identifier`): живой
// отказ пришёл на голое camelCase-имя в поле `evidence` двух рабочих
// заметок. Обе заметки легли только через `--allow-secret` и несут
// `secret_guard_bypassed` в `data`; обе отказали на одном и том же имени, на
// смещении 55 и 384 байт. Ниже — пара «принять/отказать» для этой формы и
// полезная нагрузка заказа целиком, обезличенная с сохранением формы.

/// Значение `evidence` второй отказавшей заметки в обезличенном виде той же
/// формы: имя функции стоит на том же смещении 55, где заметка получила
/// код 13, и остаётся единственным кандидатом на отказ.
const CAMEL_EVIDENCE: &str = "python3 scan of src/widget.js: the single call site of calculateCartSummary sits inside the onClick of menu entry web-invoice-cart-calculate-action; the two render calls are the only layout calls in the widget";

/// Репро: та же строка в `--evidence` обязана лечь узлом без обхода. Если этот
/// тест красный, рубеж снова считает имя функции случайным токеном и заказ не
/// закрыт.
#[test]
fn evidence_with_camel_case_code_identifier_is_accepted() {
    let home = TmpHome::dir("camel-evidence");
    let (code, out, err) = run(&home, &["note", BENIGN_BODY, "--evidence", CAMEL_EVIDENCE]);
    assert_eq!(
        code, 0,
        "camelCase-имя в --evidence не должно отказывать: stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_data(&home)["evidence"],
        serde_json::Value::String(CAMEL_EVIDENCE.to_owned()),
        "evidence обязан дойти до data без изменений"
    );
    assert_eq!(
        only_node_data(&home).get("secret_guard_bypassed"),
        None,
        "рубеж обязан быть пройден честно, а не в обход"
    );
}

/// Та же форма в позиционном тексте — скан `note` не должен расходиться со
/// сканом `data`.
#[test]
fn positional_text_with_camel_case_code_identifier_is_accepted() {
    for text in [
        "the single call site of calculateCartSummary sits inside the onClick",
        "field latestMonthlyReportRows is written before the callback",
        "computeFontDimensions is called before the write",
    ] {
        let home = TmpHome::dir("camel-text");
        let (code, out, err) = run(&home, &["note", text]);
        assert_eq!(
            code, 0,
            "camelCase-имя не должно отказывать: {text}: stdout={out} stderr={err}"
        );
        assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    }
}

/// Асимметрия: поблажка дана рисунку регистра, а не «строке из букв». Строка
/// той же длины, где строчные идут по одной между заглавными, — по-прежнему
/// случайный токен. Литерал собран `concat!`, чтобы цельный
/// credential-подобный текст не лежал в исходнике.
#[test]
fn note_with_random_mixed_case_word_is_still_refused() {
    const RANDOM_WORD: &str = concat!("aZbQmKxRvNpLwTs", "HdGfQr");
    let home = TmpHome::dir("random-mixed-case");
    let text = format!("leaked: {RANDOM_WORD} rotate it");

    let (code, out, err) = run(&home, &["note", &text]);
    assert_eq!(
        code, SECRET_LOOKALIKE,
        "случайная строка обязана отказать: stdout={out} stderr={err}"
    );
    assert_eq!(
        node_count(&home),
        0,
        "отказанная запись не должна была создать узел"
    );
}

// Полезная нагрузка заказа (отчёт от 14.09.2026) в обезличенном виде той же
// формы: заметка, её `claim`, `subject` и `evidence`. Хранится
// целиком, потому что приёмка требует прогнать через настоящий путь именно её,
// а не отдельные слова: `note` — 1865 байт, смещение 926 в ней попадает на
// обычное слово из кириллицы, и рубеж обязан принять все поля без обхода.
const ORDER_PAYLOAD_NOTE: &str = r#"ASKED: выяснить, почему ночные снимки теплицы попадают в семейный альбом без подписи. WHY: по подписи с датой и влажностью сверяют рост рассады, серия без неё бесполезна. FOUND: подпись ставит сборщик кадров, а миниатюры режет отдельный модуль превью, который о подписи не знает; хук theme.override в шаблоне альбома срабатывает раньше модуля, и при каждом пересчёте миниатюра собирается из чистого кадра. Модуль превью взят из форка 04Harbor17/web-img-gallery. NEXT: неделю гонять ночные серии на форке с исправленным порядком хуков и считать кадры без подписи.

Стенд: камера над грядками снимает раз в десять минут. Сборщик сам кладёт файлы в каталог альбома, а pix-web.service раз в час пересобирает страницы. Модуль превью версии 1.4.2, тема закрепляет 1.4.0 через link 2.1.0, так что обновлять приходится в двух местах. Поле lazyThumb в config.toml включено по умолчанию и откладывает пересчёт до первого просмотра, поэтому пропажа подписи видна только утром. Отвергнуто: править шаблон ядра, ставить подпись вторым проходом по готовым миниатюрам, выключить превью целиком."#;
const ORDER_PAYLOAD_CLAIM: &str = r#"Подпись на миниатюрах альбома теряется не в ядре: хук theme.override срабатывает раньше модуля превью, и миниатюра собирается из чистого кадра до наложения подписи"#;
const ORDER_PAYLOAD_SUBJECT: &str = r#"backyard-gallery:web-gui:thumbnailing-ru"#;
const ORDER_PAYLOAD_EVIDENCE: &str = r#"git -C workSpace/project/web-img-gallery log --oneline; yarn-version обоих пакетов темы; grep -c по EXIF в собранном бандле превью img-gallery даёт 0"#;

/// Приёмка заказа: все четыре поля обязаны лечь одним узлом, код 0, без
/// маркера обхода.
#[test]
fn order_payload_is_accepted_end_to_end() {
    let home = TmpHome::dir("order-payload");
    let (code, out, err) = run(
        &home,
        &[
            "note",
            ORDER_PAYLOAD_NOTE,
            "--claim",
            ORDER_PAYLOAD_CLAIM,
            "--subject",
            ORDER_PAYLOAD_SUBJECT,
            "--evidence",
            ORDER_PAYLOAD_EVIDENCE,
        ],
    );
    assert_eq!(
        code, 0,
        "полезная нагрузка заказа обязана пройти: stdout={out} stderr={err}"
    );
    assert_eq!(node_count(&home), 1, "принятая запись обязана лечь узлом");
    assert_eq!(
        only_node_note(&home),
        ORDER_PAYLOAD_NOTE,
        "заметка обязана дойти до узла без потерь"
    );
    let data = only_node_data(&home);
    assert_eq!(
        data["claim"],
        serde_json::Value::String(ORDER_PAYLOAD_CLAIM.to_owned())
    );
    assert_eq!(
        data["subject"],
        serde_json::Value::String(ORDER_PAYLOAD_SUBJECT.to_owned())
    );
    assert_eq!(
        data["evidence"],
        serde_json::Value::String(ORDER_PAYLOAD_EVIDENCE.to_owned())
    );
    assert_eq!(
        data.get("secret_guard_bypassed"),
        None,
        "рубеж обязан быть пройден честно, а не в обход"
    );
}
