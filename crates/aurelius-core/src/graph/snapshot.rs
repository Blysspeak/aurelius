//! Слой 7-уровневой памяти: замороженный снапшот и дистилляция.
//!
//! Снапшот — компактный Markdown (жёсткий потолок байт на слой и
//! [`B_TOTAL`] на всё), который инжектится в контекст агента ОДИН раз при
//! старте сессии. Урок hermes-agent: маленький курируемый срез в системном
//! промпте бьёт большой JSON по запросу — и не ломает prefix-cache, потому что
//! заморожен на сессию. Но и платится он каждым следующим запросом сессии,
//! поэтому здесь только то, что нужно в первую секунду: где стоит репозиторий,
//! как работать с владельцем, что открыто в ЭТОМ проекте.
//!
//! Дистиллят — структурная выжимка без LLM: незакрытые next_steps
//! последних сессий + нерешённые проблемы, пересобирается consolidate().

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::git::RepoState;
use crate::models::{MemoryKind, Node, NodeType};
use crate::secret::is_secret_ref;

/// Потолки слоёв в БАЙТАХ, а не в символах: снапшот оплачивается байтами на
/// каждом запросе сессии, а кириллица вдвое дороже латиницы. Слой
/// «Репозиторий» держит свой потолок в [`crate::git`].
const B_IDENTITY: usize = 400;
const B_WORKING: usize = 800;
const B_PRESSURE: usize = 300;
const B_EPISODIC: usize = 700;
const B_SEMANTIC: usize = 700;
const B_PROCEDURAL: usize = 400;
const B_DIGEST: usize = 300;

/// Потолок всего снапшота — длина markdown внутри JSON-строки, с
/// экранированием. Обёртка SessionStart-хука добавляет около ста байт, так
/// что весь ответ хука не больше 4000. При переборе слои снимаются целиком,
/// начиная с наименее нужных при пробуждении (см. `drop_rank` в
/// [`build_snapshot_in`]); репозиторий, владелец и «В работе» — никогда.
const B_TOTAL: usize = 3850;

/// Строка под заголовком снапшота без проекта. Хук пробуждения выводит проект
/// из репозитория и вне репозитория раньше не отдавал ничего — сессия
/// просыпалась без памяти вовсе. Теперь он отдаёт глобальный срез, и срез
/// обязан сам сказать, что он не проектный и как попросить проектный.
const GLOBAL_SCOPE_NOTE: &str =
    "Проект не определён — срез по всей памяти, не по проекту. Срез проекта: memory_status(project=…).\n";

/// Строк в слоях «Владелец», «В работе» (вместе с активными, пока их не
/// больше) и «Приёмы».
const OWNER_ROWS: usize = 3;
const WORKING_ROWS: usize = 3;
const SKILL_ROWS: usize = 3;

/// Потолок строки, у которой нет `claim`, в символах — ровно столько,
/// сколько `claim` может занять при записи. Первая фраза длиннее не режется
/// на полуслове, а строка снимается целиком.
const ROW_CHARS: usize = 240;

/// Аварийный предел слоя активных задач (FR-018): сколько задач в состоянии
/// `active` показывается, даже если их заведено больше. Инвариант «одна
/// активная на проект» держит это число маленьким на практике; предел —
/// защита от чужого проекта или бага, а не рабочий режим.
const ACTIVE_TASK_CAP: usize = 20;

/// Резать по границе слова, а не посреди него (FR-020). Раньше `chars().take(n)`
/// рубил вслепую — девять из семнадцати записей дампа обрывались на полуслове.
///
/// `pub`, а не приватная: та же обрезка нужна `mcp::handlers::task::task_view`
/// для длинных `note` вложенных узлов (decision/problem/solution/work_log) —
/// заводить вторую копию того же правила ради модульной границы бессмысленно.
pub fn clip(s: &str, budget: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= budget {
        return one;
    }
    let limit = budget.saturating_sub(1);
    let chars: Vec<char> = one.chars().collect();
    let mut end = limit.min(chars.len());
    let cuts_mid_word = end < chars.len()
        && end > 0
        && !chars[end - 1].is_whitespace()
        && !chars[end].is_whitespace();
    if cuts_mid_word {
        if let Some(boundary) = chars[..end].iter().rposition(|c| c.is_whitespace()) {
            end = boundary;
        }
    }
    let cut: String = chars[..end].iter().collect::<String>();
    format!("{}…", cut.trim_end())
}

/// Текст узла для выдачи.
///
/// Короткое утверждение (`claim`) отдаётся ЦЕЛИКОМ и не режется никогда: смысл
/// разделения в том, что суть влезает в бюджет, а длинное обоснование лежит
/// отдельно и приезжает по запросу. Раньше и то и другое было одним `note`, и
/// бюджет рубил его вслепую — в стартовом снапшоте всё обрывалось многоточием
/// на полуслове. Потолок в 240 символов гарантирован при записи.
fn body(node: &Node, per_line: usize) -> String {
    match crate::provenance::Provenance::from_data(&node.data).claim {
        Some(claim) => claim,
        None => clip(node.note.as_deref().unwrap_or(&node.label), per_line),
    }
}

/// Дописать к тексту то, что о нём известно: чем подтверждён и не пора ли
/// перепроверить. Измеренное и свежее не помечается — пометка на всём подряд
/// становится фоном и перестаёт читаться.
fn annotate(node: &Node, text: String) -> String {
    let p = crate::provenance::Provenance::from_data(&node.data);
    let mut out = text;
    if let Some(mark) = p.confidence_mark() {
        out.push_str(&format!(" [{mark}]"));
    }
    if let Some(stale) = p.staleness(node.created_at, Utc::now()) {
        out.push_str(&format!(" ({})", stale.note()));
    }
    out
}

/// Строки слоя: по одной на узел, не больше `max_rows` строк и `budget` байт.
/// Узел, для которого `line` ничего не вернул, пропускается, и место
/// достаётся следующему.
///
/// Координаты секретов ([`is_secret_ref`]) отсеиваются — FR-027: они отдаются
/// по запросу (`au secret list`) и НЕ ДОЛЖНЫ попадать в подаваемую память
/// автоматически. Ни один из типов, перечисленных в [`gather`], сегодня не
/// выбирает `Config`, так что фильтр здесь — граница, а не текущая
/// необходимость: он держит инвариант верным и тогда, когда слой снапшота
/// однажды расширят. Правило живёт в `secret`, рядом с остальными правилами о
/// секретах, и переиспользуется отсюда и из поиска — двух копий у него быть не
/// должно, иначе они разойдутся молча.
fn lines<'a>(
    nodes: impl IntoIterator<Item = &'a Node>,
    max_rows: usize,
    budget: usize,
    line: impl Fn(&Node) -> Option<String>,
) -> String {
    let mut out = String::new();
    let mut rows = 0;
    for n in nodes.into_iter().filter(|n| !is_secret_ref(n)) {
        if rows >= max_rows {
            break;
        }
        let Some(text) = line(n) else { continue };
        let row = format!("- {text}\n");
        if out.len() + row.len() > budget {
            break;
        }
        out.push_str(&row);
        rows += 1;
    }
    out
}

