mod crud;
mod export;
mod import;
mod lease;
mod path;
mod pickup;
mod rank;
mod render;
mod search;
mod session;
mod snapshot;
mod traverse;

pub use crud::*;
pub use export::*;
pub use import::*;
pub use lease::*;
pub use path::*;
pub use pickup::*;
pub use rank::*;
pub use render::*;
pub use search::*;
pub use session::*;
pub use snapshot::*;
pub use traverse::*;

use crate::models::{Edge, MemoryKind, Node, NodeType, Relation};
use chrono::Utc;
use uuid::Uuid;

/// Улика прогона, которую не к чему привязать: в проекте нет активной задачи.
///
/// Своё значение, а не `USAGE`. Код `1` означает «вызвали неправильно», и
/// вызывающий, получив его, чинит собственный вызов — тогда как чинить надо
/// состояние проекта: завести или активировать задачу. Измерено 07.09.2026:
/// на репозитории ulika мост улик отказывал ВСЕГДА (30 задач, все `done` или
/// `backlog`, активной ни одной), и каждая зелёная улика падала на пол, потому
/// что законная ситуация была неотличима от кривого вызова. Тот же принцип,
/// по которому разведены [`LeaseError::NoTasksAvailable`] (10) и
/// [`LeaseError::Busy`] (11).
///
/// `run: None` — узла прогона нет вовсе: не было и узла проекта, к которому
/// его прицепить (см. [`link_evidence_run`]).
#[derive(Debug, thiserror::Error)]
#[error("в проекте '{project}' нет активной задачи — {}", run_fate(.run))]
pub struct NoActiveTask {
    pub project: String,
    pub run: Option<uuid::Uuid>,
}

fn run_fate(run: &Option<uuid::Uuid>) -> String {
    match run {
        Some(run) => format!("улика привязана к проекту без задачи: {run}"),
        None => "узла проекта нет, узел улики не заведён; прогон остался в журнале вызывающего"
            .to_owned(),
    }
}

