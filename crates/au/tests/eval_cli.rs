//! `au eval` — прогон, который завтра даёт то же число (спека 010, фаза A,
//! T011; `contracts/cli.md` §2, `contracts/eval-cases.md` §4).
//!
//! Проверяется запуском настоящего бинаря, а не вызовом функций: детерминизм
//! живёт у процесса. Побайтовое равенство двух прогонов, код возврата и след,
//! оставленный прогоном в фикстуре, снаружи видны, а изнутри — нет.
//!
//! Каждый тест собирает свой стенд в своём `AURELIUS_HOME`: три заметки, снимок
//! `au db backup` рядом и файл кейсов на него. Настоящая база владельца не
//! задета ни на чтение, ни на запись.
//!
//! Фикстура берётся снимком (`VACUUM INTO`), а не самим домашним файлом,
//! и это не формальность: домашний файл живёт в WAL, и read-only соединение к
//! нему трогает `-shm` — то есть пишет рядом с тем, что меряет. Снимок выходит
//! без WAL, и после прогона по нему на диске не появляется ни одного байта.

// Интеграционный тест — весь файл рантайм-путём не является; unwrap/expect
// здесь и есть сам способ проверки.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use std::path::Path;
use std::process::{Command, Stdio};

/// `au::main::exit::EVAL_NOT_COMPARABLE` — «прогон не состоялся, чисел нет».
/// Не ошибка вызова (1) и не ошибка хранилища (2): вызов был верным, база
/// цела, испорчена именно сопоставимость.
const NOT_COMPARABLE: i32 = 14;

/// `meta.as_of` стенда. Дата заведомо не сегодняшняя: прогон, взявший момент
/// из системных часов вместо параметра, отличается от правильного именно этим
/// полем, а не вердиктами.
const AS_OF: &str = "2019-03-04T05:06:07Z";

/// Значение `--now`. Отличается от [`AS_OF`] и тоже не сегодня.
const OTHER_NOW: &str = "2024-11-12T13:14:15Z";

/// Временный домен данных для одного теста, убирается при выходе.
struct TmpHome(std::path::PathBuf);

impl TmpHome {
    fn dir(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("au-eval-{tag}-{}", std::process::id()));
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
    cmd.env("AURELIUS_HOME", &home.0)
        // Каталог запуска — сам временный дом: `au note` индексирует текущий
        // каталог побочным эффектом (`commands::open_and_ensure`), и из дерева
        // репозитория фикстура набралась бы его файлами.
        .current_dir(&home.0)
        .args(args);
    cmd
}

/// Запустить и вернуть (код возврата, stdout, stderr). stderr нужен обоим
/// концам: в него уходит причина несостоявшегося прогона, и он же объясняет
/// упавшую сборку стенда.
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

/// Написать заметку и вернуть её `id`.
fn note(home: &TmpHome, node_type: &str, subject: &str, claim: &str, text: &str) -> String {
    let (code, out, err) = run(
        home,
        &[
            "note",
            "--json",
            "--project",
            "evaldemo",
            "--type",
            node_type,
            "--subject",
            subject,
            "--claim",
            claim,
            text,
        ],
    );
    assert_eq!(code, 0, "заметка не легла: {out}{err}");
    let saved: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON заметки");
    saved["id"].as_str().expect("id в JSON заметки").to_owned()
}

