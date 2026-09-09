use crate::models::{Edge, MemoryKind, Node, NodeType};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::{rank, row_to_edge, row_to_node, search};

/// Одно ребро видно с обоих концов, и на следующем шаге BFS оно приходит
/// вторично — уже со стороны соседа. Без этой отметки связь A→B попадала в
/// ответ дважды: счётчик «N edges» врал, а печать связей показывала близнеца.
type SeenEdges = std::collections::HashSet<uuid::Uuid>;

/// Hard ceiling on the total number of nodes one traversal may return,
/// seeds included. The project node is a hub everything hangs on, so an
/// uncapped BFS at depth 2 used to fan out across the whole database and
/// into other projects (measured 2026-08-30: 2809 nodes, 2.4 MB for one
/// memory_recall call). The cap lives here, not at the call sites, so
/// every caller inherits it and no per-caller default can reintroduce the
/// blow-up.
pub const MAX_TRAVERSAL_NODES: usize = 200;

/// Depth clamp for every traversal. Explicit depths above this are clamped
/// silently; smaller requested depths stay as they are. Three call sites
/// already had to lower their own defaults to 1 to survive hub nodes — the
/// clamp belongs to the walk itself, one defect, one fix.
pub const MAX_TRAVERSAL_DEPTH: u32 = 3;

/// Traversal outcome with the truncation report. Callers that only need
/// the graph use [`context`] / [`context_from_id`]; call sites that answer
/// to a model should surface `hidden_nodes`, so a cut answer can say so
/// instead of posing as the complete picture.
#[derive(Default)]
pub struct Traversal {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Nodes discovered by BFS but dropped because the node budget ran
    /// out. A lower bound: once the budget is gone the walk stops
    /// exploring, so anything further is unmeasured.
    pub hidden_nodes: usize,
    /// 1-based BFS depth at which the budget cut the walk, if it did.
    /// `Some(0)` would mean even the seed set did not fit.
    pub truncated_at_depth: Option<u32>,
}

pub fn context(conn: &Connection, topic: &str, depth: u32) -> Result<(Vec<Node>, Vec<Edge>)> {
    let traversal = context_with_report(conn, topic, depth)?;
    Ok((traversal.nodes, traversal.edges))
}

/// Same walk as [`context`], but keeps the truncation report: how many
/// nodes the cap hid and at which BFS depth the cut happened.
pub fn context_with_report(conn: &Connection, topic: &str, depth: u32) -> Result<Traversal> {
    context_with_report_seeded(conn, topic, depth, DEFAULT_SEEDS)
}

/// Сколько записей FTS берётся посевом по умолчанию.
pub const DEFAULT_SEEDS: usize = 5;

/// Посев для ответа, который потом ранжируется по подграфу. Пяти сидов хватало,
/// пока ответом служили они сами; для ранжирования нужен запас. Измерено
/// 07.09.2026 на теме «ulika»: у всех пяти сидов FTS степень 0 или 1, узел-хаб
/// проекта с 671 ребром в посев не попадал вовсе, и ответ строился из
/// случайных листьев.
pub const RECALL_SEEDS: usize = 12;

/// Сколько записей знания отдаёт recall. Ответ читает модель с ограниченным
/// окном: двенадцать отранжированных записей она использует, сто —
/// пролистывает.
pub const RECALL_LIMIT: usize = 12;

/// Хвост эпизодического. Двух хватает, чтобы ответить «чем занимались
/// последний раз»; больше — это уже журнал, а за ним идут в `au journal`.
pub const RECALL_TAIL_LIMIT: usize = 2;

/// Та же прогулка, что и [`context_with_report`], но с явным размером посева.
pub fn context_with_report_seeded(
    conn: &Connection,
    topic: &str,
    depth: u32,
    seeds: usize,
) -> Result<Traversal> {
    let seeds = search(conn, topic, seeds)?;
    if seeds.is_empty() {
        return Ok(Traversal::default());
    }
    walk(conn, seeds, depth)
}