/// Слой в прежней форме: `claim` целиком или тело по границе слова, с
/// пометкой происхождения.
fn layer(nodes: &[Node], per_line: usize, budget: usize) -> String {
    lines(nodes, usize::MAX, budget, |n| {
        Some(annotate(n, body(n, per_line)))
    })
}

/// Метка без префикса `[проект] ` — в слое своего проекта он повторяет
/// заголовок снапшота.
fn strip_project_prefix(label: &str) -> &str {
    label
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .map_or(label, |(_, tail)| tail)
}

/// Метка — всего лишь копия текста: префикс или обрезка с многоточием,
/// какую пишут сами `memory_add`/`au note`, когда метку не назвали. Такую
/// метку показывать рядом с текстом — значит отдать одно и то же дважды.
///
/// `pub`: то же правило нужно `node_recall` в MCP-крейте; вторая копия
/// разошлась бы с этой молча.
#[must_use]
pub fn label_repeats(label: &str, text: &str) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let head = norm(
        strip_project_prefix(label)
            .trim_end()
            .trim_end_matches(['…', '.'])
            .trim_end(),
    );
    head.is_empty() || norm(text).starts_with(&head)
}

/// Первая фраза текста — до первой точки, восклицательного или
/// вопросительного знака с пробелом после или до конца строки. Длиннее
/// [`ROW_CHARS`] — `None`: фраза не режется посередине, строка снимается.
fn first_sentence(text: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let one = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = [". ", "! ", "? "]
        .iter()
        .filter_map(|p| one.find(p).map(|i| i + 1))
        .min()
        .unwrap_or(one.len());
    let sentence = one[..end].to_owned();
    (sentence.chars().count() <= ROW_CHARS).then_some(sentence)
}

/// Строка владельца: его метка — заголовок, который автор дал правилу.
/// Метка-копия текста заменяется `claim`, а без него — первой фразой тела.
/// Пометка происхождения не ставится: факт о владельце по своей природе
/// сказан им самим, и `[reported]` на каждой строке — фон, а не сведение.
fn owner_line(n: &Node) -> Option<String> {
    let claim = crate::provenance::Provenance::from_data(&n.data).claim;
    let copied = [claim.as_deref(), n.note.as_deref()]
        .into_iter()
        .flatten()
        .any(|t| label_repeats(&n.label, t));
    if !copied {
        return Some(strip_project_prefix(&n.label).to_owned());
    }
    claim.or_else(|| n.note.as_deref().and_then(first_sentence))
}

/// Строка задачи: `claim`, а без него — заголовок из метки. Первая фраза
/// тела задачи — это предыстория («Найдено 16.09 по свидетельствам…»), а
/// что делать, `task_create` кладёт в заголовок в повелительной форме.
/// Происхождение не помечается: задача — намерение, а не утверждение о мире.
fn task_line(n: &Node) -> String {
    crate::provenance::Provenance::from_data(&n.data)
        .claim
        .unwrap_or_else(|| strip_project_prefix(&n.label).to_owned())
}

/// Строка слоя «В работе»: задача — [`task_line`]; прочее — `claim`, а без
/// него первая фраза тела, если влезает в [`ROW_CHARS`], иначе ничего.
fn work_line(n: &Node) -> Option<String> {
    if matches!(n.node_type, NodeType::Task) {
        return Some(task_line(n));
    }
    let text = match crate::provenance::Provenance::from_data(&n.data).claim {
        Some(claim) => claim,
        None => first_sentence(n.note.as_deref().unwrap_or(&n.label))?,
    };
    Some(annotate(n, text))
}

/// Убрать списки абсолютных путей после двоеточия: «файлов затронуто 14:
/// /home/…/a.rs, /home/…/b.rs» → «файлов затронуто 14». Чекпоинт сессии
/// сообщает, сколько файлов тронул, а не какие: пути принадлежат слою
/// «Репозиторий», и там их не больше трёх.
fn drop_path_lists(text: &str) -> String {
    let is_path = |w: &str| {
        let b = w.as_bytes();
        w.starts_with('/')
            || w.starts_with("~/")
            || w.starts_with(r"\\?\")
            || (b.len() > 2
                && b[0].is_ascii_alphabetic()
                && b[1] == b':'
                && (b[2] == b'\\' || b[2] == b'/'))
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if w.ends_with(':') && words.get(i + 1).is_some_and(|next| is_path(next)) {
            out.push(w.trim_end_matches(':').to_owned());
            i += 1;
            let mut closed_sentence = false;
            while i < words.len() && is_path(words[i]) {
                closed_sentence = words[i].ends_with('.');
                i += 1;
            }
            if closed_sentence {
                if let Some(last) = out.last_mut() {
                    last.push('.');
                }
            }
            continue;
        }
        out.push(w.to_owned());
        i += 1;
    }
    out.join(" ")
}

/// Строка слоя «Последние сессии»: как прежде, но без списков путей.
fn session_line(n: &Node, per_line: usize) -> String {
    let text = match crate::provenance::Provenance::from_data(&n.data).claim {
        Some(claim) => drop_path_lists(&claim),
        None => clip(
            &drop_path_lists(n.note.as_deref().unwrap_or(&n.label)),
            per_line,
        ),
    };
    annotate(n, text)
}

/// Строка карточки навыка: имя — ключ для `skill_get` — и начало условия
/// загрузки. Без имени строка бесполезна: по ней карточку не достать.
fn skill_line(n: &Node) -> String {
    match n.note.as_deref().filter(|t| !t.trim().is_empty()) {
        Some(trigger) => format!("{} — {}", n.label, clip(trigger, 60)),
        None => n.label.clone(),
    }
}

/// Свежие узлы типа в области проекта. Тонкая обёртка над общим предикатом
/// принадлежности: раньше здесь жил свой запрос по префиксу метки, который не
/// видел узлы, связанные с проектом ребром.
fn typed_recent(
    conn: &Connection,
    t: &NodeType,
    project: Option<&str>,
    limit: usize,
) -> Result<Vec<Node>> {
    super::typed_in_project(conn, t, project, limit)
}

/// Один факт снапшота в машинной форме.
///
/// Существует потому, что потребитель, разбирающий markdown регулярками по
/// `## N · Заголовок`, читает ОФОРМЛЕНИЕ: следующая смена вёрстки сломает его
/// так же тихо, как молчал сам канал. Здесь форма зафиксирована.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Fact {
    /// Слой-источник: `userfact` | `active_task` | `task` | `problem` |
    /// `obligation` | `session` | `decision` | `concept` | `skill` | `digest`.
    pub kind: &'static str,
    /// Полный текст, без бюджетной обрезки: у потребителя свой бюджет, а молча
    /// укороченный факт неотличим от короткого.
    pub text: String,
    /// RFC 3339, время последнего изменения узла. `null` там, где источник
    /// времени не хранит (обязательства).
    pub at: Option<String>,
    /// Чем подтверждён факт: `measured` | `inferred` | `reported` |
    /// `unverified`. Отсутствие происхождения читается как `unverified`, а не
    /// как «наверное измерено».
    pub confidence: &'static str,
    /// Приписка «старше N дней — перепроверь …», когда факт волатилен и
    /// просрочен. `null`, пока свежий или пока волатильность не названа.
    pub stale: Option<String>,
}

