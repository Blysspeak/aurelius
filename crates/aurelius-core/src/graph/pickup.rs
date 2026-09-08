//! `au pickup --project <P> --json` — один вызов, который пересобирает
//! рабочее состояние после разрыва контекста.
//!
//! Отличие от [`super::snapshot`]: снапшот ведёт слоями по СВЕЖЕСТИ (что
//! записано последним), и старая, но всё ещё актуальная семья фактов вроде
//! `xhub:bank131:refunds` тонет под сегодняшними мелочами. Здесь порядок —
//! от ЯКОРЯ: сперва решить, какой фасет сейчас держит внимание (задача в
//! работе → мода среди недавнего → просто свежесть), а затем показать, что
//! знает граф именно про него, ранжируя степенью внутри найденного обхода
//! (см. [`super::subgraph_degree`]) — с T018 это сигнал `pickup`, `memory_recall`
//! сортирует `rank::score` и на степень не смотрит (**C17**).
//!
//! Бюджет — пять жёстких потолков в символах, а не в записях: `tail` 960,
//! `anchor` 400, `records` 2080, `facets` 300, `critical` 500. Резание честное
//! — где форма позволяет (`tail`/`facets`/`critical` рендерятся объектами),
//! обрезка несёт свой флаг `truncated`, как обход несёт `hidden_nodes`.
//! Единственное исключение — `records`: это ГОЛЫЙ массив (так его проверяет
//! приёмка, `jq '.records | length'`), и добавить туда соседний флаг —
//! сломать именно эту форму; риск смягчён тем, что бюджет на элемент
//! (`claim` уже обрезан до 160) редко подходит вплотную к потолку в 2080.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;
use uuid::Uuid;

use crate::models::{Node, NodeType};
use crate::provenance::Provenance;

const B_ANCHOR: usize = 400;
const B_TAIL: usize = 960;
const B_RECORDS: usize = 2080;
const B_FACETS: usize = 300;
const B_CRITICAL: usize = 500;

/// Сколько сессий просмотреть в поисках непустого хвоста. Измерено 07.09.2026:
/// из 82 cli-сессий хвост несут только 34, так что окно уже в 20-30 последних
/// рискует не найти ни одной — со 120 запас на оба источника разом.
const TAIL_SCAN_WINDOW: usize = 120;

/// Типы, которые формируют «записи» якоря и тот же пул, по которому анкор
/// угадывает фасет через моду/свежесть. Ровно то множество, что назвала задача
/// (problem/decision/concept/solution) — расширение до task/session тоже
/// возможно, но не запрошено и стоило бы вдвое больше выборок ради края
/// случая, который задача не описывала.
const RECORD_TYPES: [NodeType; 4] = [
    NodeType::Problem,
    NodeType::Decision,
    NodeType::Concept,
    NodeType::Solution,
];