/// Готовая выдача recall: что показывают читателю и чем ответ признаётся
/// неполным. Счётчики совпадений считаются до срезов — `knowledge.len()`
/// после [`recall_selection`] говорит, сколько показано, а
/// `matched_knowledge` — сколько нашлось.
pub struct RecallSelection {
    /// Знание, порядок ответа. Не длиннее [`RECALL_LIMIT`].
    pub knowledge: Vec<Node>,
    /// Эпизодический хвост. Не длиннее [`RECALL_TAIL_LIMIT`].
    pub recent: Vec<Node>,
    /// Сколько записей знания нашлось до среза.
    pub matched_knowledge: usize,
    /// Сколько эпизодических записей нашлось до среза.
    pub matched_recent: usize,
    /// Размер обхода целиком, до отсева типов: и `Skill`, и `Project`.
    pub total_graph_nodes: usize,
    /// Перенесено из [`Traversal::hidden_nodes`].
    pub hidden_nodes: usize,
    /// Перенесено из [`Traversal::truncated_at_depth`].
    pub truncated_at_depth: Option<u32>,
}

/// Сборка выдачи recall целиком: обход, отсев, раскладка по [`MemoryKind`],
/// порядок и срезы. Всё, что MCP-инструмент `memory_recall` делал у себя
/// внутри.
///
/// Живёт здесь, а не в обработчике, по той же причине, что и
/// [`super::subgraph_degree`]: `memory_recall` лежит в крейте `aurelius`,
/// который зависит от `aurelius-core`, а не наоборот. Пока сборка была внутри
/// обработчика, ни `au recall`, ни прогон `au eval` не могли её позвать и
/// мерили бы копию боевого пути вместо самого пути.
///
/// **Счётчик обращений эта функция не трогает.** `touch_node` остаётся ровно
/// одним вызовом в MCP-обработчике: фикстура прогона открыта только на чтение,
/// и одна запись здесь роняла бы каждый кейс `recall_top5`.
///
/// **Порядок — `rank::score` (T018, `data-model.md` §1), не степень в
/// подграфе.** Подсчёт степени, который раньше жил здесь ради
/// `by_degree_then_recency`, снят вместе с сортировкой: степени в
/// произведении `score` нет, и читать её на этом пути больше некому.
/// `by_degree_then_recency`/[`super::subgraph_degree`] сами не тронуты — у
/// них остаются два потребителя в `au pickup` (`pickup.rs:335,370`), где
/// порядок по степени — заявленное намерение команды, а не унаследованное
/// поведение (**C17**, `contracts/mcp.md` §4 п.16); `au pickup --json` этим
/// изменением не задет.
///
/// `now` — параметр, не `Utc::now()` внутри (FR-030, D1, «момент — параметр
/// на всю глубину», T029/C15): эту функцию зовёт и `au eval` на замороженной
/// фикстуре, и системные часы внутри неё сделали бы прогон невоспроизводимым
/// — тот же довод, что и у `rank::score` (T013).
///
/// Эта функция не отличает узел-посев от узла, пришедшего обходом — обеим
/// группам подставляется один и тот же нейтральный `RankWeights::r_traversed`.
/// Настоящий нормированный bm25 у посевов уже существует внутри
/// `search::search_ranked` (T015), но наружу за пределы `search.rs` он
/// сегодня не отдаётся; прокидывание этого числа сюда — известный пробел вне
/// объёма этой задачи (см. отчёт агента волны T018). Различие между узлами
/// при равном `r` решают оставшиеся четыре множителя — `P`, `R`, `A`, `T`.
pub fn recall_selection(
    conn: &Connection,
    topic: &str,
    depth: u32,
    now: DateTime<Utc>,
) -> Result<RecallSelection> {
    let Traversal {
        nodes: context_nodes,
        hidden_nodes,
        truncated_at_depth,
        ..
    } = context_with_report_seeded(conn, topic, depth, RECALL_SEEDS)?;
    let total_graph_nodes = context_nodes.len();

    let mut knowledge = vec![];
    let mut recent = vec![];

    for node in context_nodes {
        // Карточки навыков приходят на SessionStart через `au skills --hook` и
        // в выдаче recall были бы вторым экземпляром того же текста.
        if matches!(node.node_type, NodeType::Skill) {
            continue;
        }
        // Узел проекта — навигация, а не знание: у него нет ни claim, ни note,
        // метка равна имени проекта.
        if matches!(node.node_type, NodeType::Project) {
            continue;
        }
        // Эпизодическое — снимок момента: сессия, срез перед компакцией. Оно
        // отвечает на «что происходило», а спрашивают «что известно», поэтому
        // уходит в хвост, а не смешивается со знанием.
        if matches!(node.memory_kind, MemoryKind::Episodic) {
            recent.push(node);
        } else {
            knowledge.push(node);
        }
    }

    let weights = rank::RankWeights::default();
    let mut knowledge = sort_by_score(knowledge, &weights, now);
    let mut recent = sort_by_score(recent, &weights, now);

    let matched_knowledge = knowledge.len();
    let matched_recent = recent.len();
    knowledge.truncate(RECALL_LIMIT);
    recent.truncate(RECALL_TAIL_LIMIT);

    Ok(RecallSelection {
        knowledge,
        recent,
        matched_knowledge,
        matched_recent,
        total_graph_nodes,
        hidden_nodes,
        truncated_at_depth,
    })
}

