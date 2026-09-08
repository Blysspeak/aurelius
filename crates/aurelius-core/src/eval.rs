//! Прогон кейсов по замороженной фикстуре: спор «стало лучше или хуже»
//! получает число, и это число повторяется завтра.
//!
//! Формат файла кейсов и правила судейства нормативны в
//! `specs/010-waking-memory/contracts/eval-cases.md` (§1, §2, §4); типы —
//! `data-model.md` §4. Здесь они исполняются, а не переопределяются.
//!
//! Модуль живёт в ядре, а не в `au`, ровно затем, чтобы судья звал боевые
//! функции ([`crate::graph::recall_selection`], [`crate::graph::search_ranked`]),
//! а не их копию: крейт `aurelius` зависит от ядра, обратной зависимости нет,
//! и пока сборка выдачи лежала в MCP-обработчике, eval мерил бы не тот путь,
//! которым пользуется владелец.
//!
//! **`Utc::now()` в этом модуле запрещён (FR-030).** Момент прогона приходит
//! параметром `now` — из `meta.as_of` файла кейсов либо из флага `--now`. Без
//! внешнего момента множитель свежести (фаза B) считался бы от системных
//! часов, и та же фикстура через неделю давала бы другие числа: у пары
//! близких кандидатов порядок на границе пятой позиции переворачивается, и
//! разница чисел «до» и «после» перестаёт что-либо означать.
//!
//! Фикстура открывается **только на чтение** (`db::open_readonly`): показ
//! узла инкрементирует `access_count`, а он — множитель ранга, то есть первый
//! прогон менял бы вход второго. Прогон, которому потребовалась запись, —
//! провал прогона, а не кейса.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

use crate::graph;
use crate::models::{Node, NodeType};

/// Версия формата файла кейсов, которую понимает этот бинарник.
///
/// Незнакомая версия — не «попробуем разобрать что получится», а
/// несостоявшийся прогон: файл написан под другой контракт, и его кейсы
/// нельзя считать понятыми даже поодиночке.
pub const FORMAT_VERSION: u32 = 1;

/// Сколько первых записей выдачи судит `recall_top5`.
const TOP_N: usize = 5;

/// Глубина обхода recall по умолчанию — та же, что у боевого `memory_recall`
/// (`crates/aurelius/src/mcp/handlers/session.rs`). Судья не имеет права
/// ходить глубже читателя: он мерил бы выдачу, которой никто не видит.
const DEFAULT_RECALL_DEPTH: u32 = 2;

/// Предел выдачи для `morphology` — тот же, что у `au search`
/// (`crates/au/src/commands.rs`, `graph::search_ranked(&conn, query, 20)`).
/// Вид проверяет находимость, а не порядок, поэтому предел здесь щедрее
/// пятёрки топа и совпадает с тем, что видит человек в командной строке.
const MORPHOLOGY_LIMIT: usize = 20;

/// Сколько шестнадцатеричных знаков отпечатка печатает отчёт.
const DIGEST_HEX_LEN: usize = 16;

/// Прогон не состоялся: чисел нет и печатать нечего.
///
/// Отдельный тип, а не строка внутри `anyhow`: командная строка отдаёт на
/// него код возврата **14**, отличая «файл понят, и именно поэтому видно, что
/// прогон несравним» от «файл не понят» (код 1) и от «база недоступна»
/// (код 2). Признавать этот случай по тексту сообщения запрещено — ветка
/// `classify()` смотрит на тип.
///
/// Несравнимое число хуже отсутствующего: отсутствующее заставляет прогнать
/// заново, несравнимое молча ложится в `research.md` рядом с числом, снятым
/// на другой базе.
#[derive(Debug, thiserror::Error)]
pub enum EvalRunFailed {
    /// `meta.version` из будущего или из другого контракта.
    #[error(
        "файл кейсов версии {found}, а этот бинарник понимает только версию {FORMAT_VERSION} — \
         прогон не состоялся"
    )]
    UnknownVersion { found: u32 },

    /// sha256 распакованной фикстуры не сошёлся с `meta.fixture_sha256`.
    /// Сверяется до первого кейса: числа, снятые на другой базе, несравнимы с
    /// прежними, и молча сравнить их хуже, чем не сравнить вовсе.
    #[error("фикстура не та: ожидался sha256 {expected}, у файла {actual} — прогон не состоялся")]
    FixtureChanged { expected: String, actual: String },

    /// Кейс вида, который этот бинарник ещё не исполняет.
    ///
    /// **Это не `SKIP`.** `SKIP` в контракте означает «кейс непригоден на этой
    /// фикстуре» (узел вырезан обезличиванием) и не входит ни в числитель, ни
    /// в знаменатель — то есть доля остаётся честной. Неисполненная проверка,
    /// зачтённая как `SKIP`, наоборот, делает долю ложной: она молча объявляет
    /// сто процентов там, где не проверялось ничего.
    #[error("кейс «{id}»: вид {kind} этот бинарник ещё не исполняет — прогон не состоялся")]
    KindNotExecutable { id: String, kind: &'static str },
}

// ── Файл кейсов ──────────────────────────────────────────────────────────