/// Заводит узел прогона и связывает его с задачей ребром `verified_by`
/// (спека 007, T013/T014, data-model.md «Ребро»). Улика внутри `data.evidence`
/// задачи — для быстрого чтения без обхода графа; этот узел и ребро — для
/// обратного пути: от прогона к задаче, которую он подтвердил.
///
/// `task_id: None` — в проекте нет активной задачи. Тогда узел цепляется
/// ребром `belongs_to` к УЖЕ существующему узлу проекта, а если такого нет
/// (или проект не назван) — не пишется вовсе, `Ok(None)`. Раньше он писался
/// без единого ребра: 19.09.2026 таких сирот было 81 из 401, ни одна
/// выборка через граф их не находила. Сам прогон при этом не теряется —
/// команда, код возврата, артефакт и `subject` уже лежат в журнале
/// вызывающего (ulika). Узел проекта здесь не заводится: пустая заглушка
/// проекта — тот же мусор, только другого типа.
///
/// Пишется через `upsert_node_by_key` (`crud.rs:128`), а не голым `add_node`:
/// без ключа один и тот же прогон, повторённый N раз, заводил бы N узлов
/// (измерено 16.09.2026: 4119 таких узлов из 25707, 2916 — дубликаты по
/// метке). Ключ — обязательно с префиксом `run:`: `upsert_node_by_key` ищет
/// совпадение только по значению `key` (`find_node_by_data_field`,
/// `crud.rs:504`), без фильтра по типу или источнику — `expected_type:
/// Some(NodeType::Run)` здесь ровно затем, чтобы совпадение по ключу с узлом
/// чужого типа было отказом, а не тихой перезаписью чужой записи узлом
/// прогона. Ключ без своего пространства имён мог бы случайно совпасть с
/// чужим. `subject` — уже нормализованный хуком адрес прогона
/// (`<project>:verify:<key>`); без него (вызов не от хука) в ключ идут
/// проект и команда — обе формы всё равно живут под одним префиксом.
pub fn link_evidence_run(
    conn: &rusqlite::Connection,
    task_id: Option<Uuid>,
    project: Option<&str>,
    subject: Option<&str>,
    command: &str,
    exit_code: i64,
    artifact: Option<&str>,
) -> anyhow::Result<Option<Uuid>> {
    let project_node = match (task_id, project) {
        (Some(_), _) => None,
        (None, Some(project)) => match crud::find_project_by_label(conn, project)? {
            Some(node) => Some(node.id),
            None => return Ok(None),
        },
        (None, None) => return Ok(None),
    };
    let label = format!("прогон: {command}");
    let key = match subject {
        Some(subject) => format!("run:{subject}"),
        None => format!("run:{}:{command}", project.unwrap_or("")),
    };
    let now = Utc::now();

    // `upsert_node_by_key` заменяет `data` целиком — счётчик и первая метка
    // времени читаются из старой записи ДО вызова и переносятся руками,
    // иначе повтор сбрасывал бы счётчик на единицу и весь смысл схлопывания
    // терялся.
    let existing = crud::find_node_by_data_field(conn, "key", &key)?;
    let run_count = existing
        .as_ref()
        .and_then(|n| n.data.get("run_count"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
        + 1;
    let first_seen_at = existing
        .as_ref()
        .and_then(|n| n.data.get("first_seen_at"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| now.to_rfc3339());

    let mut data = serde_json::Map::new();
    // Командой служит сама команда — улика, а не её адрес: `subject` лишь
    // указывает, куда положить узел, и не подменяет собой то, что реально
    // выполнялось.
    data.insert("command".to_owned(), serde_json::json!(command));
    data.insert("artifact".to_owned(), serde_json::json!(artifact));
    data.insert("project".to_owned(), serde_json::json!(project));
    data.insert("subject".to_owned(), serde_json::json!(subject));
    // Провенанс прогона не спрашивается у вызывающего, а выводится: раз
    // улика существует, прогон состоялся, командой служит он сам. Просить
    // хук передать `--confidence measured` значило бы просить его ввести
    // то, что уже известно отсюда.
    data.insert("confidence".to_owned(), serde_json::json!("measured"));
    data.insert("evidence".to_owned(), serde_json::json!(command));
    data.insert("run_count".to_owned(), serde_json::json!(run_count));
    data.insert("first_seen_at".to_owned(), serde_json::json!(first_seen_at));
    data.insert(
        "last_seen_at".to_owned(),
        serde_json::json!(now.to_rfc3339()),
    );
    data.insert("last_exit_code".to_owned(), serde_json::json!(exit_code));

    let (run, _created, _replaced) = crud::upsert_node_by_key(
        conn,
        &key,
        NodeType::Run,
        Some(NodeType::Run),
        &label,
        None,
        "au-task-evidence",
        data,
        MemoryKind::Semantic,
    )?;
    if let Some(task_id) = task_id {
        crud::add_edge(conn, task_id, run.id, Relation::VerifiedBy, 1.0)?;
    }
    if let Some(project_id) = project_node {
        crud::add_edge(conn, run.id, project_id, Relation::BelongsTo, 1.0)?;
    }
    Ok(Some(run.id))
}

/// Заводит координату секрета — узел `Config` с признаком `kind: "secret_ref"`
/// (спека 007, US4, T039, data-model.md). Значения секрета здесь нет ни в
/// одном поле: вызывающий обязан прогнать `location` через
/// `secret::detect_lookalike` до вызова — эта функция только пишет.
///
/// Метка следует соглашению задач: `[project] name`, если проект назван, иначе
/// голое имя — так `typed_in_project` находит координату тем же механизмом
/// области видимости, что и прочие типы узлов.
pub fn add_secret_ref(
    conn: &rusqlite::Connection,
    project: Option<&str>,
    name: &str,
    purpose: Option<&str>,
    location: &str,
) -> anyhow::Result<Node> {
    let location_kind = crate::secret::infer_location_kind(location);
    let label = match project {
        Some(p) => format!("[{p}] {name}"),
        None => name.to_owned(),
    };
    let data = serde_json::json!({
        "kind": "secret_ref",
        "name": name,
        "purpose": purpose,
        "location": location,
        "location_kind": location_kind.as_str(),
    });
    crud::add_node(conn, NodeType::Config, &label, None, "au-secret", data)
}

/// Живые координаты секретов, свежие первыми. Область видимости — та же, что
/// у `typed_in_project`: без `project` отдаёт координаты всех проектов.
///
/// Тип `Config` уже занят прочими настройками, поэтому фильтр по
/// `data.kind == "secret_ref"` обязателен — иначе `au secret list` показал бы
/// чужие конфигурационные узлы.
pub fn list_secret_refs(
    conn: &rusqlite::Connection,
    project: Option<&str>,
) -> anyhow::Result<Vec<Node>> {
    let mut nodes = search::typed_in_project(conn, &NodeType::Config, project, 500)?;
    nodes.retain(crate::secret::is_secret_ref);
    Ok(nodes)
}

/// Правило `au db prune`. Других нет: всё, что несёт `claim`, не удаляется
/// никогда — кроме прогона ([`PruneRule::TechnicalJunk`]), — а знание без
/// связей только считается ([`PrunePlan::unlinked_knowledge`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneRule {
    /// Прогон или зависимость без рёбер, без `claim` и без тела.
    TechnicalOrphan,
    /// Остальное техническое ([`search::is_technical`]) с рёбрами или без:
    /// любой прогон, зависимость без `claim` и тела. Сирот забирает
    /// [`Self::TechnicalOrphan`] — его счёт цитируется в отчётах владельца.
    TechnicalJunk,
    /// Дистиллят, не самый свежий для своего проекта.
    StaleDigest,
    /// Узел проекта без рёбер, без содержания и без единой записи, которая
    /// называла бы его своим (префикс метки `[p]` или `data.project`).
    EmptyProject,
}

impl PruneRule {
    #[must_use]
    pub fn why(self) -> &'static str {
        match self {
            Self::TechnicalOrphan => {
                "прогон или зависимость без рёбер, claim и тела: ни одна выборка через граф \
                 его не находит, прогон лежит в журнале ulika, зависимость — в манифесте"
            }
            Self::TechnicalJunk => {
                "любой прогон и зависимость без claim и тела, даже с рёбрами: история проверки \
                 живёт в data.evidence задачи, прогон и ребро verified_by — её зеркало, \
                 зависимость — строка манифеста"
            }
            Self::StaleDigest => {
                "не самый свежий дистиллят проекта: близнец от гонки параллельных \
                 `au snapshot --hook`, снапшот читает только один"
            }
            Self::EmptyProject => {
                "узел проекта без рёбер, без содержания и без записей под его именем: \
                 заглушка, которую никто не наполнил"
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PruneCandidate {
    pub rule: PruneRule,
    pub id: Uuid,
    pub node_type: String,
    pub label: String,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct PrunePlan {
    pub candidates: Vec<PruneCandidate>,
    /// Живых узлов всего — чтобы было видно, от чего доля.
    pub live_nodes: usize,
    /// Узлы с `claim` или телом и без единого ребра, по типу. Не в счёт:
    /// дистилляты (их пишет машина) и то, что глобально по замыслу — проект,
    /// факт о владельце, навык (та же тройка, которой `memory_add` не
    /// предупреждает о непривязанности). Не удаляются: причина их сиротства —
    /// писатель без привязки, а не лишняя запись.
    pub unlinked_knowledge: std::collections::BTreeMap<String, usize>,
}

/// Имя типа так, как оно лежит в базе: `user_fact`, а не `userfact`.
fn type_name(node_type: &NodeType) -> String {
    match serde_json::to_value(node_type) {
        Ok(serde_json::Value::String(name)) => name,
        _ => format!("{node_type:?}").to_lowercase(),
    }
}

/// Что снял бы `au db prune`, ничего не трогая. Один проход по живым узлам и
/// концам живых рёбер: правила зависят от состояния графа целиком (кто
/// свежее, кто кого называет своим), и запрос на узел здесь стоил бы сотни
/// полных просмотров.
///
/// # Errors
/// Ошибка чтения `nodes` или `edges`.
pub fn prune_plan(conn: &rusqlite::Connection) -> anyhow::Result<PrunePlan> {
    let mut linked = std::collections::HashSet::new();
    let mut stmt = conn.prepare("SELECT from_id, to_id FROM edges WHERE deleted_at IS NULL")?;
    for pair in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (from, to) = pair?;
        linked.insert(from);
        linked.insert(to);
    }
    let nodes = crud::get_all_nodes(conn)?;
    let has_claim = |n: &Node| {
        crate::provenance::Provenance::from_data(&n.data)
            .claim
            .is_some_and(|c| !c.trim().is_empty())
    };

    let mut owners = std::collections::HashSet::new();
    let mut newest_digest: std::collections::HashMap<String, &Node> =
        std::collections::HashMap::new();
    let digest_owner = |n: &Node| {
        n.data
            .get("project")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| n.label.clone(), str::to_owned)
    };
    for n in &nodes {
        if let Some((p, _)) = n.label.strip_prefix('[').and_then(|r| r.split_once(']')) {
            owners.insert(p.to_owned());
        }
        if let Some(p) = n.data.get("project").and_then(serde_json::Value::as_str) {
            owners.insert(p.to_owned());
        }
        if matches!(n.node_type, NodeType::Digest) {
            let slot = newest_digest.entry(digest_owner(n)).or_insert(n);
            if (n.updated_at, n.created_at) > (slot.updated_at, slot.created_at) {
                *slot = n;
            }
        }
    }

    let mut plan = PrunePlan {
        live_nodes: nodes.len(),
        ..PrunePlan::default()
    };
    for n in &nodes {
        let edgeless = !linked.contains(&n.id.to_string());
        let rule = match &n.node_type {
            t if (search::is_run(n) || matches!(t, NodeType::Dependency))
                && edgeless
                && search::is_bare(n) =>
            {
                Some(PruneRule::TechnicalOrphan)
            }
            _ if search::is_technical(n) => Some(PruneRule::TechnicalJunk),
            NodeType::Digest
                if !has_claim(n)
                    && newest_digest
                        .get(&digest_owner(n))
                        .is_some_and(|newest| newest.id != n.id) =>
            {
                Some(PruneRule::StaleDigest)
            }
            NodeType::Project if edgeless && search::is_bare(n) && !owners.contains(&n.label) => {
                Some(PruneRule::EmptyProject)
            }
            _ => None,
        };
        match rule {
            Some(rule) => plan.candidates.push(PruneCandidate {
                rule,
                id: n.id,
                node_type: type_name(&n.node_type),
                label: n.label.clone(),
            }),
            None if edgeless
                && !search::is_bare(n)
                && !matches!(
                    n.node_type,
                    NodeType::Digest | NodeType::Project | NodeType::UserFact | NodeType::Skill
                ) =>
            {
                *plan
                    .unlinked_knowledge
                    .entry(type_name(&n.node_type))
                    .or_default() += 1;
            }
            None => {}
        }
    }
    plan.candidates.sort_by_key(|c| c.rule);
    Ok(plan)
}

/// Снять всё, что нашёл [`prune_plan`], — мягко, как [`delete_node`]
/// (`deleted_at`, отменяемо), одной транзакцией: план считается уже под
/// замком записи, так что снимается ровно то, что посчитано.
///
/// # Errors
/// Ошибка плана или удаления; тогда не снято ничего.
pub fn prune_apply(conn: &rusqlite::Connection) -> anyhow::Result<PrunePlan> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let plan = prune_plan(&tx)?;
    for candidate in &plan.candidates {
        crud::delete_node(&tx, candidate.id)?;
    }
    tx.commit()?;
    Ok(plan)
}

/// Степень каждого узла внутри уже найденного подграфа обхода: по скольким
/// рёбрам из `edges` он виден. Мера того, насколько запись держит тему обхода,
/// а не того, как часто слово встретилось в её теле (найдено 07.09.2026 на
/// теме «ulika»: мёртвые windows-пути повторяли имя проекта десятками раз и
/// выигрывали по частоте терма) — этим сигналом по-прежнему ранжирует
/// `au pickup` (`graph::pickup`, `pickup.rs:335,370`), заявленно и намеренно
/// (**C17**, `contracts/mcp.md` §4 п.16). `memory_recall` (MCP) с T018 ушёл
/// на `rank::score` (`graph::recall_selection`, `traverse.rs`), где степени
/// нет ни в множителях, ни в подсчёте — это разные пути с разным порядком,
/// а не два потребителя одной формулы.
pub fn subgraph_degree(edges: &[Edge]) -> std::collections::HashMap<Uuid, usize> {
    let mut degree = std::collections::HashMap::new();
    for edge in edges {
        *degree.entry(edge.from_id).or_insert(0usize) += 1;
        *degree.entry(edge.to_id).or_insert(0usize) += 1;
    }
    degree
}

/// Компаратор `au pickup`: выше степень в найденном подграфе первой, при
/// равенстве — новее `created_at` первым. Узел вне карты степеней (не
/// встретился в обходе) читается как степень 0, а не как ошибка. Единственные
/// потребители — `pickup.rs:335,370`; `memory_recall` с T018 сортирует
/// `rank::score` и этот компаратор не зовёт (**C17**).
pub fn by_degree_then_recency(
    degree: &std::collections::HashMap<Uuid, usize>,
    a: &Node,
    b: &Node,
) -> std::cmp::Ordering {
    let da = degree.get(&a.id).copied().unwrap_or(0);
    let db = degree.get(&b.id).copied().unwrap_or(0);
    db.cmp(&da).then(b.created_at.cmp(&a.created_at))
}

pub(crate) fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    let memory_kind_str: String = row
        .get::<_, String>(8)
        .unwrap_or_else(|_| "semantic".to_owned());
    let memory_kind = match memory_kind_str.as_str() {
        "episodic" => MemoryKind::Episodic,
        _ => MemoryKind::Semantic,
    };

    let last_accessed_str: Option<String> = row.get(9).ok();
    let last_accessed_at = last_accessed_str
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(Utc::now);

    Ok(Node {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        node_type: serde_json::from_str(&row.get::<_, String>(1)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        label: row.get(2)?,
        note: row.get(3)?,
        source: row.get(4)?,
        data: serde_json::from_str(&row.get::<_, String>(5)?)
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_at: row
            .get::<_, String>(6)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        updated_at: row
            .get::<_, String>(7)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        memory_kind,
        last_accessed_at,
        access_count: row.get(10).unwrap_or(0),
        content_hash: row.get(11).ok().and_then(|v: Option<String>| v),
        created_by: row.get(12).ok().and_then(|v: Option<String>| v),
        updated_by: row.get(13).ok().and_then(|v: Option<String>| v),
        deleted_at: row
            .get::<_, Option<String>>(14)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok()),
        sync_seq: row.get(15).ok().and_then(|v: Option<i64>| v),
    })
}

pub(crate) fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row
            .get::<_, String>(0)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        from_id: row
            .get::<_, String>(1)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        to_id: row
            .get::<_, String>(2)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        relation: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        weight: row.get(4)?,
        created_at: row
            .get::<_, String>(5)?
            .parse()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("{e}")))?,
        created_by: row.get(6).ok().and_then(|v: Option<String>| v),
        deleted_at: row
            .get::<_, Option<String>>(7)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok()),
        sync_seq: row.get(8).ok().and_then(|v: Option<i64>| v),
    })
}