/// Сортировка одной группы (`knowledge` либо `episodic_tail`) по
/// `rank::score`, невозрастающе. Общая точка для обеих групп —
/// `recall_selection` не заводит второго компаратора: обе зовут ровно эту
/// функцию, которая сама зовёт ровно `rank::score`.
fn sort_by_score(nodes: Vec<Node>, weights: &rank::RankWeights, now: DateTime<Utc>) -> Vec<Node> {
    let mut scored: Vec<(f64, Node)> = nodes
        .into_iter()
        .map(|node| {
            let s = rank::score(weights, &node, weights.r_traversed, now);
            (s, node)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, node)| node).collect()
}

/// BFS traversal from a specific node ID (no FTS search — starts from a known node).
pub fn context_from_id(
    conn: &Connection,
    node_id: &str,
    depth: u32,
) -> Result<(Vec<Node>, Vec<Edge>)> {
    let traversal = context_from_id_with_report(conn, node_id, depth)?;
    Ok((traversal.nodes, traversal.edges))
}

/// Same walk as [`context_from_id`] with the truncation report attached.
pub fn context_from_id_with_report(
    conn: &Connection,
    node_id: &str,
    depth: u32,
) -> Result<Traversal> {
    let seed = super::crud::get_node(conn, node_id)?;
    match seed {
        Some(n) => walk(conn, vec![n], depth),
        None => Ok(Traversal::default()),
    }
}