/// sha256 фикстуры, спрошенный у того же кода, который её сверяет.
///
/// Своего sha256 у теста нет: `sha2` в зависимостях `au` не числится, а заводить
/// его в манифест ради одной строки — менять то, что тест проверяет. Ответ
/// берётся типом, а не разбором текста ошибки: `verify_fixture` с заведомо не
/// той суммой возвращает `EvalRunFailed::FixtureChanged`, и фактическая сумма
/// лежит полем варианта.
///
/// Чего это НЕ проверяет: правильность самого sha256. Предмет теста — договор
/// командной строки («не сошлось — прогона нет»), а не хеш-функция; подмена в
/// [`tampered`] отличается от настоящей суммы одним знаком и остаётся
/// несовпадением, каким бы способом сумма ни считалась.
fn fixture_sha(path: &Path) -> String {
    let err = aurelius_core::eval::verify_fixture(path, "не сумма")
        .expect_err("такая «сумма» не может сойтись ни с одной фикстурой");
    let sha = match err.downcast_ref::<aurelius_core::eval::EvalRunFailed>() {
        Some(aurelius_core::eval::EvalRunFailed::FixtureChanged { actual, .. }) => actual.clone(),
        other => panic!("ожидался FixtureChanged, пришло {other:?}"),
    };
    assert_eq!(sha.len(), 64, "sha256 в hex — это 64 знака, пришло «{sha}»");
    sha
}

/// Та же сумма с одним изменённым знаком — ровно то, как выглядит подменённая
/// фикстура со стороны файла кейсов.
fn tampered(sha: &str) -> String {
    let head = if sha.starts_with('a') { 'b' } else { 'a' };
    format!("{head}{}", &sha[1..])
}

/// Кейсы стенда: один `recall_top5` и четыре `morphology` — два вида, которые
/// фаза A уже исполняет.
///
/// Вердикты намеренно разные: один провал и один пропуск обязательны. Прогон
/// из одних `PASS` не отличил бы отпечаток, считающий вердикты, от отпечатка,
/// считающего одни `id`, — и «два прогона дали тот же digest» ничего бы не
/// значило.
fn case_lines(wanted: &str) -> Vec<serde_json::Value> {
    vec![
        json!({
            "id": "recall-redis-cache",
            "kind": "recall_top5",
            "input": {"topic": "redis cache"},
            "expect": {"top5": [wanted], "mode": "any"},
            "why": "выведено командой au note --json этим же тестом строкой выше: \
                   узел заведён здесь, других кандидатов в фикстуре нет",
            "tags": ["by:derived"],
        }),
        json!({
            "id": "morph-redis",
            "kind": "morphology",
            "input": {"query": "redis"},
            "expect": {"non_empty": true},
            "why": "выведено командой au note --json этим же тестом: слово стоит в каждой \
                   из трёх заметок стенда",
            "tags": ["by:derived"],
        }),
        json!({
            "id": "morph-nothing",
            "kind": "morphology",
            "input": {"query": "kvakozyabram"},
            "expect": {"non_empty": false},
            "why": "выведено командой au note --json этим же тестом: слова нет ни в одной \
                   заметке стенда — отрицательный кейс против стеммера, матчащего всё со всем",
            "tags": ["by:derived"],
        }),
        json!({
            "id": "morph-star",
            "kind": "morphology",
            "input": {"query": "redis*"},
            "expect": {"non_empty": true},
            "why": "тест T011: звёздочка — подсказка префикса, а не словоформа; кейс обязан \
                   получить SKIP, и он держит в прогоне третий вердикт",
            "tags": ["by:derived"],
        }),
        json!({
            "id": "morph-wrong-on-purpose",
            "kind": "morphology",
            "input": {"query": "cache"},
            "expect": {"non_empty": false},
            "why": "тест T011: ожидание намеренно неверное — слово в фикстуре есть; кейс \
                   держит в прогоне FAIL, без которого отпечаток не проверяем",
            "tags": ["by:derived"],
        }),
    ]
}

/// Стенд: домен, снятая с него фикстура и файл кейсов на неё.
struct Bench {
    home: TmpHome,
    /// Пути строками: аргументы команды — `&str`.
    fixture: String,
    cases: String,
    sha: String,
    /// `id` узла, которого ждёт кейс `recall_top5`.
    wanted: String,
}