/// Первая строка файла. Отдельный тип, а не вариант [`EvalCase`]: у неё нет ни
/// `id`, ни `kind`, и общий построчный разбор на ней падает.
///
/// Обёртка `{"meta": {…}}` выбрана вместо шестого значения `kind` затем, чтобы
/// у дискриминатора кейсов не было значения, которое кейсом не является:
/// разбор первой строки и разбор кейса — два разных разбора, и общее поле их
/// бы склеило.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MetaLine {
    pub meta: EvalMeta,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EvalMeta {
    /// Версия формата этого контракта. Сейчас 1. Незнакомая версия — отказ
    /// прогона, а не попытка разобрать «как получится».
    pub version: u32,
    /// Часы прогона: момент, относительно которого считается возраст узлов.
    /// Живёт в одном месте, а не в каждом кейсе: пятьдесят пять копий одной
    /// даты — это пятьдесят пять шансов разойтись.
    pub as_of: DateTime<Utc>,
    /// Путь к распакованной базе относительно корня репозитория.
    pub fixture: String,
    /// sha256 распакованного `.db`, hex. Сверяется ДО первого кейса: на
    /// порядок выдачи влияет содержимое базы, а не байты архива.
    pub fixture_sha256: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EvalCase {
    /// Стабильный слаг, kebab-case, ASCII. Уникален в файле; освободившийся
    /// после удаления кейса `id` мёртв навсегда — на него ссылаются отчёты.
    pub id: String,
    /// Одна строка прозы: кто назвал этот ответ верным и когда. Печатается в
    /// отчёте о провале и в вердикте не участвует — это единственное место
    /// кейса, где живёт человеческое суждение.
    pub why: String,
    /// Метки кейса. Ровно одна из пространства `by:` обязательна
    /// (`by:owner` | `by:derived` | `by:agent:<метка>`) — по ней отчёт строит
    /// разбивку авторства (FR-027); остальные метки группируют и на вердикт
    /// не влияют.
    pub tags: Vec<String>,
    /// Дискриминатор — `kind`. Внутренне тегированное перечисление: строка
    /// файла остаётся плоским объектом с ключами `kind`/`input`/`expect`.
    #[serde(flatten)]
    pub body: CaseBody,
}

impl EvalCase {
    /// Вид проверки — то, чем кейс считается в учёте отчёта.
    #[must_use]
    pub fn kind(&self) -> CaseKind {
        match self.body {
            CaseBody::RecallTop5 { .. } => CaseKind::RecallTop5,
            CaseBody::Morphology { .. } => CaseKind::Morphology,
            CaseBody::Refusal { .. } => CaseKind::Refusal,
            CaseBody::SnapshotContains { .. } => CaseKind::SnapshotContains,
            CaseBody::RecognizeFeed { .. } => CaseKind::RecognizeFeed,
        }
    }

    /// Метка авторства из пространства `by:` — ровно одна на кейс.
    fn author_tag(&self) -> Option<&str> {
        let mut found = None;
        for tag in &self.tags {
            if tag.starts_with("by:") {
                if found.is_some() {
                    return None;
                }
                found = Some(tag.as_str());
            }
        }
        found
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaseBody {
    /// Нужный узел входит в топ-5 recall по теме (SC-002, SC-002a, FR-007…009).
    RecallTop5 {
        input: RecallInput,
        expect: RecallExpect,
    },
    /// Запрос в косвенном падеже даёт непустую выдачу (SC-003, FR-020, FR-023).
    Morphology {
        input: MorphologyInput,
        expect: MorphologyExpect,
    },
    /// Слишком широкая тема получает отказ, а не ответ (SC-001, FR-006).
    Refusal {
        input: RefusalInput,
        expect: RefusalExpect,
    },
    /// Снимок содержит ожидаемое, блок стоит первым и влезает в бюджет
    /// (FR-001…FR-005, SC-006).
    SnapshotContains {
        input: SnapshotInput,
        expect: SnapshotExpect,
    },
    /// Подача узнавания называет что надо и молчит где надо (FR-014…FR-018).
    RecognizeFeed {
        input: RecognizeInput,
        expect: RecognizeExpect,
    },
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecallInput {
    pub topic: String,
    /// Область поиска. Сегодня боевой recall по проекту не фильтрует, поэтому
    /// поле работает ровно как условие пригодности кейса: названного проекта в
    /// фикстуре нет — `SKIP`. Судья не заводит фильтра, которого нет у
    /// читателя: он мерил бы собственную выдумку.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub depth: Option<u32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecallExpect {
    /// UUID узлов фикстуры.
    pub top5: Vec<Uuid>,
    /// `any` (умолчание) — хотя бы один из `top5` на позициях 1…5; `all` — все.
    #[serde(default)]
    pub mode: Option<TopMode>,
    /// SC-002a: ни одного узла этих типов на позициях 1…5.
    #[serde(default)]
    pub forbid_types: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MorphologyInput {
    pub query: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MorphologyExpect {
    pub non_empty: bool,
    /// Если задан — этот UUID присутствует на любой позиции, не только в топ-5.
    #[serde(default)]
    pub contains: Option<Uuid>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RefusalInput {
    pub topic: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RefusalExpect {
    pub refused: bool,
    /// Ожидаемый повод отказа: `topic_too_broad` | `topic_too_narrow`.
    #[serde(default)]
    pub reason: Option<String>,
    /// Порог снизу, а не равенство: фаза E меняет индекс и поднимет df почти
    /// всех тем.
    #[serde(default)]
    pub matched_at_least: Option<i64>,
    /// Подстрока совета. Самим текстом совета владеет `contracts/cli.md` §4.
    #[serde(default)]
    pub advice_contains: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SnapshotInput {
    pub project: String,
    /// `markdown` (умолчание) — то, что отдаёт `build_snapshot`; `json` — поле
    /// `state` машинной формы, которое собирает `snapshot_facts`.
    #[serde(default)]
    pub form: Option<SnapshotForm>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotForm {
    Markdown,
    Json,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SnapshotExpect {
    /// `true` — блок «где остановились» стоит ПЕРВОЙ секцией; `false` — блока
    /// в снимке нет вовсе (проект без трассы, FR-005).
    #[serde(default)]
    pub first_section: Option<bool>,
    /// Все подстроки обязаны найтись, побайтово.
    #[serde(default)]
    pub contains: Vec<String>,
    /// Ни одной из этих подстрок нет. Здесь живут UUID и наносекунды.
    #[serde(default)]
    pub absent: Vec<String>,
    /// Длина блока в символах не больше указанной — прокси бюджета SC-006.
    #[serde(default)]
    pub max_chars: Option<usize>,
    /// Только при `form: json`: каждый ключ есть в объекте `state` и не `null`.
    #[serde(default)]
    pub state_keys: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecognizeInput {
    /// Ключ называется `prompt`, как поле события `UserPromptSubmit`, а не
    /// `message`: кейс подаёт ровно то, что придёт хуку.
    pub prompt: String,
    #[serde(default)]
    pub project: Option<String>,
    /// Каталог, по которому определяется проект, когда `project` не задан.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecognizeExpect {
    /// Истина — подача обязана быть пустой (ноль байт).
    pub empty: bool,
    /// Имена сущностей, обязанные прозвучать в подаче.
    #[serde(default)]
    pub names: Vec<String>,
    /// `any` (умолчание) — хотя бы одно имя из `names`; `all` — все.
    #[serde(default)]
    pub names_mode: Option<TopMode>,
    /// Ни одно из этих имён прозвучать не имеет права (FR-015).
    #[serde(default)]
    pub forbid_names: Vec<String>,
    /// Прокси бюджета FR-016: 50 токенов ≈ 150 символов.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// Сколько ожидаемых значений обязано совпасть.
///
/// Умолчание `any` — не мягкость, а следствие устройства графа:
/// `--resolution supersede|refine|coexist` не прячет старый узел, а строит
/// линию, так что об одном факте законно лежат два-три узла, и кейс,
/// назвавший только новейший, провалился бы по причине, не имеющей отношения
/// к ранжированию.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopMode {
    Any,
    All,
}

/// Пять видов проверки — других нет. Значение вне этих пяти читается serde как
/// неразобранная строка, то есть сломанный файл.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseKind {
    RecallTop5,
    Morphology,
    Refusal,
    SnapshotContains,
    RecognizeFeed,
}

impl CaseKind {
    /// Порядок этого массива — порядок строк отчёта и ключей `by_kind` в
    /// `--json`. Он задан здесь, а не на печати, чтобы у двух форм отчёта не
    /// разошёлся порядок видов.
    pub const ALL: [CaseKind; 5] = [
        CaseKind::RecallTop5,
        CaseKind::Morphology,
        CaseKind::Refusal,
        CaseKind::SnapshotContains,
        CaseKind::RecognizeFeed,
    ];

    /// Имя вида ровно то, что стоит в `kind` строки файла.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CaseKind::RecallTop5 => "recall_top5",
            CaseKind::Morphology => "morphology",
            CaseKind::Refusal => "refusal",
            CaseKind::SnapshotContains => "snapshot_contains",
            CaseKind::RecognizeFeed => "recognize_feed",
        }
    }

    /// Место вида в [`CaseKind::ALL`] и в массиве учёта.
    #[must_use]
    fn index(self) -> usize {
        match self {
            CaseKind::RecallTop5 => 0,
            CaseKind::Morphology => 1,
            CaseKind::Refusal => 2,
            CaseKind::SnapshotContains => 3,
            CaseKind::RecognizeFeed => 4,
        }
    }
}

/// Виды, которые исполняет этот бинарник. Остальные три ждут своих фаз
/// (C — снимок, D и G — подача, F — отказ) и до тех пор роняют прогон, а не
/// пополняют долю.
const EXECUTABLE_KINDS: &[CaseKind] = &[CaseKind::RecallTop5, CaseKind::Morphology];

// ── Загрузка ─────────────────────────────────────────────────────────────

/// Первая строка — [`MetaLine`], остальные — [`EvalCase`].
///
/// Битая строка называет свой номер в файле (нумерация с 1, вместе с `meta`).
/// Пустой файл — ошибка «нет строки `meta`», а не «ноль кейсов»: файл без
/// часов прогона невоспроизводим по построению. Файл из одной строки `meta` —
/// законные ноль кейсов, а не отказ.
pub fn load(path: &Path) -> Result<(EvalMeta, Vec<EvalCase>)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("не читается файл кейсов: {}", path.display()))?;
    parse(&text)
}

/// Разбор содержимого файла кейсов. Вынесен из [`load`] затем, чтобы правила
/// первой строки проверялись без файловой системы.
fn parse(text: &str) -> Result<(EvalMeta, Vec<EvalCase>)> {
    let mut lines = text.lines().enumerate();
    let (_, first) = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("файл кейсов пуст: нет строки meta"))?;
    let meta: MetaLine = serde_json::from_str(first).with_context(|| {
        "строка 1 не разбирается как meta — первой строкой файла кейсов обязан стоять \
         объект {\"meta\": {…}}"
            .to_string()
    })?;
    let meta = meta.meta;
    if meta.version != FORMAT_VERSION {
        return Err(EvalRunFailed::UnknownVersion {
            found: meta.version,
        }
        .into());
    }

    let mut cases: Vec<EvalCase> = Vec::new();
    for (idx, line) in lines {
        // Нумерация человеческая, с единицы и вместе с meta: номер из отчёта
        // должен совпадать с номером в редакторе, иначе он не помогает.
        let number = idx + 1;
        if line.trim().is_empty() {
            anyhow::bail!("строка {number}: пустые строки в файле кейсов запрещены");
        }
        let case: EvalCase = serde_json::from_str(line)
            .with_context(|| format!("строка {number} не разбирается как кейс"))?;
        // Дубль `id` — сломанный файл, а не проваленный кейс: отчёт и
        // `research.md` ссылаются на `id`, и повторённый идентификатор
        // превращает историю чисел в ложь, которую нечем поймать.
        if cases.iter().any(|c| c.id == case.id) {
            anyhow::bail!("строка {number}: id «{}» уже занят выше по файлу", case.id);
        }
        // Без метки авторства разбивка отчёта (FR-027) перестаёт что-либо
        // значить: набор, наполовину сочинённый агентом, который в базу не
        // смотрел, выглядит так же, как снятый с живой базы владельцем.
        if case.author_tag().is_none() {
            anyhow::bail!(
                "строка {number}: у кейса «{}» нет ровно одной метки авторства \
                 (by:owner | by:derived | by:agent:<метка>)",
                case.id
            );
        }
        cases.push(case);
    }
    Ok((meta, cases))
}

// ── Сверка фикстуры ──────────────────────────────────────────────────────

/// sha256 распакованной фикстуры против `meta.fixture_sha256`.
///
/// Зовётся **до первого кейса** и до открытия соединения: числа, снятые на
/// другой базе, несравнимы с прежними, и половина отчёта, напечатанная до
/// того, как несовпадение вскрылось, — это ровно то несравнимое число,
/// которое потом ляжет в `research.md`.
///
/// Считается по `.db`, а не по `.db.zst`: на порядок выдачи влияет содержимое
/// базы, а не байты архива.
pub fn verify_fixture(db_path: &Path, expected_sha256: &str) -> Result<()> {
    let actual = sha256_file(db_path)?;
    // Регистр hex ничего не значит, поэтому сравнение по нему не судит.
    if !actual.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err(EvalRunFailed::FixtureChanged {
            expected: expected_sha256.trim().to_owned(),
            actual,
        }
        .into());
    }
    Ok(())
}

/// sha256 файла целиком, hex в нижнем регистре. Читается кусками: фикстура —
/// это десятки мегабайт, и целиком в память её класть незачем.
fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .with_context(|| format!("не открывается фикстура: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buf)
            .with_context(|| format!("не читается фикстура: {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ── Вердикты и отчёт ─────────────────────────────────────────────────────

/// Вердикт кейса — ровно один из трёх.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    /// Кейс непригоден на этой фикстуре: ожидаемый узел вырезан
    /// обезличиванием, названного проекта в фикстуре нет, вход пуст.
    ///
    /// **Не входит ни в числитель, ни в знаменатель.** Причина отдельного
    /// вердикта: обезличивание (FR-027) вырезает узлы, и без `SKIP`
    /// вырезанный узел читался бы как регрессия ранжирования.
    Skip,
}

impl Verdict {
    /// Имя вердикта в отчёте и в отпечатке.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Skip => "SKIP",
        }
    }
}

/// Что стало с одним кейсом. Порядок в отчёте — порядок строк файла.
#[derive(Debug, Clone)]
pub struct CaseOutcome {
    pub id: String,
    pub kind: CaseKind,
    pub verdict: Verdict,
    /// Причина пропуска или разбор провала — «ожидалось / пришло» для печати
    /// (FR-026). У `PASS` пусто: объяснять нечего.
    pub detail: Option<String>,
}

/// Учёт одного вида. `eligible` не поле, а вычисление: два числа, обязанные
/// совпадать, рано или поздно расходятся.
#[derive(Debug, Clone, Copy)]
pub struct KindTally {
    pub kind: CaseKind,
    pub cases: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl KindTally {
    /// Знаменатель доли: все кейсы вида минус его пропуски.
    #[must_use]
    pub fn eligible(&self) -> usize {
        self.cases - self.skipped
    }

    /// Есть ли что делить. Ноль в знаменателе не печатается никогда и паникой
    /// не является: `нет кейсов` и `непригодно (N)` — два разных состояния, и
    /// различает их `cases`.
    #[must_use]
    pub fn has_cases(&self) -> bool {
        self.cases > 0
    }
}

/// Итог прогона. Долю в процентах не несёт намеренно: округление рядом с
/// целыми — это второй источник истины.
#[derive(Debug, Clone)]
pub struct EvalReport {
    /// Момент, от которого считался прогон, — тот самый параметр `now`.
    /// Печатается в шапке, чтобы отчёт называл свои часы, а не подразумевал их.
    pub now: DateTime<Utc>,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Учёт по видам в порядке [`CaseKind::ALL`].
    pub by_kind: [KindTally; 5],
    /// По кейсу на строку файла, в порядке файла: `outcomes[i]` — про
    /// `cases[i]`, поэтому печать провала берёт `why` и метки из самого кейса.
    pub outcomes: Vec<CaseOutcome>,
    /// sha256 отсортированного по `id` списка «`<id> <вердикт>`», первые
    /// [`DIGEST_HEX_LEN`] знаков.
    pub digest: String,
}

impl EvalReport {
    /// Учёт одного вида.
    #[must_use]
    pub fn tally(&self, kind: CaseKind) -> &KindTally {
        &self.by_kind[kind.index()]
    }
}

/// Отпечаток прогона: sha256 по отсортированному списку «`<id> <вердикт>`».
///
/// Сортировка по `id`, а не по порядку файла, — затем, чтобы перестановка
/// строк в файле не меняла число: порядок строк на результат не влияет, и
/// отпечаток обязан говорить то же самое. Строка на пару, разделитель —
/// перевод строки; байты фиксированы, потому что отпечаток печатается и
/// сравнивается между прогонами.
fn digest_of(outcomes: &[CaseOutcome]) -> String {
    let mut pairs: Vec<(&str, Verdict)> = outcomes
        .iter()
        .map(|o| (o.id.as_str(), o.verdict))
        .collect();
    pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut hasher = Sha256::new();
    for (id, verdict) in pairs {
        hasher.update(id.as_bytes());
        hasher.update(b" ");
        hasher.update(verdict.as_str().as_bytes());
        hasher.update(b"\n");
    }
    let full = format!("{:x}", hasher.finalize());
    full.chars().take(DIGEST_HEX_LEN).collect()
}

/// Арифметика отчёта, отдельно от судейства: её проверяет юнит-тест без базы.
fn tally_up(outcomes: Vec<CaseOutcome>, now: DateTime<Utc>) -> EvalReport {
    let mut by_kind = CaseKind::ALL.map(|kind| KindTally {
        kind,
        cases: 0,
        passed: 0,
        failed: 0,
        skipped: 0,
    });
    let (mut passed, mut failed, mut skipped) = (0usize, 0usize, 0usize);
    for outcome in &outcomes {
        let tally = &mut by_kind[outcome.kind.index()];
        tally.cases += 1;
        match outcome.verdict {
            Verdict::Pass => {
                tally.passed += 1;
                passed += 1;
            }
            Verdict::Fail => {
                tally.failed += 1;
                failed += 1;
            }
            Verdict::Skip => {
                tally.skipped += 1;
                skipped += 1;
            }
        }
    }
    EvalReport {
        now,
        total: outcomes.len(),
        passed,
        failed,
        skipped,
        by_kind,
        digest: digest_of(&outcomes),
        outcomes,
    }
}

// ── Прогон ───────────────────────────────────────────────────────────────

/// Прогон кейсов по фикстуре.
///
/// `now` подаётся снаружи — `meta.as_of` либо флаг `--now`; `Utc::now()`
/// внутри модуля запрещён (FR-030). Соединение обязано быть открыто
/// `db::open_readonly`, а `verify_fixture` — вызван до сюда: обе гарантии
/// живут у вызывающего, потому что сюда приходит уже открытое соединение.
///
/// Порядок кейсов — порядок строк файла, никакой параллельности: судейство
/// одного кейса не должно зависеть от того, чем занят соседний.
///
/// Отличие от нормативной сигнатуры `data-model.md` §4: параметра
/// `w: &RankWeights` здесь по-прежнему нет. T018 перевёл путь recall
/// (`graph::recall_selection`) на `rank::score`, но там веса берутся
/// `RankWeights::default()` на месте, а не приходят сюда параметром — эта
/// строка `run()` до калибровки не дотянута и остаётся известным пробелом
/// (см. отчёт агента волны T018): довести до нормативной формы значило бы
/// протащить `w` ещё и через `judge`/`judge_recall_top5`, и через CLI-вызов
/// в `au` (`commands.rs`), что уже вне зоны той правки.
pub fn run(
    conn: &Connection,
    meta: &EvalMeta,
    cases: &[EvalCase],
    now: DateTime<Utc>,
) -> Result<EvalReport> {
    // Версию проверяет и `load`, но `run` — публичная функция и не вправе
    // считать, что кейсы пришли именно оттуда.
    if meta.version != FORMAT_VERSION {
        return Err(EvalRunFailed::UnknownVersion {
            found: meta.version,
        }
        .into());
    }
    // Неисполнимый вид ловится до первого кейса: половина отчёта, напечатанная
    // перед отказом, — это те же числа, которых по контракту быть не должно.
    for case in cases {
        let kind = case.kind();
        if !EXECUTABLE_KINDS.contains(&kind) {
            return Err(EvalRunFailed::KindNotExecutable {
                id: case.id.clone(),
                kind: kind.as_str(),
            }
            .into());
        }
    }

    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let (verdict, detail) = judge(conn, case, now)?;
        outcomes.push(CaseOutcome {
            id: case.id.clone(),
            kind: case.kind(),
            verdict,
            detail,
        });
    }
    Ok(tally_up(outcomes, now))
}

/// Вердикт одного кейса. Ошибка отсюда — сбой хранилища, а не провал кейса:
/// «база не ответила» и «ответ не тот» — разные вещи, и код возврата у них
/// разный.
fn judge(
    conn: &Connection,
    case: &EvalCase,
    now: DateTime<Utc>,
) -> Result<(Verdict, Option<String>)> {
    // Похожее на секрет вырезается из выдачи раньше ранжирования
    // (`graph::search`, `nodes.retain(|n| !secret::is_secret_ref(n))`), так
    // что ожидать такой узел в топ-5 бессмысленно, а хранить его текст в
    // репозитории — тем более.
    if let Some(text) = case_texts(case)
        .into_iter()
        .find(|t| crate::secret::scan_text_for_lookalike(t).is_some())
    {
        let head: String = text.chars().take(24).collect();
        return Ok((
            Verdict::Skip,
            Some(format!(
                "во входе или ожидании кейса есть похожее на секрет («{head}…») — \
                 такой узел вырезается из выдачи до ранжирования"
            )),
        ));
    }
    match &case.body {
        CaseBody::RecallTop5 { input, expect } => judge_recall_top5(conn, input, expect, now),
        CaseBody::Morphology { input, expect } => judge_morphology(conn, input, expect),
        // Сюда не дойти: неисполнимые виды роняют прогон в `run` до судейства.
        // Ветка существует затем, чтобы шестой вид, добавленный в enum, не
        // прошёл молча.
        other => Err(EvalRunFailed::KindNotExecutable {
            id: case.id.clone(),
            kind: match other {
                CaseBody::Refusal { .. } => CaseKind::Refusal.as_str(),
                CaseBody::SnapshotContains { .. } => CaseKind::SnapshotContains.as_str(),
                _ => CaseKind::RecognizeFeed.as_str(),
            },
        }
        .into()),
    }
}

/// Тексты кейса, снятые с живой базы: их и проверяет заслонка секретов.
/// Виды, которых этот бинарник не исполняет, до сюда не доходят.
fn case_texts(case: &EvalCase) -> Vec<&str> {
    match &case.body {
        CaseBody::RecallTop5 { input, .. } => {
            let mut texts = vec![input.topic.as_str()];
            if let Some(project) = &input.project {
                texts.push(project.as_str());
            }
            texts
        }
        CaseBody::Morphology { input, .. } => vec![input.query.as_str()],
        _ => Vec::new(),
    }
}

/// `recall_top5` — правила целиком в `eval-cases.md` §2.1.
///
/// Топ-5 — первые пять записей той последовательности, которую recall
/// показывает читателю: `knowledge ++ recent` в этом порядке. Позиция 6 —
/// провал, позиция 12 — провал: частичного зачёта нет, но отчёт называет
/// фактическую позицию, потому что «ранжирование промахнулось» и «поиск не
/// нашёл» чинятся в разных фазах.
fn judge_recall_top5(
    conn: &Connection,
    input: &RecallInput,
    expect: &RecallExpect,
    now: DateTime<Utc>,
) -> Result<(Verdict, Option<String>)> {
    if input.topic.trim().is_empty() {
        return Ok((
            Verdict::Skip,
            Some("вход пуст: topic — пустая строка".into()),
        ));
    }
    if expect.top5.is_empty() {
        return Ok((
            Verdict::Skip,
            Some("ожидание пусто: top5 не называет ни одного узла".into()),
        ));
    }
    if let Some(project) = &input.project {
        if graph::find_project_by_label(conn, project)?.is_none() {
            return Ok((
                Verdict::Skip,
                Some(format!("проекта «{project}» в фикстуре нет")),
            ));
        }
    }

    // Ожидаемый узел, вырезанный обезличиванием, — это непригодный кейс, а не
    // регрессия ранжирования.
    let mut missing: Vec<Uuid> = Vec::new();
    for id in &expect.top5 {
        if graph::get_node(conn, &id.to_string())?.is_none() {
            missing.push(*id);
        }
    }
    let mode = expect.mode.unwrap_or(TopMode::Any);
    let unusable = match mode {
        TopMode::Any => missing.len() == expect.top5.len(),
        TopMode::All => !missing.is_empty(),
    };
    if unusable {
        return Ok((
            Verdict::Skip,
            Some(format!(
                "ожидаемых узлов нет в фикстуре: {}",
                short_ids(&missing)
            )),
        ));
    }

    let depth = input.depth.unwrap_or(DEFAULT_RECALL_DEPTH);
    let selection = graph::recall_selection(conn, &input.topic, depth, now)?;
    let shown: Vec<&Node> = selection
        .knowledge
        .iter()
        .chain(selection.recent.iter())
        .collect();
    let top: Vec<&Node> = shown.iter().take(TOP_N).copied().collect();

    let hits = expect
        .top5
        .iter()
        .filter(|id| top.iter().any(|n| n.id == **id))
        .count();
    let enough = match mode {
        TopMode::Any => hits > 0,
        TopMode::All => hits == expect.top5.len(),
    };
    if !enough {
        return Ok((Verdict::Fail, Some(positions_report(&expect.top5, &shown))));
    }

    // SC-002a механически: машинный узел на верхних пяти строках отменяет
    // попадание, каким бы точным оно ни было.
    if !expect.forbid_types.is_empty() {
        let offenders: Vec<String> = top
            .iter()
            .filter(|n| expect.forbid_types.iter().any(|t| *t == type_name(n)))
            .map(|n| format!("[{}] {}", type_name(n), n.label))
            .collect();
        if !offenders.is_empty() {
            return Ok((
                Verdict::Fail,
                Some(format!(
                    "запрещённые типы в топ-5: {}",
                    offenders.join("; ")
                )),
            ));
        }
    }
    Ok((Verdict::Pass, None))
}

/// `morphology` — правила целиком в `eval-cases.md` §2.2.
///
/// Запрос со звёздочкой — `SKIP`: хвостовая звёздочка это подсказка префикса
/// (`fts::parse` сохраняет `"redis"*`), то есть ровно тот костыль, который
/// стемминг снимает. Кейс со звёздочкой измерял бы обход, а не исправление.
fn judge_morphology(
    conn: &Connection,
    input: &MorphologyInput,
    expect: &MorphologyExpect,
) -> Result<(Verdict, Option<String>)> {
    if input.query.contains('*') {
        return Ok((
            Verdict::Skip,
            Some("в запросе звёздочка: это подсказка префикса, а не словоформа".into()),
        ));
    }
    if input.query.trim().is_empty() {
        return Ok((
            Verdict::Skip,
            Some("вход пуст: query — пустая строка".into()),
        ));
    }
    if let Some(id) = expect.contains {
        if graph::get_node(conn, &id.to_string())?.is_none() {
            return Ok((
                Verdict::Skip,
                Some(format!("ожидаемого узла {} нет в фикстуре", short_id(&id))),
            ));
        }
    }

    let outcome = graph::search_ranked(conn, &input.query, MORPHOLOGY_LIMIT)?;
    if !expect.non_empty {
        return Ok(if outcome.nodes.is_empty() {
            (Verdict::Pass, None)
        } else {
            (
                Verdict::Fail,
                Some(format!(
                    "ожидалась пустая выдача, пришло {} записей, первая — [{}] {}",
                    outcome.nodes.len(),
                    outcome
                        .nodes
                        .first()
                        .map(type_name)
                        .unwrap_or_else(|| "?".into()),
                    outcome.nodes.first().map_or("?", |n| n.label.as_str())
                )),
            )
        });
    }
    if outcome.nodes.is_empty() {
        return Ok((
            Verdict::Fail,
            Some(format!(
                "0 записей; unmatched_terms: [{}]",
                outcome.unmatched_terms.join(", ")
            )),
        ));
    }
    if let Some(id) = expect.contains {
        if !outcome.nodes.iter().any(|n| n.id == id) {
            return Ok((
                Verdict::Fail,
                Some(format!(
                    "{} записей, но {} среди них нет",
                    outcome.nodes.len(),
                    short_id(&id)
                )),
            ));
        }
    }
    Ok((Verdict::Pass, None))
}

/// Фактические позиции ожидаемых узлов во всей выдаче: «на позиции 9 из 12»
/// или «не найден». Разница между этими двумя ответами — это разница между
/// фазой B (ранжирование промахнулось) и фазой E (поиск не нашёл).
fn positions_report(expected: &[Uuid], shown: &[&Node]) -> String {
    let total = shown.len();
    let parts: Vec<String> = expected
        .iter()
        .map(|id| match shown.iter().position(|n| n.id == *id) {
            Some(idx) => format!("{} на позиции {} из {total}", short_id(id), idx + 1),
            None => format!("{} не найден", short_id(id)),
        })
        .collect();
    parts.join("; ")
}

/// Голова UUID: отчёт читает человек, и полные тридцать шесть знаков в строке
/// провала не помогают, а мешают.
fn short_id(id: &Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

fn short_ids(ids: &[Uuid]) -> String {
    ids.iter().map(short_id).collect::<Vec<_>>().join(", ")
}

/// Имя типа узла — то, каким его печатает выдача и каким его пишет кейс.
///
/// Через serde, а не через `Debug`: `WorkLog` в файле кейсов называется
/// `work_log`, и `format!("{:?}").to_lowercase()` дал бы `worklog`. Тип `run` —
/// это `NodeType::Custom("run")`, а не вариант перечисления, и его имя лежит
/// внутри варианта: `serde_json` завернул бы его в `{"custom":"run"}`, поэтому
/// он разобран отдельной веткой.
fn type_name(node: &Node) -> String {
    match &node.node_type {
        NodeType::Custom(name) => name.clone(),
        known => serde_json::to_value(known)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = r#"{"meta":{"version":1,"as_of":"2026-09-08T00:00:00Z","fixture":"fixtures/eval/aurelius-2026-09-08.db","fixture_sha256":"3f9c1b7a"}}"#;
    const CASE_RECALL: &str = r#"{"id":"recall-a","kind":"recall_top5","input":{"topic":"лесенка замка"},"expect":{"top5":["8c1d0f52-3b7a-4d19-9f2e-6a0c4b18d7e5"]},"why":"владелец, 08.09.2026","tags":["by:owner"]}"#;
    const CASE_MORPH: &str = r#"{"id":"morph-a","kind":"morphology","input":{"query":"перезагрузкам"},"expect":{"non_empty":true},"why":"владелец, 08.09.2026","tags":["by:owner","phase-E"]}"#;

    fn outcome(id: &str, kind: CaseKind, verdict: Verdict) -> CaseOutcome {
        CaseOutcome {
            id: id.to_owned(),
            kind,
            verdict,
            detail: None,
        }
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .expect("тестовая дата")
            .with_timezone(&Utc)
    }

    /// Первая строка разбирается своим типом — и наоборот: построчный разбор
    /// как кейса на ней падает. Это и есть причина, по которой `MetaLine`
    /// существует отдельно от `EvalCase`.
    #[test]
    fn meta_line_parses_as_meta_and_never_as_a_case() {
        let meta: MetaLine = serde_json::from_str(META).expect("meta разбирается");
        assert_eq!(meta.meta.version, 1);
        assert_eq!(meta.meta.as_of, at("2026-09-08T00:00:00Z"));
        assert_eq!(meta.meta.fixture_sha256, "3f9c1b7a");

        assert!(
            serde_json::from_str::<EvalCase>(META).is_err(),
            "у строки meta нет ни id, ни kind — как кейс она обязана не разбираться"
        );
        assert!(
            serde_json::from_str::<MetaLine>(CASE_RECALL).is_err(),
            "кейс не обязан разбираться как meta"
        );
    }

    /// Файл из одной строки meta — законные ноль кейсов, а не отказ.
    #[test]
    fn meta_only_file_loads_with_zero_cases() {
        let (meta, cases) = parse(META).expect("файл из одной meta");
        assert_eq!(meta.version, FORMAT_VERSION);
        assert!(cases.is_empty());
    }

    #[test]
    fn empty_file_names_the_missing_meta_line() {
        let err = parse("").expect_err("пустой файл — отказ");
        assert!(
            format!("{err:#}").contains("нет строки meta"),
            "пустой файл обязан жаловаться на meta, а не на ноль кейсов: {err:#}"
        );
    }

    /// Незнакомая версия — несостоявшийся прогон, а не проваленный кейс:
    /// отдельный тип ошибки, по которому командная строка отдаёт код 14.
    #[test]
    fn unknown_meta_version_refuses_the_whole_run() {
        let meta = META.replace("\"version\":1", "\"version\":2");
        let text = format!("{meta}\n{CASE_MORPH}");
        let err = parse(&text).expect_err("версия 2 не понимается");
        let refused = err
            .chain()
            .find_map(|c| c.downcast_ref::<EvalRunFailed>())
            .expect("отказ прогона своим типом, а не строкой");
        assert!(matches!(
            refused,
            EvalRunFailed::UnknownVersion { found: 2 }
        ));
    }

    #[test]
    fn duplicate_id_is_a_broken_file() {
        let text = format!("{META}\n{CASE_MORPH}\n{CASE_MORPH}");
        let err = parse(&text).expect_err("дубль id");
        let msg = format!("{err:#}");
        assert!(msg.contains("строка 3"), "номер строки в сообщении: {msg}");
        assert!(msg.contains("morph-a"), "id в сообщении: {msg}");
    }

    #[test]
    fn case_without_author_tag_is_a_broken_file() {
        let no_tag = CASE_MORPH.replace(r#"["by:owner","phase-E"]"#, r#"["phase-E"]"#);
        let err = parse(&format!("{META}\n{no_tag}")).expect_err("нет метки авторства");
        assert!(format!("{err:#}").contains("by:owner"));
    }

    /// Две метки `by:` — та же беда, что ни одной: разбивка авторства
    /// перестаёт сходиться с числом кейсов.
    #[test]
    fn two_author_tags_are_a_broken_file() {
        let two = CASE_MORPH.replace(r#"["by:owner","phase-E"]"#, r#"["by:owner","by:derived"]"#);
        assert!(parse(&format!("{META}\n{two}")).is_err());
    }

    #[test]
    fn blank_line_is_a_broken_file() {
        let err = parse(&format!("{META}\n\n{CASE_MORPH}")).expect_err("пустая строка");
        assert!(format!("{err:#}").contains("строка 2"));
    }

    #[test]
    fn broken_case_line_names_its_own_number() {
        let text = format!("{META}\n{CASE_MORPH}\n{{\"id\":\"x\"}}");
        let err = parse(&text).expect_err("кейс без kind");
        assert!(format!("{err:#}").contains("строка 3"));
    }

    /// Порядок строк на результат не влияет — значит и на отпечаток.
    #[test]
    fn digest_ignores_file_order_but_not_verdicts() {
        let a = vec![
            outcome("alpha", CaseKind::RecallTop5, Verdict::Pass),
            outcome("beta", CaseKind::Morphology, Verdict::Fail),
            outcome("gamma", CaseKind::Morphology, Verdict::Skip),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(digest_of(&a), digest_of(&b));

        let mut changed = a.clone();
        changed[1].verdict = Verdict::Pass;
        assert_ne!(
            digest_of(&a),
            digest_of(&changed),
            "смена вердикта обязана менять отпечаток"
        );
        assert_eq!(digest_of(&a).chars().count(), DIGEST_HEX_LEN);
    }

    /// Один и тот же вход даёт один и тот же отпечаток — это и есть обещание
    /// «завтра то же число».
    #[test]
    fn digest_is_stable_across_calls() {
        let outcomes = vec![outcome("only", CaseKind::Morphology, Verdict::Pass)];
        assert_eq!(digest_of(&outcomes), digest_of(&outcomes));
        assert_eq!(digest_of(&[]), digest_of(&[]));
    }

    /// `SKIP` не входит ни в числитель, ни в знаменатель, а сумма трёх
    /// вердиктов равна числу кейсов — та самая строка сверки отчёта.
    #[test]
    fn skip_stays_out_of_both_numerator_and_denominator() {
        let now = at("2026-09-08T00:00:00Z");
        let report = tally_up(
            vec![
                outcome("r-pass", CaseKind::RecallTop5, Verdict::Pass),
                outcome("r-fail", CaseKind::RecallTop5, Verdict::Fail),
                outcome("r-skip", CaseKind::RecallTop5, Verdict::Skip),
                outcome("m-pass", CaseKind::Morphology, Verdict::Pass),
            ],
            now,
        );

        assert_eq!(report.total, 4);
        assert_eq!(report.passed + report.failed + report.skipped, report.total);
        assert_eq!(report.now, now);

        let recall = report.tally(CaseKind::RecallTop5);
        assert_eq!(recall.cases, 3);
        assert_eq!(recall.skipped, 1);
        assert_eq!(recall.eligible(), 2, "знаменатель — кейсы вида минус SKIP");
        assert_eq!(recall.passed, 1);

        // Вид без кейсов печатается как «нет кейсов», а не как 0 %: ноль в
        // знаменателе не печатается никогда.
        let refusal = report.tally(CaseKind::Refusal);
        assert!(!refusal.has_cases());
        assert_eq!(refusal.eligible(), 0);
    }

    /// Все кейсы вида пропущены — «непригодно (N)», третье состояние доли.
    #[test]
    fn all_skipped_kind_is_unusable_not_zero_percent() {
        let report = tally_up(
            vec![
                outcome("m-1", CaseKind::Morphology, Verdict::Skip),
                outcome("m-2", CaseKind::Morphology, Verdict::Skip),
            ],
            at("2026-09-08T00:00:00Z"),
        );
        let morph = report.tally(CaseKind::Morphology);
        assert!(morph.has_cases());
        assert_eq!(morph.eligible(), 0);
    }

    /// Порядок видов задан в одном месте — иначе две формы отчёта разошлись бы.
    #[test]
    fn kind_order_is_fixed() {
        let names: Vec<&str> = CaseKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "recall_top5",
                "morphology",
                "refusal",
                "snapshot_contains",
                "recognize_feed"
            ]
        );
        for kind in CaseKind::ALL {
            assert_eq!(CaseKind::ALL[kind.index()], kind);
        }
    }

    /// Вид, который этот бинарник не исполняет, — не `SKIP`, а несостоявшийся
    /// прогон: неисполненная проверка, зачтённая в долю, объявляет сто
    /// процентов там, где не проверялось ничего. Соединение здесь пустое —
    /// отказ обязан случиться до первого запроса.
    #[test]
    fn unexecutable_kind_fails_the_run_before_any_query() {
        let refusal = r#"{"id":"refuse-topic-au","kind":"refusal","input":{"topic":"au"},"expect":{"refused":true},"why":"владелец, 08.09.2026","tags":["by:owner"]}"#;
        let (meta, cases) = parse(&format!("{META}\n{refusal}")).expect("файл разбирается");
        let conn = Connection::open_in_memory().expect("память под соединение");

        let err = run(&conn, &meta, &cases, at("2026-09-08T00:00:00Z"))
            .expect_err("вид refusal этот бинарник не исполняет");
        let refused = err
            .chain()
            .find_map(|c| c.downcast_ref::<EvalRunFailed>())
            .expect("отказ прогона своим типом");
        match refused {
            EvalRunFailed::KindNotExecutable { id, kind } => {
                assert_eq!(id, "refuse-topic-au");
                assert_eq!(*kind, "refusal");
            }
            other => panic!("не тот отказ: {other}"),
        }
    }

    /// Пустой набор кейсов — прогон состоялся, чисел нет ни по одному виду.
    #[test]
    fn empty_case_set_runs_and_reports_nothing() {
        let (meta, cases) = parse(META).expect("одна meta");
        let conn = Connection::open_in_memory().expect("память под соединение");
        let report = run(&conn, &meta, &cases, at("2026-09-08T00:00:00Z")).expect("прогон");
        assert_eq!(report.total, 0);
        for kind in CaseKind::ALL {
            assert!(!report.tally(kind).has_cases());
        }
    }

    /// Звёздочка в запросе — `SKIP` до всякого обращения к базе: подсказка
    /// префикса это обход морфологии, а не её проверка.
    #[test]
    fn wildcard_query_skips_without_touching_the_database() {
        let wildcard = r#"{"id":"morph-star","kind":"morphology","input":{"query":"redis*"},"expect":{"non_empty":true},"why":"владелец, 08.09.2026","tags":["by:owner"]}"#;
        let (meta, cases) = parse(&format!("{META}\n{wildcard}")).expect("файл разбирается");
        let conn = Connection::open_in_memory().expect("память под соединение");

        let report = run(&conn, &meta, &cases, at("2026-09-08T00:00:00Z")).expect("прогон");
        assert_eq!(report.skipped, 1);
        assert_eq!(report.tally(CaseKind::Morphology).eligible(), 0);
        assert!(report.outcomes[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("звёздочка")));
    }

    /// Похожее на секрет во входе кейса — `SKIP` с причиной, и тоже до базы.
    #[test]
    fn secret_lookalike_in_input_skips_the_case() {
        let leaky = r#"{"id":"morph-leak","kind":"morphology","input":{"query":"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"},"expect":{"non_empty":true},"why":"владелец, 08.09.2026","tags":["by:owner"]}"#;
        let (meta, cases) = parse(&format!("{META}\n{leaky}")).expect("файл разбирается");
        let conn = Connection::open_in_memory().expect("память под соединение");

        let report = run(&conn, &meta, &cases, at("2026-09-08T00:00:00Z")).expect("прогон");
        assert_eq!(report.skipped, 1);
        assert!(report.outcomes[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("секрет")));
    }

    #[test]
    fn top_mode_defaults_to_any() {
        let case: EvalCase = serde_json::from_str(CASE_RECALL).expect("кейс разбирается");
        match case.body {
            CaseBody::RecallTop5 { expect, .. } => {
                assert!(expect.mode.is_none(), "умолчание живёт в судействе");
                assert_eq!(expect.mode.unwrap_or(TopMode::Any), TopMode::Any);
                assert!(expect.forbid_types.is_empty());
            }
            other => panic!("не тот вид: {other:?}"),
        }
    }

    /// Отчёт о позициях отличает «промахнулось ранжирование» от «поиск не
    /// нашёл» — это разные фазы и разный ремонт.
    #[test]
    fn positions_report_tells_missing_from_low_ranked() {
        assert_eq!(positions_report(&[], &[]), "");
        let id = Uuid::parse_str("8c1d0f52-3b7a-4d19-9f2e-6a0c4b18d7e5").expect("uuid");
        assert_eq!(
            positions_report(&[id], &[]),
            format!("{} не найден", "8c1d0f52")
        );
    }

    /// sha256 фикстуры сверяется до первого кейса, и несовпадение — свой тип
    /// ошибки, а не строка.
    #[test]
    fn fixture_checksum_mismatch_is_its_own_error() {
        let path = std::env::temp_dir().join(format!("aurelius-eval-fixture-{}", Uuid::new_v4()));
        std::fs::write(&path, b"not a database").expect("временный файл");

        let actual = sha256_file(&path).expect("хэш файла");
        assert!(verify_fixture(&path, &actual).is_ok());
        assert!(
            verify_fixture(&path, &actual.to_uppercase()).is_ok(),
            "регистр hex ничего не значит"
        );

        let err = verify_fixture(&path, "deadbeef").expect_err("не тот хэш");
        let refused = err
            .chain()
            .find_map(|c| c.downcast_ref::<EvalRunFailed>())
            .expect("отказ прогона своим типом");
        assert!(matches!(refused, EvalRunFailed::FixtureChanged { .. }));

        let _ = std::fs::remove_file(&path);
    }

    /// Имя типа берётся из serde, а не из `Debug`: `WorkLog` в файле кейсов
    /// пишется `work_log`, а `run` — это `Custom("run")`.
    #[test]
    fn type_name_matches_what_the_case_file_writes() {
        let node = |t: NodeType| Node {
            id: Uuid::new_v4(),
            node_type: t,
            label: "x".into(),
            note: None,
            source: "test".into(),
            data: serde_json::Value::Null,
            created_at: at("2026-09-08T00:00:00Z"),
            updated_at: at("2026-09-08T00:00:00Z"),
            memory_kind: crate::models::MemoryKind::Semantic,
            last_accessed_at: at("2026-09-08T00:00:00Z"),
            access_count: 0,
            content_hash: None,
            created_by: None,
            updated_by: None,
            deleted_at: None,
            sync_seq: None,
        };
        assert_eq!(type_name(&node(NodeType::WorkLog)), "work_log");
        assert_eq!(type_name(&node(NodeType::File)), "file");
        assert_eq!(type_name(&node(NodeType::Custom("run".into()))), "run");
    }
}