/// BFS shared by every entry point. Depth is clamped to
/// [`MAX_TRAVERSAL_DEPTH`]; the node budget stops the walk mid-level and
/// every discovered-but-dropped node lands in `hidden_nodes`.
fn walk(conn: &Connection, seeds: Vec<Node>, depth: u32) -> Result<Traversal> {
    let depth = depth.min(MAX_TRAVERSAL_DEPTH);
    let mut visited_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_edges = SeenEdges::new();
    let mut out = Traversal::default();
    let mut queue: Vec<String> = vec![];
    for node in seeds {
        if !visited_nodes.insert(node.id.to_string()) {
            continue;
        }
        if out.nodes.len() < MAX_TRAVERSAL_NODES {
            queue.push(node.id.to_string());
            out.nodes.push(node);
        } else {
            out.hidden_nodes += 1;
            out.truncated_at_depth.get_or_insert(0);
        }
    }
    for level in 0..depth {
        if queue.is_empty() {
            break;
        }
        if out.nodes.len() >= MAX_TRAVERSAL_NODES {
            // The budget ran out before this level: deeper neighborhoods
            // are not explored at all, so nothing can be counted there.
            out.truncated_at_depth.get_or_insert(level + 1);
            break;
        }
        let edges = get_edges_batch(conn, &queue)?;
        let mut neighbor_ids = vec![];
        for edge in edges {
            let neighbor_id = if queue.contains(&edge.from_id.to_string()) {
                edge.to_id.to_string()
            } else {
                edge.from_id.to_string()
            };
            if !visited_nodes.contains(&neighbor_id) {
                visited_nodes.insert(neighbor_id.clone());
                neighbor_ids.push(neighbor_id);
            }
            if seen_edges.insert(edge.id) {
                out.edges.push(edge);
            }
        }
        let neighbors = get_nodes_batch(conn, &neighbor_ids)?;
        let mut next_queue: Vec<String> = vec![];
        for node in neighbors {
            if out.nodes.len() < MAX_TRAVERSAL_NODES {
                next_queue.push(node.id.to_string());
                out.nodes.push(node);
            } else {
                out.hidden_nodes += 1;
                out.truncated_at_depth.get_or_insert(level + 1);
            }
        }
        queue = next_queue;
    }
    Ok(out)
}