impl Bench {
    fn new(tag: &str) -> Self {
        let home = TmpHome::dir(tag);
        let wanted = note(
            &home,
            "decision",
            "eval demo redis cache",
            "redis holds the session cache",
            "the redis cache decision, written for the eval fixture",
        );
        note(
            &home,
            "problem",
            "eval demo stale cache",
            "the redis cache went stale after the deploy",
            "the stale cache problem, written for the eval fixture",
        );
        note(
            &home,
            "solution",
            "eval demo cache flush",
            "flush the redis cache on deploy",
            "the cache flush solution, written for the eval fixture",
        );

        let fixture = home.0.join("fixture.db").display().to_string();
        let (code, out, err) = run(&home, &["db", "backup", "--out", &fixture]);
        assert_eq!(code, 0, "снимок фикстуры не снялся: {out}{err}");
        let sha = fixture_sha(Path::new(&fixture));

        let cases = home.0.join("cases.jsonl").display().to_string();
        write_cases(&cases, &sha, &fixture, &case_lines(&wanted));
        Self {
            home,
            fixture,
            cases,
            sha,
            wanted,
        }
    }

    /// Прогон по названному файлу кейсов.
    ///
    /// `--db` называется всегда: без него `meta.fixture` разрешается от корня
    /// репозитория (`commands::eval_from_repo_root`), а фикстура стенда лежит
    /// во временном каталоге.
    fn eval(&self, cases: &str, extra: &[&str]) -> (i32, String, String) {
        let mut args = vec!["eval", cases, "--db", self.fixture.as_str()];
        args.extend_from_slice(extra);
        run(&self.home, &args)
    }

    /// Ещё один файл кейсов рядом с основным, с подменённой суммой или без
    /// единого кейса. Возвращает путь.
    fn variant(&self, name: &str, sha: &str, cases: &[serde_json::Value]) -> String {
        let path = self.home.0.join(name).display().to_string();
        write_cases(&path, sha, &self.fixture, cases);
        path
    }
}

/// Файл кейсов: строка `meta` первой, дальше по кейсу на строку
/// (`eval-cases.md` §1). `to_string` у `Value` даёт компактный JSON — ровно
/// один объект в строке, без висячих пробелов.
fn write_cases(path: &str, sha: &str, fixture: &str, cases: &[serde_json::Value]) {
    let meta = json!({
        "meta": {
            "version": 1,
            "as_of": AS_OF,
            "fixture": fixture,
            "fixture_sha256": sha,
        }
    });
    let mut text = format!("{meta}\n");
    for case in cases {
        text.push_str(&format!("{case}\n"));
    }
    std::fs::write(path, text).expect("записать файл кейсов");
}

/// Разобрать машинную форму отчёта.
fn report(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout.trim()).expect("JSON отчёта")
}