/// Машинная форма снапшота.
///
/// Пустой `facts` при коде возврата 0 однозначно значит «нечего сказать»;
/// отсутствие вывода или ненулевой код — «сломан». Отдельные коды возврата под
/// каждое состояние не нужны: форма их уже различает.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SnapshotFacts {
    pub project: Option<String>,
    pub facts: Vec<Fact>,
}

/// Дистиллят, в котором нечего сказать.
///
/// Это НЕ факт: пустой дистиллят несёт ноль информации, занимает бюджет и в
/// машинной форме ломает контракт «пустой `facts` = нечего сказать» — новый
/// проект отдавал бы одну строку-заглушку вместо пустого массива.
const EMPTY_DIGEST: &str = "Хвостов нет — чисто.";

/// Открытые задачи не в работе: `active` уже выбрана отдельным, приоритетным
/// слоем (см. [`gather`]), здесь — только то, что ждёт своей очереди.
const OTHER_OPEN_TASK_STATUSES: &str = "blocked,backlog";

/// Момент, по которому активные задачи ранжируются под аварийным пределом:
/// когда взята в работу, а не когда заведена. Узел без `activated_at`
/// (переход в `active` до этой фичи) считается наименее свежим.
fn activated_key(n: &Node) -> String {
    n.data
        .get("activated_at")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_default()
}

/// Всё, что снапшот читает из графа. Одно место сбора на обе формы вывода —
/// иначе markdown и JSON разойдутся, и разойдутся молча.
struct Gathered {
    identity: Vec<Node>,
    /// Задачи в состоянии `active`: по FR-017 обязаны попасть в дамп целиком,
    /// поэтому отделены от прочих открытых и режутся не общим лимитом выборки,
    /// а собственным аварийным пределом [`ACTIVE_TASK_CAP`].
    active_tasks: Vec<Node>,
    /// Сколько активных задач не поместилось под аварийный предел (FR-018) —
    /// печатается явно, а не молчаливо теряется.
    active_overflow: usize,
    /// Прочие открытые задачи (`blocked`, `backlog`) — ниже приоритетом.
    other_tasks: Vec<Node>,
    problems: Vec<Node>,
    pressure: Vec<String>,
    sessions: Vec<Node>,
    decisions: Vec<Node>,
    concepts: Vec<Node>,
    skills: Vec<Node>,
    /// Сколько карточек навыков всего — для строки-указателя на остальные.
    skills_total: usize,
    digest: Vec<Node>,
}