/// Машинная форма `au pickup`. Ровно пять ключей — контракт с потребителем,
/// который читает конкретные поля (`tail.source`, `records` как массив), а не
/// разбирает вёрстку.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Pickup {
    pub anchor: String,
    pub tail: Tail,
    pub facets: Facets,
    pub records: Vec<Record>,
    pub critical: Critical,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Tail {
    pub session_id: Option<String>,
    /// `mcp` | `cli` | иной источник записи сессии — то самое поле, на
    /// котором приёмка проверяет предпочтение mcp над cli.
    pub source: Option<String>,
    pub at: Option<String>,
    pub next_steps: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FacetRow {
    pub subject: String,
    pub count: usize,
    pub id8: String,
    pub at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Facets {
    pub items: Vec<FacetRow>,
    pub truncated: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Record {
    pub id8: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub subject: Option<String>,
    pub claim: Option<String>,
    pub confidence: &'static str,
    pub at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CriticalTask {
    pub id8: String,
    pub label: String,
    pub status: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Critical {
    pub items: Vec<CriticalTask>,
    pub truncated: bool,
}

/// Первые восемь шестнадцатеричных символов id — тот же `id8`, которым уже
/// пользуется `au recall` (первый сегмент UUID, разбор без пересчёта строки).
fn id8(id: Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

/// `<repo>:<facet>[:<discriminator>]` → `<repo>:<facet>`. Третий сегмент —
/// частность одного факта, а якорь должен ловить и его соседей с тем же
/// фасетом, но другим дискриминатором; полный subject как префикс их не
/// нашёл бы.
fn facet_of(subject: &str) -> String {
    subject.splitn(3, ':').take(2).collect::<Vec<_>>().join(":")
}

/// Живые узлы `RECORD_TYPES` в области проекта, свежие первыми, из нескольких
/// типизированных выборок слитые в одну. Не новый SQL — то же самое
/// `typed_in_project`, которым уже читает `snapshot::gather`, просто на одном
/// проекте и нескольких типах разом.
fn recent_project_records(conn: &Connection, project: &str, limit: usize) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for t in &RECORD_TYPES {
        nodes.extend(super::typed_in_project(conn, t, Some(project), limit)?);
    }
    nodes.sort_by_key(|n| std::cmp::Reverse(n.updated_at));
    nodes.truncate(limit);
    Ok(nodes)
}

/// Мода фасета среди узлов, что пришли уже отсортированными свежими первыми:
/// при равном счёте побеждает тот, чьё первое (значит, самое свежее)
/// вхождение раньше остальных — `find` по накопленному максимуму, а не
/// `max_by_key`, которого документация Rust честно предупреждает отдавать
/// ПОСЛЕДНИЙ из равных, то есть самый старый в этом порядке.
fn mode_facet(nodes: &[Node]) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for n in nodes {
        let Some(subject) = Provenance::from_data(&n.data).subject else {
            continue;
        };
        let facet = facet_of(&subject);
        match counts.iter_mut().find(|(f, _)| *f == facet) {
            Some((_, c)) => *c += 1,
            None => counts.push((facet, 1)),
        }
    }
    let max_count = counts.iter().map(|(_, c)| *c).max()?;
    counts
        .into_iter()
        .find(|(_, c)| *c == max_count)
        .map(|(f, _)| f)
}

/// Якорный фасет — три попытки по убыванию уверенности, честный отказ на дне.
///
/// 1) Мода фасета среди узлов на расстоянии одного шага от активной задачи
///    проекта («в работе прямо сейчас» — самый сильный сигнал внимания).
/// 2) Мода фасета среди последних `RECORD_TYPES`-узлов проекта.
/// 3) Свежесть — тот же пул, но окно шире: subject самого недавнего узла.
/// 4) Страховка: без единого subject в проекте угадывать нечего, а голый
///    префикс `<project>:` — не выдумка, а честная нижняя граница: он не
///    утечёт в чужой репозиторий (в отличие от префикса `""`, который
///    `find_subject_families_by_prefix` прочитал бы как «весь граф»).
fn anchor_facet(conn: &Connection, project: &str) -> Result<String> {
    let active = super::get_tasks_filtered(conn, Some(project), Some("active"), None, 1)?;
    if let Some(task) = active.first() {
        let (neighbours, _edges) = super::context_from_id(conn, &task.id.to_string(), 1)?;
        if let Some(f) = mode_facet(&neighbours) {
            return Ok(f);
        }
    }

    let recent = recent_project_records(conn, project, 20)?;
    if let Some(f) = mode_facet(&recent) {
        return Ok(f);
    }

    let wider = recent_project_records(conn, project, 200)?;
    if let Some(subject) = wider
        .iter()
        .find_map(|n| Provenance::from_data(&n.data).subject)
    {
        return Ok(facet_of(&subject));
    }

    Ok(format!("{project}:"))
}

/// Ужать сериализованный список под символьный бюджет, роняя ПОСЛЕДНИЕ
/// элементы — список уже пришёл ранжированным, так что худшее по рангу уходит
/// первым. Возвращает, резалось ли на самом деле.
fn fit_budget<T: serde::Serialize>(mut items: Vec<T>, budget: usize) -> (Vec<T>, bool) {
    let mut truncated = false;
    while !items.is_empty() {
        let size = serde_json::to_string(&items)
            .map(|s| s.chars().count())
            .unwrap_or(usize::MAX);
        if size <= budget {
            break;
        }
        items.pop();
        truncated = true;
    }
    (items, truncated)
}

/// Тот же приём, что [`fit_budget`], но для `Tail` — не списка, а одного
/// объекта, где резать есть только один список: `next_steps`.
fn fit_tail(mut tail: Tail, budget: usize) -> Tail {
    loop {
        let size = serde_json::to_string(&tail)
            .map(|s| s.chars().count())
            .unwrap_or(usize::MAX);
        if size <= budget || tail.next_steps.is_empty() {
            if size > budget {
                tail.truncated = true;
            }
            return tail;
        }
        tail.next_steps.pop();
        tail.truncated = true;
    }
}

/// Самая свежая сессия проекта с непустым `next_steps`, mcp впереди cli.
///
/// Измерено 07.09.2026: у cli-сессий хвост несут 34 из 82 (среднее 3.7
/// пункта), у mcp — все 73 (среднее 5.5). Замок пишет через cli дежурными
/// строками, которые заслоняют настоящий хвост модели, поэтому предпочтение
/// mcp — не эстетика, а фильтр шума.
fn build_tail(conn: &Connection, project: &str) -> Result<Tail> {
    let sessions =
        super::typed_in_project(conn, &NodeType::Session, Some(project), TAIL_SCAN_WINDOW)?;
    let with_tail: Vec<&Node> = sessions
        .iter()
        .filter(|s| {
            s.data
                .get("next_steps")
                .and_then(|v| v.as_array())
                .is_some_and(|a| !a.is_empty())
        })
        .collect();

    // `typed_in_project` уже отдаёт узлы свежими первыми, так что первое
    // совпадение в каждой группе и есть самое свежее в ней.
    let chosen = with_tail
        .iter()
        .find(|s| s.source == "mcp")
        .or_else(|| with_tail.first());

    let Some(session) = chosen else {
        return Ok(Tail {
            session_id: None,
            source: None,
            at: None,
            next_steps: Vec::new(),
            truncated: false,
        });
    };

    let steps: Vec<String> = session
        .data
        .get("next_steps")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| super::clip(s, 180))
                .collect()
        })
        .unwrap_or_default();

    Ok(fit_tail(
        Tail {
            session_id: Some(session.id.to_string()),
            source: Some(session.source.clone()),
            at: Some(session.updated_at.to_rfc3339()),
            next_steps: steps,
            truncated: false,
        },
        B_TAIL,
    ))
}

/// До 8 problem(без solves)/decision/concept/solution под якорным фасетом,
/// ранжированных степенью в подграфе обхода `anchor`, при равенстве — новее
/// первым. Один обход на весь вызов `build_pickup` (степень приходит уже
/// посчитанной) — второго BFS ради этой же цели здесь нет.
fn build_records(
    conn: &Connection,
    project: &str,
    anchor: &str,
    degree: &HashMap<Uuid, usize>,
) -> Result<Vec<Record>> {
    let under_anchor = |n: &Node| -> bool {
        Provenance::from_data(&n.data)
            .subject
            .as_deref()
            .is_some_and(|s| s == anchor || s.starts_with(&format!("{anchor}:")))
    };

    let mut candidates: Vec<(Node, &'static str)> =
        super::get_unsolved_problems(conn, Some(project), 200)?
            .into_iter()
            .filter(|n| under_anchor(n))
            .map(|n| (n, "problem"))
            .collect();

    for (t, kind) in [
        (NodeType::Decision, "decision"),
        (NodeType::Concept, "concept"),
        (NodeType::Solution, "solution"),
    ] {
        candidates.extend(
            super::typed_in_project(conn, &t, Some(project), 200)?
                .into_iter()
                .filter(|n| under_anchor(n))
                .map(|n| (n, kind)),
        );
    }

    candidates.sort_by(|(a, _), (b, _)| super::by_degree_then_recency(degree, a, b));
    candidates.truncate(8);

    let records = candidates
        .into_iter()
        .map(|(n, kind)| {
            let p = Provenance::from_data(&n.data);
            let confidence = p.confidence_or_default().as_str();
            Record {
                id8: id8(n.id),
                kind,
                subject: p.subject,
                claim: p
                    .claim
                    .or_else(|| n.note.clone())
                    .map(|c| super::clip(&c, 160)),
                confidence,
                at: n.updated_at.to_rfc3339(),
            }
        })
        .collect();

    // `truncated` здесь намеренно не читается: форма `records` — голый
    // массив (см. doc модуля), отбросить лишнее без соседнего флага — это
    // тот самый принятый компромисс.
    let (records, _truncated) = fit_budget(records, B_RECORDS);
    Ok(records)
}

/// Фасеты проекта — семьи subject с префиксом `<project>:`, вызов той же
/// функции, что и `au recall --prefix` (лежит в `crud::find_subject_families_by_prefix`,
/// SQL здесь не пишется заново), ранжированные тем же обходом, что и `records`.
fn build_facets(conn: &Connection, project: &str, degree: &HashMap<Uuid, usize>) -> Result<Facets> {
    let prefix = format!("{project}:");
    let mut families = super::find_subject_families_by_prefix(conn, &prefix)?;
    families.sort_by(|a, b| super::by_degree_then_recency(degree, &a.newest, &b.newest));

    let items: Vec<FacetRow> = families
        .into_iter()
        .map(|f| FacetRow {
            subject: f.subject,
            count: f.count,
            id8: id8(f.newest.id),
            at: f.newest.created_at.to_rfc3339(),
        })
        .collect();

    let (items, truncated) = fit_budget(items, B_FACETS);
    Ok(Facets { items, truncated })
}

/// Открытые задачи приоритета `critical`, до пяти. `get_tasks_filtered` уже
/// умеет фильтровать по приоритету — второй выборки под это заводить не надо.
fn build_critical(conn: &Connection, project: &str) -> Result<Critical> {
    let tasks = super::get_tasks_filtered(
        conn,
        Some(project),
        Some(super::OPEN_TASK_STATUSES),
        Some("critical"),
        5,
    )?;
    let items: Vec<CriticalTask> = tasks
        .into_iter()
        .map(|n| CriticalTask {
            id8: id8(n.id),
            label: super::clip(&n.label, 120),
            status: n
                .data
                .get("status")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            at: n.updated_at.to_rfc3339(),
        })
        .collect();
    let (items, truncated) = fit_budget(items, B_CRITICAL);
    Ok(Critical { items, truncated })
}

/// Собрать `au pickup`. Только чтение — тот же контракт, что у
/// [`super::build_snapshot`].
pub fn build_pickup(conn: &Connection, project: &str) -> Result<Pickup> {
    let anchor_raw = anchor_facet(conn, project)?;
    let anchor = super::clip(&anchor_raw, B_ANCHOR);

    let tail = build_tail(conn, project)?;

    // Один обход обслуживает и `records`, и `facets` — вторая прогулка по
    // графу ради того же самого сигнала была бы дублированием, которого
    // просило избежать задание.
    let traversal = super::context_with_report_seeded(conn, &anchor_raw, 2, super::RECALL_SEEDS)?;
    let degree = super::subgraph_degree(&traversal.edges);

    let records = build_records(conn, project, &anchor_raw, &degree)?;
    let facets = build_facets(conn, project, &degree)?;
    let critical = build_critical(conn, project)?;

    Ok(Pickup {
        anchor,
        tail,
        facets,
        records,
        critical,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::Relation;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-pickup-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    fn session_with(conn: &Connection, project: &str, source: &str, steps: &[&str]) -> Node {
        super::super::add_node_full(
            conn,
            NodeType::Session,
            &format!("[{project}] сессия"),
            Some("итог"),
            source,
            serde_json::json!({ "project": project, "next_steps": steps }),
            crate::models::MemoryKind::Episodic,
            None,
        )
        .expect("add session")
    }

    /// Смысл всей команды: mcp обязан перебить cli, даже если cli-сессия
    /// свежее — иначе дежурные строки замка снова заслоняют хвост модели.
    #[test]
    fn tail_prefers_mcp_over_more_recent_cli() {
        let conn = test_conn();
        session_with(&conn, "demo", "cli", &["хвост cli, он свежее"]);
        std::thread::sleep(std::time::Duration::from_millis(5));
        session_with(&conn, "demo", "mcp", &["хвост mcp"]);

        let tail = build_tail(&conn, "demo").expect("tail");
        assert_eq!(tail.source.as_deref(), Some("mcp"));
        assert_eq!(tail.next_steps, vec!["хвост mcp".to_owned()]);
    }

    /// Сессии без next_steps не годятся в хвост, даже если это единственная
    /// сессия проекта — иначе подъём подсунул бы пустышку как «хвост».
    #[test]
    fn tail_skips_sessions_without_next_steps() {
        let conn = test_conn();
        super::super::add_node_full(
            &conn,
            NodeType::Session,
            "[demo] сессия без хвоста",
            Some("итог"),
            "mcp",
            serde_json::json!({ "project": "demo" }),
            crate::models::MemoryKind::Episodic,
            None,
        )
        .expect("add session");

        let tail = build_tail(&conn, "demo").expect("tail");
        assert!(tail.source.is_none());
        assert!(tail.next_steps.is_empty());
    }

    /// Без единого subject в проекте якорь обязан остаться безопасным
    /// префиксом проекта, а не пустой строкой (которая читалась бы как «весь
    /// граф» в `find_subject_families_by_prefix`).
    #[test]
    fn anchor_falls_back_to_project_prefix_without_any_subject() {
        let conn = test_conn();
        let anchor = anchor_facet(&conn, "demo").expect("anchor");
        assert_eq!(anchor, "demo:");
    }

    /// Мода фасета среди недавних узлов проекта, при равном счёте — более
    /// свежий факт побеждает.
    #[test]
    fn anchor_picks_mode_facet_among_recent_nodes() {
        let conn = test_conn();
        for (subj, claim) in [
            ("demo:billing", "billing раз"),
            ("demo:billing", "billing два"),
            ("demo:refunds", "refunds один"),
        ] {
            super::super::add_node(
                &conn,
                NodeType::Decision,
                "[demo] метка",
                None,
                "test",
                serde_json::json!({ "subject": subj, "claim": claim }),
            )
            .expect("add decision");
        }

        let anchor = anchor_facet(&conn, "demo").expect("anchor");
        assert_eq!(anchor, "demo:billing");
    }

    /// Records — голый массив (приёмка читает `.records | length`), и он
    /// обязан оставаться в пределах восьми даже когда кандидатов больше.
    #[test]
    fn records_are_capped_at_eight() {
        let conn = test_conn();
        for i in 0..15 {
            super::super::add_node(
                &conn,
                NodeType::Decision,
                &format!("[demo] решение {i}"),
                None,
                "test",
                serde_json::json!({
                    "subject": "demo:overflow",
                    "claim": format!("решение номер {i}"),
                }),
            )
            .expect("add decision");
        }

        let payload = build_pickup(&conn, "demo").expect("pickup");
        assert!(payload.records.len() <= 8, "{}", payload.records.len());
    }

    /// FR аналог снапшота: секретные координаты не имеют права дойти ни до
    /// records, ни до facets, ни до critical.
    #[test]
    fn pickup_excludes_secret_coordinates_from_records() {
        let conn = test_conn();
        super::super::add_secret_ref(
            &conn,
            Some("demo"),
            "STRIPE_SECRET_KEY",
            None,
            "1password://Private/Stripe/api-key",
        )
        .expect("add secret ref");
        super::super::add_node(
            &conn,
            NodeType::Decision,
            "[demo] решение",
            None,
            "test",
            serde_json::json!({ "subject": "demo:billing", "claim": "видимое решение" }),
        )
        .expect("add decision");

        let payload = build_pickup(&conn, "demo").expect("pickup");
        assert!(payload
            .records
            .iter()
            .all(|r| r.claim.as_deref() != Some("STRIPE_SECRET_KEY")));
    }

    /// Полный обход `build_pickup`: все пять ключей на месте, критичная
    /// задача проекта видна.
    #[test]
    fn build_pickup_assembles_all_five_keys() {
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
        let task = super::super::add_node(
            &conn,
            NodeType::Task,
            "[demo] важная задача",
            Some("чинит бюджет pickup"),
            "test",
            serde_json::json!({ "status": "backlog", "priority": "critical" }),
        )
        .expect("add task");
        super::super::add_edge(&conn, task.id, project.id, Relation::BelongsTo, 1.0)
            .expect("link task to project");
        session_with(&conn, "demo", "mcp", &["продолжить чинить бюджет"]);

        let payload = build_pickup(&conn, "demo").expect("pickup");
        assert!(!payload.anchor.is_empty());
        assert_eq!(payload.tail.source.as_deref(), Some("mcp"));
        assert_eq!(payload.critical.items.len(), 1);
        assert_eq!(payload.critical.items[0].label, "[demo] важная задача");
    }
}