/// Главное обещание фазы: спор «стало лучше или хуже» получает число, и завтра
/// это число то же. Два прогона подряд — одни и те же байты и один и тот же
/// отпечаток; всё, что зависит от часов, счётчика показов или порядка строк
/// SQLite, ломает именно это равенство.
#[test]
fn two_runs_in_a_row_agree_byte_for_byte_and_on_the_digest() {
    let bench = Bench::new("determinism");

    let (code, human_1, err) = bench.eval(&bench.cases, &[]);
    assert_eq!(code, 0, "провал кейса — не ошибка прогона: {human_1}{err}");
    let (code, human_2, err) = bench.eval(&bench.cases, &[]);
    assert_eq!(code, 0, "второй прогон: {human_2}{err}");
    assert_eq!(
        human_1, human_2,
        "два прогона подряд обязаны дать побайтово тот же stdout"
    );

    let (code, json_1, err) = bench.eval(&bench.cases, &["--json"]);
    assert_eq!(code, 0, "машинная форма: {json_1}{err}");
    let (code, json_2, _) = bench.eval(&bench.cases, &["--json"]);
    assert_eq!(code, 0);
    assert_eq!(
        json_1, json_2,
        "машинная форма тоже обязана совпасть побайтово"
    );

    let first = report(&json_1);
    let second = report(&json_2);
    let digest = first["digest"].as_str().expect("digest строкой");
    assert!(!digest.is_empty(), "пустой digest сравнивать нечего");
    assert_eq!(digest, second["digest"], "digest двух прогонов подряд");

    // Равенство отпечатков что-то значит только на прогоне, где есть все три
    // вердикта: на одних `PASS` его не отличить от отпечатка, считающего `id`.
    assert_eq!(first["total"], 5, "кейсов стенда пять: {json_1}");
    assert!(
        first["passed"].as_u64().unwrap() >= 1,
        "ни одного PASS — стенд собран не тем: {json_1}"
    );
    // «Хотя бы один», а не «ровно один»: провалов может стать два, когда фаза B
    // поменяет порядок recall, и заложник чужой фазы из этого теста получился бы
    // зря — предмет проверки в том, что FAIL в прогоне ЕСТЬ.
    assert!(
        first["failed"].as_u64().unwrap() >= 1,
        "намеренно неверный кейс обязан провалиться: {json_1}"
    );
    assert_eq!(first["skipped"], 1, "кейс со звёздочкой: {json_1}");
    let (passed, failed, skipped) = (
        first["passed"].as_u64().unwrap(),
        first["failed"].as_u64().unwrap(),
        first["skipped"].as_u64().unwrap(),
    );
    assert_eq!(
        passed + failed + skipped,
        first["total"].as_u64().unwrap(),
        "пройдено + провалено + пропущено = всего: {json_1}"
    );
}

/// Момент — параметр, а не системные часы (FR-030, D-2 `eval-cases.md`):
/// `--now` его называет, без флага он берётся из `meta.as_of`.
///
/// Оба значения — заведомо не сегодняшняя дата, и это вся суть проверки:
/// прогон, дёрнувший `Utc::now()`, отличается от правильного именно этим полем.
/// Вердикты в фазе A от момента ещё не зависят — свежесть входит в порядок
/// вместе с `rank::score` в фазе B, — поэтому проверяется то, что уже
/// проверяемо: откуда прогон взял свои часы и что вывод от этого меняется.
#[test]
fn the_instant_comes_from_the_parameter_and_never_from_the_clock() {
    let bench = Bench::new("instant");

    let (code, from_meta, err) = bench.eval(&bench.cases, &["--json"]);
    assert_eq!(code, 0, "{from_meta}{err}");
    let default_run = report(&from_meta);
    assert_eq!(
        default_run["as_of"], AS_OF,
        "без --now момент берётся из meta.as_of: {from_meta}"
    );
    assert_eq!(default_run["now_source"], "meta.as_of");

    let (code, from_flag, err) = bench.eval(&bench.cases, &["--now", OTHER_NOW, "--json"]);
    assert_eq!(code, 0, "{from_flag}{err}");
    let flag_run = report(&from_flag);
    assert_eq!(
        flag_run["as_of"], OTHER_NOW,
        "--now перекрывает meta.as_of: {from_flag}"
    );
    assert_eq!(flag_run["now_source"], "--now");

    assert_ne!(
        from_meta, from_flag,
        "момент обязан быть виден в выводе, иначе он не параметр"
    );

    // Человеческая форма называет свои часы и их источник тем же образом:
    // отчёт читают глазами, и подразумеваемый момент — это отсутствующий.
    let (code, human, err) = bench.eval(&bench.cases, &["--now", OTHER_NOW]);
    assert_eq!(code, 0, "{human}{err}");
    assert!(
        human.contains(OTHER_NOW) && human.contains("(--now)"),
        "шапка обязана назвать момент и его источник: {human}"
    );
}

