//! Слой 7-уровневой памяти: замороженный снапшот и дистилляция.
//!
//! Снапшот — компактный Markdown (жёсткий бюджет символов на слой, ~4.5К всего),
//! который инжектится в контекст агента ОДИН раз при старте сессии. Урок
//! hermes-agent: маленький курируемый срез в системном промпте бьёт большой
//! JSON по запросу — и не ломает prefix-cache, потому что заморожен на сессию.
//!
//! Дистиллят (слой 7) — структурная выжимка без LLM: незакрытые next_steps
//! последних сессий + нерешённые проблемы, пересобирается consolidate().

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::models::{MemoryKind, Node, NodeType};

/// Бюджеты слоёв в символах. Сумма ~4500 — порядка 1.5К токенов.
const B_IDENTITY: usize = 600;
const B_WORKING: usize = 1000;
const B_EPISODIC: usize = 800;
const B_SEMANTIC: usize = 900;
const B_PROCEDURAL: usize = 500;
const B_DIGEST: usize = 500;

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

/// Координата секрета (`Config` с `data.kind == "secret_ref"`) — FR-027:
/// координаты отдаются по запросу (`au secret list`) и НЕ ДОЛЖНЫ попадать в
/// подаваемую память автоматически. Ни один из типов, перечисленных в
/// [`gather`], сегодня не выбирает `Config`, так что фильтр здесь — граница,
/// а не текущая необходимость: он держит инвариант верным и тогда, когда
/// слой снапшота однажды расширят.
fn is_secret_ref(n: &Node) -> bool {
    matches!(n.node_type, NodeType::Config)
        && n.data.get("kind").and_then(|v| v.as_str()) == Some("secret_ref")
}

/// Строки слоя: по одной на узел, суммарно не больше budget.
fn layer(nodes: &[Node], per_line: usize, budget: usize) -> String {
    let mut out = String::new();
    for n in nodes.iter().filter(|n| !is_secret_ref(n)) {
        let line = format!("- {}\n", annotate(n, body(n, per_line)));
        if out.chars().count() + line.chars().count() > budget {
            break;
        }
        out.push_str(&line);
    }
    out
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
    digest: Vec<Node>,
}

fn gather(conn: &Connection, project: Option<&str>) -> Result<Gathered> {
    // Аварийный предел применяется в Rust, не в SQL: выборка сортирует по
    // приоритету/дате заведения, а предел FR-018 — по времени взятия в работу.
    let mut active_tasks = super::get_tasks_filtered(conn, project, Some("active"), None, 200)?;
    active_tasks.sort_by_key(|n| std::cmp::Reverse(activated_key(n)));
    let active_overflow = active_tasks.len().saturating_sub(ACTIVE_TASK_CAP);
    active_tasks.truncate(ACTIVE_TASK_CAP);

    Ok(Gathered {
        identity: typed_recent(conn, &NodeType::UserFact, None, 12)?,
        active_tasks,
        active_overflow,
        other_tasks: super::get_tasks_filtered(
            conn,
            project,
            Some(OTHER_OPEN_TASK_STATUSES),
            None,
            8,
        )?,
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
        skills: typed_recent(conn, &NodeType::Skill, None, 8)?,
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

/// Собрать семислойный снапшот. Только чтение, без сети и индексации —
/// вызывается хуком на старте каждой сессии и обязан быть мгновенным.
pub fn build_snapshot(conn: &Connection, project: Option<&str>) -> Result<String> {
    let ts = Utc::now().format("%Y-%m-%d %H:%M");
    let scope = project.unwrap_or("глобально");

    let g = gather(conn, project)?;
    let nodes = super::count_nodes(conn)?;
    let edges = super::count_edges(conn)?;

    // Активные задачи — высший приоритет слоя «В работе» (FR-017): рендерятся
    // без бюджетного среза, так что ни одна не теряется из-за нехватки места;
    // защита от неограниченного разрастания — аварийный предел ACTIVE_TASK_CAP
    // уже применён в gather(). Здесь просто печатаем, сколько не поместилось.
    let mut working = layer(&g.active_tasks, 160, usize::MAX);
    if g.active_overflow > 0 {
        working.push_str(&format!(
            "- …и ещё {} активных не поместилось\n",
            g.active_overflow
        ));
    }
    let active_used = working.chars().count();

    // Остаток бюджета «В работе» — прочим открытым задачам (FR-019), а то, что
    // они не выбрали, — проблемам. Раньше половины делились поровну и
    // фиксированно: пустая проблема не отдавала место задачам и наоборот.
    let working_rest = B_WORKING.saturating_sub(active_used);
    let other = layer(&g.other_tasks, 160, working_rest);
    let other_used = other.chars().count();
    working.push_str(&other);
    working.push_str(&layer(
        &g.problems,
        160,
        working_rest.saturating_sub(other_used),
    ));

    // Если активные перебрали весь бюджет слоя, разница вычитается у слоя ниже
    // приоритетом (FR-017), а не у активных: «Решения и знания» отдаёт ровно
    // столько, сколько не хватило.
    let semantic_budget = B_SEMANTIC.saturating_sub(active_used.saturating_sub(B_WORKING));

    let mut semantic = layer(&g.decisions, 150, semantic_budget * 2 / 3);
    semantic.push_str(&layer(&g.concepts, 150, semantic_budget / 3));

    let pressure = g
        .pressure
        .iter()
        .map(|line| format!("- {line}\n"))
        .collect::<String>();

    let (identity, sessions, skills, digest) = (&g.identity, &g.sessions, &g.skills, &g.digest);

    let sections: [(&str, String); 8] = [
        ("1 · Владелец", layer(identity, 120, B_IDENTITY)),
        ("2 · В работе (задачи и открытые проблемы)", working),
        ("3 · Давление (незакрытые обязательства)", pressure),
        ("4 · Последние сессии", layer(sessions, 250, B_EPISODIC)),
        ("5 · Решения и знания", semantic),
        ("6 · Приёмы", layer(skills, 100, B_PROCEDURAL)),
        (
            "7 · Архив",
            format!(
                "- {nodes} узлов, {edges} рёбер; глубже — memory_recall(topic) / memory_search(query)\n"
            ),
        ),
        ("8 · Дистиллят", layer(digest, B_DIGEST, B_DIGEST)),
    ];

    let mut md = format!("# Память Aurelius · {scope} · {ts} UTC\n");
    for (title, body) in sections {
        if body.is_empty() {
            continue;
        }
        md.push_str(&format!("\n## {title}\n{body}"));
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
        assert!(md.starts_with("# Память Aurelius · demo"));
        assert!(md.contains("1 · Владелец"));
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
            md.contains("5 · Решения и знания"),
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
            md.contains("слои 1-6 не доезжают"),
            "свежая задача в backlog обязана быть видна:\n{md}"
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
            md.contains("чиним слой активных задач в снапшоте"),
            "активная задача обязана присутствовать целиком среди 200 открытых:\n{md}"
        );
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

    /// FR-020: сокращение идёт по границе слова. Раньше `clip` рубил по счётчику
    /// символов вслепую — обрезанный текст мог заканчиваться на полуслове.
    #[test]
    fn dump_lines_never_cut_mid_word() {
        let conn = test_conn();
        let long_note = (0..40)
            .map(|i| format!("слово{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        super::super::add_node(
            &conn,
            NodeType::UserFact,
            "владелец",
            Some(&long_note),
            "test",
            serde_json::json!({}),
        )
        .expect("add user fact");

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
}