fn get_edges_batch(conn: &Connection, node_ids: &[String]) -> Result<Vec<Edge>> {
    if node_ids.is_empty() {
        return Ok(vec![]);
    }
    let n = node_ids.len();
    let ph1: Vec<String> = (1..=n).map(|i| format!("?{i}")).collect();
    let ph2: Vec<String> = (n + 1..=2 * n).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq
         FROM edges
         WHERE (from_id IN ({}) OR to_id IN ({})) AND deleted_at IS NULL",
        ph1.join(","),
        ph2.join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for id in node_ids {
        param_values.push(Box::new(id.clone()));
    }
    for id in node_ids {
        param_values.push(Box::new(id.clone()));
    }
    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let edges = stmt
        .query_map(params.as_slice(), row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

fn get_nodes_batch(conn: &Connection, ids: &[String]) -> Result<Vec<Node>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE id IN ({}) AND deleted_at IS NULL",
        placeholders.join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let nodes = stmt
        .query_map(params.as_slice(), row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NodeType, Relation};

    /// Ребро видно с обоих концов: на втором шаге BFS оно приходит со стороны
    /// соседа. Пока обход не помечал увиденное, `au context` печатал одну связь
    /// дважды, а MCP отдавал её дважды в JSON.
    #[test]
    fn traversal_returns_each_edge_once() {
        let path =
            std::env::temp_dir().join(format!("aurelius-traverse-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).expect("open temp db");

        let a = super::super::add_node(
            &conn,
            NodeType::Concept,
            "узел A",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("node a");
        let b = super::super::add_node(
            &conn,
            NodeType::Concept,
            "узел B",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("node b");
        super::super::add_edge(&conn, a.id, b.id, Relation::RelatedTo, 1.0).expect("edge");

        let (_, edges) = context_from_id(&conn, &a.id.to_string(), 3).expect("traverse");
        assert_eq!(edges.len(), 1, "одно ребро — одна запись в ответе");

        drop(conn);
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(p));
        }
    }

    /// Уборка за тестом: файл базы вместе с WAL и SHM. `:memory:` здесь не
    /// годится — `db::open` жёстко требует WAL.
    struct TmpDb(std::path::PathBuf);

    impl Drop for TmpDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.as_os_str().to_owned();
                p.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(p));
            }
        }
    }

    /// Кто есть кто в графе под [`recall_selection`], после перехода на
    /// `rank::score` (T018): расстановка больше не через степень в подграфе,
    /// а через провенанс (`P`) и тип узла (`T`) — `r` у всех узлов фикстуры
    /// одинаков (`RankWeights::r_traversed`, см. doc-комментарий
    /// [`recall_selection`]), так что порядок решают только они.
    struct RecallFixture {
        /// `measured` + `Decision` — наибольший `P · T` — обязан стоять
        /// первым в знании.
        anchor: uuid::Uuid,
        /// `reported` + `Decision` — `P` пониже, `T` тот же — обязан стоять
        /// вторым.
        second: uuid::Uuid,
        skill: uuid::Uuid,
        project: uuid::Uuid,
        /// Единственный `measured` среди эпизодических — первый в хвосте.
        ep_top: uuid::Uuid,
    }

    /// Граф построен так, чтобы порядок по `score` **расходился** с порядком
    /// обхода: `anchor`, `second` и `ep_top` слова `квазар` не несут, в посев
    /// не попадают и приходят уже вторым шагом BFS — то есть в конце
    /// `traversal.nodes`, после самих посевов. Без сортировки первыми
    /// окажутся сеятельные узлы (`seed_a`/`seed_b`, `Concept` без
    /// `confidence` — заведомо ниже `anchor` и `second` по `score`), и
    /// проверка порядка провалится; на фикстуре, где порядок обхода совпадает
    /// с порядком `score`, снятая сортировка тестом не ловится (проверено
    /// удалением `sort_by_score`).
    ///
    /// `access_count` и `created_at` у всех узлов фикстуры равны (вставлены
    /// подряд в одном тесте, обращений не было) — разрыв по ним в
    /// утверждения не входит, различает только `P` и `T`.
    fn recall_fixture(tag: &str) -> (TmpDb, Connection, RecallFixture) {
        let tmp = TmpDb(
            std::env::temp_dir().join(format!("aurelius-recall-{tag}-{}.db", uuid::Uuid::new_v4())),
        );
        let conn = crate::db::open(&tmp.0).expect("open temp db");

        let add = |node_type: NodeType, label: &str, kind: MemoryKind, data: serde_json::Value| {
            super::super::add_node_full(&conn, node_type, label, None, "test", data, kind, None)
                .expect("node")
                .id
        };
        let link = |from: uuid::Uuid, to: uuid::Uuid| {
            super::super::add_edge(&conn, from, to, Relation::RelatedTo, 1.0).expect("edge");
        };
        let no_confidence = || serde_json::json!({});
        let measured = || serde_json::json!({"confidence": "measured"});
        let reported = || serde_json::json!({"confidence": "reported"});

        // Посев: только эти узлы находит FTS по теме. Без `confidence` — `P`
        // ниже, чем у `anchor`/`second` ниже.
        let seed_a = add(
            NodeType::Concept,
            "квазар первый",
            MemoryKind::Semantic,
            no_confidence(),
        );
        let seed_b = add(
            NodeType::Concept,
            "квазар второй",
            MemoryKind::Semantic,
            no_confidence(),
        );
        let skill = add(
            NodeType::Skill,
            "квазар карточка",
            MemoryKind::Semantic,
            no_confidence(),
        );
        let project = add(
            NodeType::Project,
            "квазар",
            MemoryKind::Semantic,
            no_confidence(),
        );
        let ep_a = add(
            NodeType::Session,
            "квазар сессия A",
            MemoryKind::Episodic,
            no_confidence(),
        );
        let ep_b = add(
            NodeType::Session,
            "квазар сессия B",
            MemoryKind::Episodic,
            no_confidence(),
        );

        // Первый шаг BFS: до них тема не дотягивается, дотягиваются рёбра.
        // `Decision`+`measured` (P=1.0, T=1.0) против `Decision`+`reported`
        // (P=0.8, T=1.0) — выше `score` при равном `r`, а не выше степени.
        let anchor = add(
            NodeType::Decision,
            "лист опора",
            MemoryKind::Semantic,
            measured(),
        );
        let second = add(
            NodeType::Decision,
            "лист связка",
            MemoryKind::Semantic,
            reported(),
        );
        for node in [seed_a, seed_b, skill, project, ep_a, ep_b, second] {
            link(anchor, node);
        }
        link(second, seed_a);
        link(second, seed_b);

        // Второй шаг: знания больше среза на четыре, `matched_knowledge`
        // обязан считать до него, а не после. `Concept` без `confidence`
        // (P=0.7, T=0.95, произведение 0.665) — заведомо ниже и `anchor`
        // (1.0), и `second` (0.8).
        for i in 0..RECALL_LIMIT {
            let leaf = add(
                NodeType::Concept,
                &format!("лист {i}"),
                MemoryKind::Semantic,
                no_confidence(),
            );
            link(anchor, leaf);
        }

        // Эпизодических на одну больше хвоста — по той же причине. `ep_top`
        // — единственный `measured` среди эпизодических, `ep_a`/`ep_b` без
        // `confidence`.
        let ep_top = add(
            NodeType::Session,
            "лист сессия",
            MemoryKind::Episodic,
            measured(),
        );
        link(anchor, ep_top);
        link(second, ep_top);

        (
            tmp,
            conn,
            RecallFixture {
                anchor,
                second,
                skill,
                project,
                ep_top,
            },
        )
    }

    /// Сборка, переехавшая из MCP-обработчика: карточки навыков и узел
    /// проекта выброшены, эпизодическое ушло в хвост, порядок — `rank::score`
    /// (T018), срезы стоят на 12 и 2, а счётчики совпадений считаются до
    /// срезов.
    #[test]
    fn recall_selection_filters_splits_orders_and_slices() {
        let (_tmp, conn, f) = recall_fixture("assembly");

        let out = recall_selection(&conn, "квазар", 2, chrono::Utc::now()).expect("recall");

        let ids: Vec<uuid::Uuid> = out.knowledge.iter().map(|n| n.id).collect();
        assert_eq!(
            ids.first(),
            Some(&f.anchor),
            "первым идёт узел с наибольшим score (measured Decision), а не первый найденный обходом"
        );
        assert_eq!(
            ids.get(1),
            Some(&f.second),
            "второй по score (reported Decision) — вторая строка"
        );
        assert_eq!(out.knowledge.len(), RECALL_LIMIT, "срез знания — 12");
        assert_eq!(
            out.matched_knowledge,
            RECALL_LIMIT + 4,
            "совпадения считаются до среза: двенадцать листьев, два посева, опора и связка"
        );

        assert!(
            !ids.contains(&f.skill) && !ids.contains(&f.project),
            "карточка навыка и узел проекта не показываются"
        );

        let tail: Vec<uuid::Uuid> = out.recent.iter().map(|n| n.id).collect();
        assert_eq!(
            tail.first(),
            Some(&f.ep_top),
            "эпизодическое ранжируется тем же score: единственный measured — первый"
        );
        assert_eq!(out.recent.len(), RECALL_TAIL_LIMIT, "срез хвоста — 2");
        assert_eq!(out.matched_recent, 3, "и здесь счётчик до среза");

        assert_eq!(
            out.total_graph_nodes,
            RECALL_LIMIT + 4 + 3 + 2,
            "размер обхода — до отсева типов: знание, эпизодическое, навык и проект"
        );
    }

    /// Общая функция ядра не пишет в базу ни строки. Фикстура прогона
    /// `au eval` открывается только на чтение, и один `touch_node` здесь
    /// ронял бы каждый кейс `recall_top5`; инкремент `access_count` живёт
    /// ровно в одном месте — в MCP-обработчике `memory_recall`.
    #[test]
    fn recall_selection_never_touches_access_count() {
        let (_tmp, conn, _f) = recall_fixture("readonly");

        let out = recall_selection(&conn, "квазар", 2, chrono::Utc::now()).expect("recall");
        assert!(
            !out.knowledge.is_empty(),
            "выдача не пуста — есть что портить"
        );

        let touched: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE access_count != 0",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(touched, 0, "ни одного инкремента на общем пути");
    }
}