fn gather(conn: &Connection, project: Option<&str>) -> Result<Gathered> {
    // Аварийный предел применяется в Rust, не в SQL: выборка сортирует по
    // приоритету/дате заведения, а предел FR-018 — по времени взятия в работу.
    let mut active_tasks = super::get_tasks_filtered(conn, project, Some("active"), None, 200)?;
    active_tasks.sort_by_key(|n| std::cmp::Reverse(activated_key(n)));
    let active_overflow = active_tasks.len().saturating_sub(ACTIVE_TASK_CAP);
    active_tasks.truncate(ACTIVE_TASK_CAP);

    // Прочие открытые — свежими первыми. Выборка сортирует по приоритету, и
    // с лимитом 8 самая свежая задача большого бэклога в неё просто не
    // попадала; берётся всё и режется уже по свежести.
    let mut other_tasks =
        super::get_tasks_filtered(conn, project, Some(OTHER_OPEN_TASK_STATUSES), None, 200)?;
    other_tasks.sort_by_key(|n| std::cmp::Reverse(n.updated_at));
    other_tasks.truncate(8);

    let skill_type = serde_json::to_string(&NodeType::Skill)?;
    let skills_total: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE node_type = ?1 AND deleted_at IS NULL",
            [skill_type],
            |r| r.get::<_, i64>(0),
        )?
        .try_into()
        .unwrap_or(0);

    Ok(Gathered {
        // Своё и глобальное; связанное с другим проектом — никогда.
        identity: super::owner_facts(conn, project, 12)?,
        active_tasks,
        active_overflow,
        other_tasks,
        problems: super::get_unsolved_problems(conn, project, 6)?,
        // Гроссбух давления (ступень 6): открытые обязательства по напряжению —
        // недоделанное лезет наверх само. Best-effort: пусто при отсутствии таблиц.
        pressure: crate::obligations::top_by_tension(conn, 6)
            .map(|obs| {
                obs.iter()
                    .map(|o| {
                        let obj: String = o.object.chars().take(72).collect();
                        format!("[{:.1}] {} → {}: {}", o.tension, o.debtor, o.creditor, obj)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sessions: typed_recent(conn, &NodeType::Session, project, 3)?,
        decisions: typed_recent(conn, &NodeType::Decision, project, 8)?,
        concepts: typed_recent(conn, &NodeType::Concept, project, 5)?,
        // Навыки межпроектны по природе — фильтра по проекту нет, есть потолок.
        skills: typed_recent(conn, &NodeType::Skill, None, SKILL_ROWS)?,
        skills_total,
        digest: typed_recent(conn, &NodeType::Digest, project, 1)?
            .into_iter()
            .filter(|n| n.note.as_deref() != Some(EMPTY_DIGEST))
            .collect(),
    })
}

fn push_facts(facts: &mut Vec<Fact>, kind: &'static str, nodes: &[Node]) {
    let now = Utc::now();
    facts.extend(nodes.iter().filter(|n| !is_secret_ref(n)).map(|n| {
        let p = crate::provenance::Provenance::from_data(&n.data);
        Fact {
            kind,
            // Машинной форме пометки в текст не подмешиваются — они отдельными
            // полями, иначе потребителю пришлось бы выковыривать их регулярками
            // из той самой строки, которую он читает как факт.
            text: p
                .claim
                .clone()
                .or_else(|| n.note.clone())
                .unwrap_or_else(|| n.label.clone()),
            at: Some(n.updated_at.to_rfc3339()),
            confidence: p.confidence_or_default().as_str(),
            stale: p.staleness(n.created_at, now).map(|s| s.note()),
        }
    }));
}

/// Тот же снапшот, что и [`build_snapshot`], но машинной формой: без вёрстки,
/// без бюджетной обрезки и без счётчика узлов (это оформление, а не знание).
pub fn snapshot_facts(conn: &Connection, project: Option<&str>) -> Result<SnapshotFacts> {
    let g = gather(conn, project)?;
    let mut facts = Vec::new();
    push_facts(&mut facts, "userfact", &g.identity);
    // Активные — приоритетный вид факта: потребитель различает «в работе
    // прямо сейчас» от «открыта, но не взята» без разбора текста.
    push_facts(&mut facts, "active_task", &g.active_tasks);
    push_facts(&mut facts, "task", &g.other_tasks);
    push_facts(&mut facts, "problem", &g.problems);
    facts.extend(g.pressure.iter().map(|line| Fact {
        kind: "obligation",
        text: line.clone(),
        at: None,
        // Обязательство — не утверждение о мире, а взятое обещание: подтверждать
        // ему нечего и протухать нечему, у него своя мера в слое «Давление».
        confidence: crate::provenance::Confidence::Reported.as_str(),
        stale: None,
    }));
    push_facts(&mut facts, "session", &g.sessions);
    push_facts(&mut facts, "decision", &g.decisions);
    push_facts(&mut facts, "concept", &g.concepts);
    push_facts(&mut facts, "skill", &g.skills);
    push_facts(&mut facts, "digest", &g.digest);
    Ok(SnapshotFacts {
        project: project.map(str::to_owned),
        facts,
    })
}

/// Собрать снапшот без слоя «Репозиторий». Только чтение, без сети и
/// индексации. Дверь MCP (`memory_snapshot`) зовёт именно его: у сервера нет
/// своего знания о том, где стоит сессия.
pub fn build_snapshot(conn: &Connection, project: Option<&str>) -> Result<String> {
    build_snapshot_in(conn, project, None)
}

/// Длина markdown внутри JSON-строки — то, во что он обойдётся в ответе хука.
fn escaped_len(md: &str) -> usize {
    serde_json::to_string(md).map_or(md.len(), |s| s.len())
}

/// Собрать снапшот. `repo` — состояние репозитория, в котором стоит сессия
/// (см. [`crate::git::locate`]); `None` — слой «Репозиторий» пуст. Вызывается
/// хуком на старте каждой сессии и обязан быть мгновенным: git читает
/// вызывающий, здесь только граф.
///
/// Скелет постоянен: слои нумеруются 1..N по месту в списке, пустой слой
/// печатается «— пусто» (почему — у `render` ниже). `project` = `None` —
/// глобальный срез, и заголовок говорит об этом строкой `GLOBAL_SCOPE_NOTE`.
pub fn build_snapshot_in(
    conn: &Connection,
    project: Option<&str>,
    repo: Option<&RepoState>,
) -> Result<String> {
    let date = Utc::now().format("%Y-%m-%d");
    let scope = project.unwrap_or("глобально");

    let g = gather(conn, project)?;
    let nodes = super::count_nodes(conn)?;
    let edges = super::count_edges(conn)?;

    // Активные задачи — высший приоритет слоя «В работе» (FR-017): рендерятся
    // без бюджетного среза, так что ни одна не теряется из-за нехватки места;
    // защита от неограниченного разрастания — аварийный предел ACTIVE_TASK_CAP
    // уже применён в gather(). Здесь просто печатаем, сколько не поместилось.
    //
    // «Ни одна не теряется» — про проект, в котором стоит сессия: активная
    // задача одна на проект. Глобальный срез — это по одной на каждый проект
    // сразу: 19.09.2026 их было 19, слой занял 2.4 КБ, бюджет снял сессии,
    // решения и приёмы целиком, а потолок 4000 байт держался случайно. Без
    // проекта активные идут под обычный бюджет слоя, остальные — счётчиком.
    let (active_rows, active_budget) = match project {
        Some(_) => (usize::MAX, usize::MAX),
        None => (WORKING_ROWS, B_WORKING),
    };
    let mut working = lines(&g.active_tasks, active_rows, active_budget, |n| {
        Some(task_line(n))
    });
    let cut = match project {
        Some(_) => 0,
        None => g.active_tasks.len().saturating_sub(working.lines().count()),
    };
    let hidden = g.active_overflow + cut;
    if hidden > 0 {
        working.push_str(&format!("- …и ещё {hidden} активных не поместилось\n"));
    }
    let active_used = working.len();

    // Остаток слоя — прочим открытым задачам и проблемам вперемешку, свежим
    // первыми, до WORKING_ROWS строк вместе с активными.
    let mut open: Vec<&Node> = g.other_tasks.iter().chain(&g.problems).collect();
    open.sort_by_key(|n| std::cmp::Reverse(n.updated_at));
    working.push_str(&lines(
        open,
        WORKING_ROWS.saturating_sub(g.active_tasks.len()),
        B_WORKING.saturating_sub(active_used),
        work_line,
    ));

    // Если активные перебрали весь бюджет слоя, разница вычитается у слоя ниже
    // приоритетом (FR-017), а не у активных: «Решения и знания» отдаёт ровно
    // столько, сколько не хватило.
    let semantic_budget = B_SEMANTIC.saturating_sub(active_used.saturating_sub(B_WORKING));

    let mut semantic = layer(&g.decisions, 150, semantic_budget * 2 / 3);
    semantic.push_str(&layer(&g.concepts, 150, semantic_budget / 3));

    let mut pressure = String::new();
    for line in &g.pressure {
        let row = format!("- {line}\n");
        if pressure.len() + row.len() > B_PRESSURE {
            break;
        }
        pressure.push_str(&row);
    }

    let mut skills = lines(&g.skills, SKILL_ROWS, B_PROCEDURAL, |n| Some(skill_line(n)));
    let skills_rest = g.skills_total.saturating_sub(g.skills.len());
    if !skills.is_empty() && skills_rest > 0 {
        skills.push_str(&format!(
            "- ещё {skills_rest} — memory_search(query), тело — skill_get(name)\n"
        ));
    }

    // (заголовок, тело, очередь на снятие при переборе B_TOTAL: больший
    // номер снимается раньше; `None` — не снимается никогда).
    let mut sections: Vec<(&str, String, Option<u8>)> = vec![
        (
            "Репозиторий",
            repo.map(crate::git::render).unwrap_or_default(),
            None,
        ),
        (
            "Владелец",
            lines(&g.identity, OWNER_ROWS, B_IDENTITY, owner_line),
            None,
        ),
        ("В работе", working, None),
        ("Давление", pressure, Some(4)),
        (
            "Последние сессии",
            lines(&g.sessions, usize::MAX, B_EPISODIC, |n| {
                Some(session_line(n, 160))
            }),
            Some(1),
        ),
        ("Решения и знания", semantic, Some(2)),
        ("Приёмы", skills, Some(3)),
        (
            "Архив",
            format!(
                "- {nodes} узлов, {edges} рёбер; глубже — memory_recall(topic) / memory_search(query)\n"
            ),
            None,
        ),
        ("Дистиллят", layer(&g.digest, 140, B_DIGEST), Some(5)),
    ];

    // Скелет снапшота постоянен: номер слоя — его место в списке, а не место
    // среди непустых, и пустой слой печатается пустым. Это стоит несколько
    // байт на строку, но номер слоя обязан значить одно и то же в разных
    // сессиях. 19.09 проверено, чем обходится обратное: в пустом доме снапшот
    // печатал «1 · Последние сессии», «2 · Решения и знания» — номера съезжали
    // от того, какие слои оказались пустыми, а тест «сессия обязана лежать в
    // слое сессий» падал по причине, к сессиям не относящейся.
    let render = |sections: &[(&str, String, Option<u8>)]| {
        let mut md = format!("# Память · {scope} · {date}\n");
        if project.is_none() {
            md.push_str(GLOBAL_SCOPE_NOTE);
        }
        for (i, (title, body, _)) in sections.iter().enumerate() {
            let body = if body.is_empty() {
                "— пусто\n"
            } else {
                body.as_str()
            };
            md.push_str(&format!("\n## {} · {title}\n{body}", i + 1));
        }
        md
    };
    let mut md = render(&sections);
    while escaped_len(&md) > B_TOTAL {
        let victim = sections
            .iter_mut()
            .filter(|s| !s.1.is_empty() && s.2.is_some())
            .max_by_key(|s| s.2);
        match victim {
            Some(section) => section.1.clear(),
            None => break,
        }
        md = render(&sections);
    }
    Ok(md)
}

/// Пересобрать дистиллят проекта: next_steps последних сессий + нерешённые
/// проблемы. Идемпотентно — один Digest-узел на проект, старый затирается.
pub fn consolidate(conn: &Connection, project: &str) -> Result<Node> {
    let sessions = typed_recent(conn, &NodeType::Session, Some(project), 5)?;
    let mut steps: Vec<String> = Vec::new();
    for s in &sessions {
        if let Some(arr) = s.data.get("next_steps").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(t) = v.as_str() {
                    let t = clip(t, 140);
                    if !steps.contains(&t) {
                        steps.push(t);
                    }
                }
            }
        }
    }
    let problems = super::get_unsolved_problems(conn, Some(project), 5)?;

    let mut note = String::new();
    if !steps.is_empty() {
        note.push_str("Хвосты из сессий: ");
        note.push_str(&steps.into_iter().take(8).collect::<Vec<_>>().join("; "));
        note.push('.');
    }
    if !problems.is_empty() {
        let p = problems
            .iter()
            .map(|n| clip(n.note.as_deref().unwrap_or(&n.label), 100))
            .collect::<Vec<_>>()
            .join("; ");
        note.push_str(&format!(" Нерешённое: {p}."));
    }
    if note.is_empty() {
        note = EMPTY_DIGEST.to_owned();
    }

    let label = format!("[{project}] дистиллят");
    let type_str = serde_json::to_string(&NodeType::Digest)?;
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM nodes WHERE node_type = ?1 AND label = ?2 AND deleted_at IS NULL",
            rusqlite::params![type_str, label],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = existing {
        conn.execute(
            "UPDATE nodes SET note = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![note, Utc::now().to_rfc3339(), id],
        )?;
        let found = typed_recent(conn, &NodeType::Digest, Some(project), 1)?;
        found
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("дистиллят обновлён, но не читается"))
    } else {
        super::add_node_full(
            conn,
            NodeType::Digest,
            &label,
            Some(&note),
            "consolidate",
            serde_json::json!({ "project": project }),
            MemoryKind::Semantic,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-snap-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn clip_respects_budget_and_collapses_whitespace() {
        assert_eq!(clip("a  b\n c", 100), "a b c");
        let clipped = clip(&"ы".repeat(50), 10);
        assert!(clipped.chars().count() <= 10);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn snapshot_has_header_and_fits_total_budget() {
        let conn = test_conn();
        super::super::add_node(
            &conn,
            NodeType::UserFact,
            "владелец",
            Some("факт о владельце"),
            "test",
            serde_json::json!({}),
        )
        .expect("add user fact");
        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");
        assert!(md.starts_with("# Память · demo · "));
        // Скелет постоянен: Репозиторий — всегда первый слой, Владелец — второй.
        // Пустой слой печатается пустым, а не исчезает вместе со своим номером.
        assert!(md.contains("## 1 · Репозиторий"), "снапшот:\n{md}");
        assert!(md.contains("## 2 · Владелец"));
        // Общий потолок: снапшот обязан оставаться маленьким при любом графе.
        assert!(md.chars().count() < 6_000, "снапшот распух: {}", md.len());
    }

    /// Знание, записанное документированным путём `memory_add` + `memory_relate`:
    /// метка голая, принадлежность проекту выражена ТОЛЬКО ребром.
    #[test]
    fn snapshot_sees_knowledge_linked_by_edge_not_by_label_prefix() {
        use crate::models::Relation;

        let conn = test_conn();
        let project = super::super::add_node(
            &conn,
            NodeType::Project,
            "demo",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add project");
        let decision = super::super::add_node(
            &conn,
            NodeType::Decision,
            "взяли sqlite вместо постгреса",
            Some("решение: sqlite, потому что память локальная и однопользовательская"),
            "test",
            serde_json::json!({}),
        )
        .expect("add decision");
        super::super::add_edge(&conn, decision.id, project.id, Relation::BelongsTo, 1.0)
            .expect("link decision to project");

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");

        assert!(
            md.contains("sqlite"),
            "узел, связанный с проектом ребром, обязан попадать в снапшот; было:\n{md}"
        );
        assert!(
            md.contains("· Решения и знания"),
            "слой решений пуст:\n{md}"
        );
    }

    /// Снапшот другого проекта не имеет права утащить чужое знание.
    #[test]
    fn snapshot_does_not_leak_other_projects_knowledge() {
        use crate::models::Relation;

        let conn = test_conn();
        let project = super::super::add_node(
            &conn,
            NodeType::Project,
            "demo",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add project");
        let decision = super::super::add_node(
            &conn,
            NodeType::Decision,
            "взяли sqlite вместо постгреса",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add decision");
        super::super::add_edge(&conn, decision.id, project.id, Relation::BelongsTo, 1.0)
            .expect("link");

        let md = build_snapshot(&conn, Some("другой")).expect("snapshot");

        assert!(!md.contains("sqlite"), "чужое знание протекло:\n{md}");
    }

    /// task_create кладёт задачу в `backlog`. Если слой «В работе» её не видит,
    /// завести задачу означает потерять её.
    #[test]
    fn snapshot_shows_freshly_created_backlog_task() {
        let conn = test_conn();
        super::super::add_node(
            &conn,
            NodeType::Task,
            "[demo] починить снапшот",
            Some("слои 1-6 не доезжают"),
            "test",
            serde_json::json!({ "status": "backlog", "priority": "high" }),
        )
        .expect("add task");

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");

        assert!(
            md.contains("- починить снапшот\n"),
            "свежая задача в backlog обязана быть видна заголовком:\n{md}"
        );
    }

    /// Форма машинного вывода зафиксирована: потребитель не должен зависеть от
    /// вёрстки markdown. Пустой массив при успехе — «нечего сказать».
    #[test]
    fn json_facts_shape_is_fixed_and_empty_means_nothing_to_say() {
        let conn = test_conn();
        // Дистиллят-заглушка рождается сама при первом обращении к проекту и
        // однажды уже подменяла «нечего сказать» на строку с нулём информации.
        consolidate(&conn, "пусто").expect("consolidate");
        let out = snapshot_facts(&conn, Some("пусто")).expect("facts");

        assert_eq!(out.project.as_deref(), Some("пусто"));
        assert!(out.facts.is_empty());
        assert_eq!(
            serde_json::to_string(&out).expect("serialize"),
            r#"{"project":"пусто","facts":[]}"#
        );
    }

    #[test]
    fn json_facts_carry_kind_for_edge_linked_knowledge() {
        use crate::models::Relation;

        let conn = test_conn();
        let project = super::super::add_node(
            &conn,
            NodeType::Project,
            "demo",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add project");
        let decision = super::super::add_node(
            &conn,
            NodeType::Decision,
            "метка",
            Some("взяли sqlite"),
            "test",
            serde_json::json!({}),
        )
        .expect("add decision");
        super::super::add_edge(&conn, decision.id, project.id, Relation::BelongsTo, 1.0)
            .expect("link");

        let out = snapshot_facts(&conn, Some("demo")).expect("facts");

        let fact = out
            .facts
            .iter()
            .find(|f| f.kind == "decision")
            .expect("решение обязано попасть в машинную форму");
        assert_eq!(
            fact.text, "взяли sqlite",
            "текст отдаётся целиком, без обрезки"
        );
        assert!(fact.at.is_some(), "время изменения обязано быть в форме");
    }

    #[test]
    fn consolidate_is_idempotent_one_digest_per_project() {
        let conn = test_conn();
        let a = consolidate(&conn, "demo").expect("first");
        let b = consolidate(&conn, "demo").expect("second");
        assert_eq!(a.id, b.id, "должен обновляться тот же узел");
        let type_str = serde_json::to_string(&NodeType::Digest).expect("type");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE node_type = ?1 AND deleted_at IS NULL",
                [type_str],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 1);
    }

    /// Регресс: раньше слой «В работе» брал 8 самых свежих открытых задач ОДНИМ
    /// общим запросом, и активная задача среди 200 бэклога терялась. Активные
    /// теперь выбираются отдельно от прочих открытых — им конкуренция за место
    /// в выборке не грозит.
    #[test]
    fn active_task_survives_whole_among_two_hundred_open_tasks() {
        let conn = test_conn();
        for i in 0..200 {
            super::super::add_node(
                &conn,
                NodeType::Task,
                &format!("[demo] фоновая задача {i}"),
                None,
                "test",
                serde_json::json!({ "status": "backlog", "priority": "high" }),
            )
            .expect("add backlog task");
        }
        super::super::add_node(
            &conn,
            NodeType::Task,
            "[demo] актуальная работа",
            Some("чиним слой активных задач в снапшоте"),
            "test",
            serde_json::json!({ "status": "active", "activated_at": "2020-01-01T00:00:00Z" }),
        )
        .expect("add active task");

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");

        assert!(
            md.contains("- актуальная работа\n"),
            "активная задача обязана присутствовать среди 200 открытых:\n{md}"
        );
    }

    /// Находка 10 (адверсариальный разбор спеки 007): аварийный предел
    /// `ACTIVE_TASK_CAP` и текст сообщения о переполнении не были покрыты ни
    /// одним тестом — единственный существующий тест на активные задачи
    /// (`active_task_survives_whole_among_two_hundred_open_tasks`) держит
    /// только ОДНУ активную задачу и до этой ветки арифметики не доходит.
    /// Здесь — `ACTIVE_TASK_CAP + 3` активных задач: три самые старые по
    /// `activated_at` обязаны уйти в overflow, а сообщение — назвать точное
    /// число.
    #[test]
    fn active_task_overflow_reports_correct_count_and_keeps_most_recent() {
        let conn = test_conn();
        let total = ACTIVE_TASK_CAP + 3;
        for i in 0..total {
            super::super::add_node(
                &conn,
                NodeType::Task,
                &format!("[demo] активная задача {i:02}"),
                Some(&format!("нота активной задачи {i:02}")),
                "test",
                serde_json::json!({
                    "status": "active",
                    // Чем больше i, тем свежее взятие в работу — самые
                    // свежие ACTIVE_TASK_CAP обязаны остаться, самые старые
                    // (i < 3) — уйти в overflow.
                    "activated_at": format!("2020-01-{:02}T00:00:00Z", i + 1),
                }),
            )
            .expect("add active task");
        }

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");

        assert!(
            md.contains("- …и ещё 3 активных не поместилось"),
            "сообщение о переполнении обязано назвать точное число:\n{md}"
        );

        // Строка задачи — её заголовок (метка без префикса проекта).
        for i in 0..3 {
            let row = format!("- активная задача {i:02}\n");
            assert!(
                !md.contains(&row),
                "самая старая активная задача {i} обязана уйти в overflow, а не остаться в дампе:\n{md}"
            );
        }
        let newest_row = format!("- активная задача {:02}\n", total - 1);
        assert!(
            md.contains(&newest_row),
            "самая свежая активная задача обязана остаться в дампе:\n{md}"
        );
    }

    /// Находка 10: когда активные задачи съедают весь бюджет «В работе» и
    /// ещё сверху, разница вычитается у бюджета «Решения и знания» (FR-017)
    /// — ветка `semantic_budget = B_SEMANTIC.saturating_sub(...)` тоже не
    /// была покрыта ни одним тестом. 15 активных задач с длинными заголовками
    /// (печатаются без бюджетного среза — FR-017) суммарно намного больше
    /// `B_WORKING + B_SEMANTIC`, так что семантический бюджет обязан
    /// обнулиться, а слой решений — исчезнуть целиком, а не просто ужаться.
    #[test]
    fn active_tasks_overrunning_working_budget_shrink_semantic_layer() {
        let conn = test_conn();
        let long_title = "слово ".repeat(40); // ~240 символов, ~440 байт на строку
        for i in 0..15 {
            super::super::add_node(
                &conn,
                NodeType::Task,
                &format!("[demo] {long_title}{i}"),
                Some("нота"),
                "test",
                serde_json::json!({
                    "status": "active",
                    "activated_at": format!("2021-01-{:02}T00:00:00Z", i + 1),
                }),
            )
            .expect("add active task");
        }
        let decision_text = "заметное решение, которое обязано пропасть при нулевом бюджете";
        super::super::add_node(
            &conn,
            NodeType::Decision,
            "[demo] решение",
            Some(decision_text),
            "test",
            serde_json::json!({}),
        )
        .expect("add decision");

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");

        assert!(
            !md.contains(decision_text),
            "decision обязан пропасть при обнулённом семантическом бюджете:\n{md}"
        );
        // Заголовок остаётся на своём месте и с постоянным номером: исчезающий
        // заголовок сдвигал номера всех слоёв ниже, и «6 · Решения и знания» в
        // одной сессии значило не то же, что в другой.
        assert!(
            md.contains("## 6 · Решения и знания"),
            "пустой слой остаётся в скелете с постоянным номером:\n{md}"
        );
    }

    /// Тот же перебор активных в глобальном срезе (хук вне репозитория): там
    /// активные — по одной на каждый проект, и печатать их без среза значит
    /// отдать весь бюджет им. Здесь они идут под бюджет слоя, остальные —
    /// счётчиком, и знание остаётся в срезе.
    #[test]
    fn global_snapshot_puts_active_tasks_under_the_layer_budget() {
        let conn = test_conn();
        let long_title = "слово ".repeat(40);
        for i in 0..15 {
            super::super::add_node(
                &conn,
                NodeType::Task,
                &format!("[p{i}] {long_title}{i}"),
                Some("нота"),
                "test",
                serde_json::json!({
                    "status": "active",
                    "activated_at": format!("2021-01-{:02}T00:00:00Z", i + 1),
                }),
            )
            .expect("add active task");
        }
        let decision_text = "решение, которое глобальный срез обязан сохранить";
        super::super::add_node(
            &conn,
            NodeType::Decision,
            "решение",
            Some(decision_text),
            "test",
            serde_json::json!({}),
        )
        .expect("add decision");

        let md = build_snapshot(&conn, None).expect("snapshot");

        let working: Vec<&str> = md
            .split("\n## ")
            .find(|s| s.contains("· В работе\n"))
            .expect("working layer")
            .lines()
            .skip(1)
            .collect();
        let rows = working.iter().filter(|l| l.contains("слово")).count();
        assert!(
            (1..=WORKING_ROWS).contains(&rows),
            "активные вне бюджета слоя: {rows}\n{md}"
        );
        let hidden = 15 - rows;
        assert!(
            md.contains(&format!("- …и ещё {hidden} активных не поместилось\n")),
            "скрытые активные обязаны быть посчитаны:\n{md}"
        );
        assert!(md.contains(decision_text), "знание снято бюджетом:\n{md}");
        assert!(escaped_len(&md) <= B_TOTAL, "{}", escaped_len(&md));
    }

    /// T044/FR-027: снапшот проекта с записанными координатами секретов не
    /// содержит ни одного значения — ни в markdown, ни в машинной форме.
    /// Координата сама по себе не значение (T041 отклоняет такую запись
    /// раньше, чем она попадёт в граф), поэтому здесь под подозрением
    /// `location` — единственное поле, где реальный секрет мог бы просочиться.
    #[test]
    fn snapshot_excludes_secret_coordinates() {
        let conn = test_conn();
        let location = "1password://Private/Stripe/api-key";
        super::super::add_secret_ref(&conn, Some("demo"), "STRIPE_SECRET_KEY", None, location)
            .expect("add secret ref");

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");
        assert!(
            !md.contains(location) && !md.contains("STRIPE_SECRET_KEY"),
            "координата секрета просочилась в markdown-снапшот:\n{md}"
        );

        let facts = snapshot_facts(&conn, Some("demo")).expect("facts");
        let leaked = facts
            .facts
            .iter()
            .any(|f| f.text.contains(location) || f.text.contains("STRIPE_SECRET_KEY"));
        assert!(!leaked, "координата секрета просочилась в машинную форму");
    }

    /// FR-013 (spec 008): vendor-doc pages must never reach snapshot layers
    /// 1-6, no matter how many of them the graph holds — they have their own
    /// door (`au search`/`au context`), and the whole point of `data.layer =
    /// "vendor-docs"` is to keep a 332-page import from being visible here at
    /// all. `gather()` already never queries `NodeType::Doc` for any layer, so
    /// this asserts a property that holds by construction — but a construction
    /// this easy to break silently (one `typed_recent(conn, &NodeType::Doc,
    /// ...)` added to a future layer) needs a guard, not just an absence of
    /// code today.
    #[test]
    fn snapshot_never_surfaces_vendor_doc_nodes() {
        let conn = test_conn();
        let project = super::super::add_node(
            &conn,
            NodeType::Project,
            "demo",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add project");

        for i in 0..20 {
            super::super::add_node(
                &conn,
                NodeType::Doc,
                &format!("vendor page {i:02}"),
                Some(&format!(
                    "vendor doc body {i:02} — must never leak into the snapshot"
                )),
                "test",
                serde_json::json!({"layer": "vendor-docs", "project": "demo"}),
            )
            .expect("add doc node");
        }

        let decision = super::super::add_node(
            &conn,
            NodeType::Decision,
            "решение demo",
            Some("decision body must be visible"),
            "test",
            serde_json::json!({}),
        )
        .expect("add decision");
        super::super::add_edge(
            &conn,
            decision.id,
            project.id,
            crate::models::Relation::BelongsTo,
            1.0,
        )
        .expect("link decision to project");

        let concept = super::super::add_node(
            &conn,
            NodeType::Concept,
            "концепт demo",
            Some("concept body must be visible"),
            "test",
            serde_json::json!({}),
        )
        .expect("add concept");
        super::super::add_edge(
            &conn,
            concept.id,
            project.id,
            crate::models::Relation::BelongsTo,
            1.0,
        )
        .expect("link concept to project");

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");
        for i in 0..20 {
            assert!(
                !md.contains(&format!("vendor page {i:02}"))
                    && !md.contains(&format!("vendor doc body {i:02}")),
                "doc node {i} leaked into the markdown snapshot:\n{md}"
            );
        }
        assert!(md.contains("decision body must be visible"));
        assert!(md.contains("concept body must be visible"));

        let facts = snapshot_facts(&conn, Some("demo")).expect("facts");
        assert!(
            facts
                .facts
                .iter()
                .all(|f| !f.text.contains("vendor doc body")),
            "doc node leaked into the machine-readable facts: {facts:?}"
        );
        assert!(
            facts
                .facts
                .iter()
                .any(|f| f.kind == "decision" && f.text.contains("decision body")),
            "decision must still be present: {facts:?}"
        );
        assert!(
            facts
                .facts
                .iter()
                .any(|f| f.kind == "concept" && f.text.contains("concept body")),
            "concept must still be present: {facts:?}"
        );
    }

    /// FR-020: сокращение идёт по границе слова. Раньше `clip` рубил по счётчику
    /// символов вслепую — обрезанный текст мог заканчиваться на полуслове.
    #[test]
    fn dump_lines_never_cut_mid_word() {
        let conn = test_conn();
        let long_note = (0..40)
            .map(|i| format!("слово{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        // Решение без claim: его тело режется по бюджету строки. Строка
        // владельца — метка целиком, её обрезка здесь ничего не проверила бы.
        super::super::add_node(
            &conn,
            NodeType::Decision,
            "[demo] решение",
            Some(&long_note),
            "test",
            serde_json::json!({}),
        )
        .expect("add decision");

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");

        let mut checked = 0;
        for line in md.lines().filter(|l| l.contains('…')) {
            let clipped = line
                .split('…')
                .next()
                .expect("в строке есть многоточие")
                .trim_start_matches("- ");
            assert!(
                long_note.starts_with(clipped),
                "обрезанный текст обязан быть точным словесным префиксом исходника: {line:?}"
            );
            let boundary = long_note.chars().nth(clipped.chars().count());
            assert!(
                boundary.is_none() || boundary == Some(' '),
                "после отрезанного текста в оригинале обязан идти пробел, а не хвост слова: {line:?}"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "тест ничего не проверил — ни одна запись не была обрезана"
        );
    }

    /// Чекпоинт сессии сообщает, сколько файлов тронул, а не какие.
    #[test]
    fn session_rows_report_file_count_not_paths() {
        let text = "[чекпоинт 150k] ходов 0. правок 36. улик зелёных 8. файлов затронуто 3: \
                    /home/u/p/crates/au/src/commands.rs, /home/u/p/crates/au/src/main.rs, \
                    /home/u/p/crates/au/tests/exit_codes.rs";
        assert_eq!(
            drop_path_lists(text),
            "[чекпоинт 150k] ходов 0. правок 36. улик зелёных 8. файлов затронуто 3"
        );
        // Путь в середине фразы без двоеточия — не список, он остаётся.
        let plain = "503 на /api/v1/subscriptions у мерчантов";
        assert_eq!(drop_path_lists(plain), plain);
    }

    #[test]
    fn label_repeats_catches_prefix_and_truncation_only() {
        let claim = "Демон единственный владелец BGE-M3 и на чтении";
        assert!(label_repeats(
            "Демон единственный владелец BGE-M3 и на чт…",
            claim
        ));
        assert!(label_repeats(
            "[aurelius] Демон единственный владелец...",
            claim
        ));
        assert!(!label_repeats("recall: посев 12, глубина 2", claim));
    }

    /// «В работе»: не больше трёх строк, свежие первыми; первая фраза длиннее
    /// потолка не режется на полуслове — строка снимается, место достаётся
    /// следующей.
    #[test]
    fn working_layer_keeps_three_freshest_and_never_clips_a_sentence() {
        let conn = test_conn();
        let long = format!("{}.", "слово ".repeat(60).trim_end());
        for (i, note) in [
            "старая проблема. хвост",
            "вторая проблема. хвост",
            long.as_str(),
            "третья проблема. хвост",
            "свежая проблема. хвост",
        ]
        .iter()
        .enumerate()
        {
            let node = super::super::add_node(
                &conn,
                NodeType::Problem,
                &format!("[demo] проблема {i}"),
                Some(note),
                "test",
                serde_json::json!({}),
            )
            .expect("add problem");
            conn.execute(
                "UPDATE nodes SET updated_at = ?1, created_at = ?1 WHERE id = ?2",
                rusqlite::params![
                    format!("2026-09-0{}T00:00:00+00:00", i + 1),
                    node.id.to_string()
                ],
            )
            .expect("date problem");
        }

        let md = build_snapshot(&conn, Some("demo")).expect("snapshot");
        let layer: Vec<&str> = md
            .split("\n## ")
            .find(|s| s.contains("· В работе"))
            .expect("слой «В работе»")
            .lines()
            .skip(1)
            .collect();

        assert_eq!(layer.len(), 3, "три строки, не больше:\n{md}");
        assert!(layer[0].starts_with("- свежая проблема."), "{layer:?}");
        assert!(layer[1].starts_with("- третья проблема."), "{layer:?}");
        assert!(layer[2].starts_with("- вторая проблема."), "{layer:?}");
        assert!(
            !md.contains("слово слово"),
            "длинная фраза обязана сняться:\n{md}"
        );
        assert!(!md.contains("хвост"), "только первая фраза:\n{md}");
    }

    /// Весь снапшот не больше потолка при любом графе: лишнее снимается слоями
    /// с низа очереди, а репозиторий, владелец и «В работе» остаются.
    #[test]
    fn whole_snapshot_stays_under_total_ceiling() {
        let conn = test_conn();
        let long = "длинное слово ".repeat(30);
        for t in [
            NodeType::Decision,
            NodeType::Concept,
            NodeType::Session,
            NodeType::Skill,
            NodeType::Problem,
        ] {
            for i in 0..10 {
                super::super::add_node(
                    &conn,
                    t.clone(),
                    &format!("[demo] узел {i}"),
                    Some(&long),
                    "test",
                    serde_json::json!({}),
                )
                .expect("add node");
            }
        }
        super::super::add_node(
            &conn,
            NodeType::UserFact,
            "правило владельца",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add user fact");
        let repo = crate::git::RepoState {
            name: "demo".into(),
            head: crate::git::Head::Branch("main".into()),
            upstream: None,
            changed: 1,
            paths: vec!["a.rs".into()],
            commits: Vec::new(),
        };

        let md = build_snapshot_in(&conn, Some("demo"), Some(&repo)).expect("snapshot");

        assert!(
            escaped_len(&md) <= B_TOTAL,
            "{} байт:\n{md}",
            escaped_len(&md)
        );
        assert!(md.contains("## 1 · Репозиторий"), "{md}");
        assert!(md.contains("· Владелец\n- правило владельца"), "{md}");
    }
}