/// Подменённая `fixture_sha256` — прогон не состоялся: код 14 и ни одного
/// числа. Несравнимое число хуже отсутствующего: отсутствующее заставляет
/// прогнать заново, несравнимое молча ложится в `research.md` рядом с числом,
/// снятым на другой базе.
#[test]
fn a_fixture_that_does_not_match_prints_no_number_at_all() {
    let bench = Bench::new("tampered");
    // Кейсы те же самые: отличается ровно один знак суммы.
    let cases = bench.variant(
        "tampered.jsonl",
        &tampered(&bench.sha),
        &case_lines(&bench.wanted),
    );

    for form in [Vec::new(), vec!["--json"]] {
        let (code, out, err) = bench.eval(&cases, &form);
        assert_eq!(
            code, NOT_COMPARABLE,
            "не сошлась sha фикстуры — это 14, а не 1 и не 2: {out}{err}"
        );
        assert!(
            !out.chars().any(|c| c.is_ascii_digit()),
            "прогон не состоялся, а в stdout число: {out}"
        );
        assert!(
            !out.contains("digest"),
            "отпечаток несостоявшегося прогона печатать нечем: {out}"
        );
    }
}

/// Прогон не оставляет в фикстуре следа. Это не гигиена, а условие
/// повторяемости: `memory_recall` инкрементирует `access_count` каждому
/// показанному узлу (`graph::touch_node`), а он — множитель ранга, и один
/// прогон менял бы вход следующего. Кейс `recall_top5` в стенде есть, то есть
/// путь показа пройден по-настоящему.
#[test]
fn a_run_leaves_the_fixture_exactly_as_it_found_it() {
    let bench = Bench::new("readonly");
    let fixture = Path::new(&bench.fixture);

    let before = std::fs::metadata(fixture).expect("фикстура на месте");
    let bytes_before = std::fs::read(fixture).expect("прочитать фикстуру");

    let (code, out, err) = bench.eval(&bench.cases, &[]);
    assert_eq!(code, 0, "{out}{err}");
    let (code, out, err) = bench.eval(&bench.cases, &["--json"]);
    assert_eq!(code, 0, "{out}{err}");

    let after = std::fs::metadata(fixture).expect("фикстура на месте");
    assert_eq!(before.len(), after.len(), "размер фикстуры изменился");
    assert_eq!(
        before.modified().expect("mtime фикстуры"),
        after.modified().expect("mtime фикстуры"),
        "фикстуру писали: mtime сдвинулся"
    );
    assert_eq!(
        bytes_before,
        std::fs::read(fixture).expect("прочитать фикстуру"),
        "содержимое фикстуры изменилось"
    );

    // Ни `-wal`, ни `-shm` рядом не появилось: соединение, открытое на запись,
    // завело бы их первым же PRAGMA, и «фикстура не изменилась» стало бы
    // правдой только про сам файл.
    for sidecar in ["fixture.db-wal", "fixture.db-shm"] {
        assert!(
            !bench.home.0.join(sidecar).exists(),
            "прогон завёл {sidecar} — соединение было не только на чтение"
        );
    }
}

/// Файл из одной строки `meta` — «нет кейсов» и код 0. Ни ноль процентов, ни
/// паника: делить нечего, и знаменателем нулю не бывать (краевой случай спеки,
/// `eval-cases.md` §5 п. 3).
#[test]
fn a_case_file_with_only_a_meta_line_says_there_are_no_cases() {
    let bench = Bench::new("meta-only");
    let cases = bench.variant("meta-only.jsonl", &bench.sha, &[]);

    let (code, out, err) = bench.eval(&cases, &[]);
    assert_eq!(code, 0, "пустой набор — это не сбой прогона: {out}{err}");
    assert!(out.contains("кейсов 0:"), "учёт кейсов: {out}");
    assert_eq!(
        out.matches("нет кейсов").count(),
        5,
        "все пять видов обязаны сказать «нет кейсов»: {out}"
    );
    assert!(
        !out.contains(" 0/0 ") && !out.contains("0 %"),
        "ноль в знаменателе не печатается никогда: {out}"
    );
}
