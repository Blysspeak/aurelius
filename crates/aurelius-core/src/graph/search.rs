use crate::models::{Node, NodeType};
use anyhow::Result;
use rusqlite::{params, Connection};

use super::row_to_node;

// T018 подключает `graph::rank` (`RankWeights`, data-model.md §1) к пути
// посевов: нормировка bm25 теперь зовёт `rank::normalize_bm25` — приватная
// копия этой формулы здесь удалена (замечено при приёмке 08.09.2026, две
// копии одной формулы расходятся молча при первой же калибровке `au eval`).
// Числа ниже по-прежнему дублируют `RankWeights::df_veto_ratio` /
// `min_significant_terms`; их унификация в объём этой правки не входит.

/// Результат поиска вместе с ответом на вопрос «почему не нашлось».
///
/// Без `unmatched_terms` инструмент не отличал «знания нет» от «запрос не
/// сработал»: пустая выдача выглядела одинаково и когда факта в памяти нет, и
/// когда слово написано в форме, которой нет в индексе («алертов» против
/// «алерт»). Первое означает «иди и выясняй», второе — «спроси иначе», и цена
/// ошибки между ними — целая ветка ненужной разведки.
pub struct SearchOutcome {
    pub nodes: Vec<Node>,
    /// Все слова запроса, как их разобрал [`crate::fts::parse`].
    pub terms: Vec<String>,
    /// Слова запроса, не встретившиеся в индексе ни разу.
    pub unmatched_terms: Vec<String>,
}

impl SearchOutcome {
    /// Человеческое объяснение пустоты — или `None`, если объяснять нечего.
    ///
    /// Диагноз говорит ровно то, что знает: про строку поиска, а не про мир.
    /// «Знания здесь просто нет» — утверждение о мире, которого у поиска на
    /// руках нет: «~20 воркеров» лежало в трёх узлах, не совпала одна лишь
    /// словоформа. Такое ложное успокоение дороже молчания — читатель идёт
    /// делать заново уже сделанное.
    #[must_use]
    pub fn diagnosis(&self) -> Option<String> {
        if self.unmatched_terms.is_empty() {
            return None;
        }
        let words = self
            .unmatched_terms
            .iter()
            .map(|w| format!("«{w}»"))
            .collect::<Vec<_>>()
            .join(", ");
        let advice = match self.unmatched_terms.first().and_then(|w| prefix_hint(w)) {
            Some(stem) => format!("попробуй другую или короче ({stem}*)"),
            None => "попробуй другую форму".to_string(),
        };
        if self.unmatched_terms.len() == self.terms.len() {
            return Some(format!(
                "ни одно слово запроса не встречается в памяти в этой форме ({words}) — \
                 возможно, дело в словоформе: {advice}. \
                 Поиск не утверждает, что знания нет, — только что совпадений по этой строке нет"
            ));
        }
        Some(format!(
            "{words} не встречается в памяти в этой форме — возможно, дело в словоформе: {advice}"
        ))
    }
}

/// Короче какого корня префикс перестаёт что-либо сужать.
const STEM_FLOOR: usize = 4;

/// Префикс, который стоит предложить вместо промахнувшегося слова.
///
/// Обрубается по букве, а не по фиксированной длине: `take(5)` на
/// пятибуквенном «батчи» возвращал то же слово со звёздочкой — совет,
/// заведомо не работавший ровно там, где нужен больше всего, ведь у коротких
/// слов окончание и есть вся разница. Снимается одна буква: «батчи» → `батч*`,
/// «воркеры» → `воркер*`; две сломали бы второе до «ворке».
///
/// `None` — когда слово и так не длиннее корня: звёздочка на нём ничего не
/// расширит, а несбыточный совет хуже его отсутствия.
fn prefix_hint(word: &str) -> Option<String> {
    let letters = word.chars().count();
    if letters <= STEM_FLOOR {
        return None;
    }
    Some(word.chars().take(letters - 1).collect())
}

/// Во сколько раз брать больше, чем просили, прежде чем ранжировать.
///
/// `OR` находит и то, где совпало одно слово из трёх; отсортировать это по
/// нормированному bm25 можно только имея запас — иначе `LIMIT` отрежет лучшее
/// ещё до ранжирования. Расширяет пул кандидатов, а не переставляет их
/// (T015, `data-model.md` §1) — остаётся неизменным с фазы, где посевы ещё
/// пересортировывались по числу совпавших слов.
const OVERFETCH: usize = 5;

/// Доля живых узлов, после которой терм считается «слишком широким» и
/// перестаёт сужать тему (FR-006). Дублирует `RankWeights::df_veto_ratio` —
/// см. комментарий у объявления `use` в начале файла.
const DF_VETO_RATIO: f64 = 0.01;

/// Меньше скольких термов с df ниже вето делает тему `topic_too_narrow`
/// (FR-006). Дублирует `RankWeights::min_significant_terms` — см. комментарий
/// у объявления `use` в начале файла.
const MIN_SIGNIFICANT_TERMS: usize = 2;

pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Node>> {
    Ok(search_ranked(conn, query, limit)?.nodes)
}

/// Поиск с нормированным bm25 и с диагностикой запроса.
///
/// Слова соединены через `OR` (см. [`crate::fts`]); овербор (`OVERFETCH`)
/// расширяет пул кандидатов, а порядок внутри него — ровно нормированный
/// bm25 (`r = |rank| / (|rank| + median|rank|)`, FR-007a/FR-008,
/// `data-model.md` §1) и ничего больше: пересортировка по числу совпавших
/// слов (`rank_by_matched_terms`, веса `W_LABEL`/`W_CLAIM`/`W_NOTE`) с этого
/// пути снята — она была вторым, снаружи невидимым сигналом ранжирования и
/// решала состав выдачи, а не только порядок внутри неё. `search_typed` эту
/// пересортировку по-прежнему использует и этой правкой не затронут.
///
/// Координаты секретов (`secret::is_secret_ref`) отфильтрованы на всех трёх
/// путях выдачи (находка 11, FR-027): узел-координата — обычный `Config`,
/// который FTS индексирует наравне со всеми (label+note+data, см. `db.rs`),
/// и без фильтра здесь он утекал в любой запрос, задевший его метку,
/// назначение или место хранения. Явный `secret_list` идёт другим путём
/// (`typed_in_project` внутри `list_secret_refs`) и фильтр не задевает.
pub fn search_ranked(conn: &Connection, query: &str, limit: usize) -> Result<SearchOutcome> {
    Ok(search_scored(conn, query, limit)?.0)
}

/// Лента последних узлов вместо поиска (пустой запрос или `*`). Скоров у
/// неё нет — порядок задаёт время, и убывающий номер позиции держит его
/// без единой ничьей, чтобы [`tidy_for_emission`] ничего в ней не двигал.
fn recent_scored(conn: &Connection, limit: usize) -> Result<(SearchOutcome, Vec<f64>)> {
    // Запас на отсев: прогоны пишутся каждым verify и иначе заняли бы ленту.
    let mut nodes = get_recent_nodes(conn, limit * OVERFETCH)?;
    nodes.retain(|n| !crate::secret::is_secret_ref(n) && !is_technical(n));
    nodes.truncate(limit);
    let scores = (0..nodes.len()).map(|i| -(i as f64)).collect();
    Ok((
        SearchOutcome {
            nodes,
            terms: Vec::new(),
            unmatched_terms: Vec::new(),
        },
        scores,
    ))
}

/// [`search_ranked`] вместе с нормированным bm25 каждого узла (параллельно
/// `nodes`) — для выдачи, которой нужны ничьи ([`tidy_for_emission`]).
/// Скоры идут вторым значением, а не полем `SearchOutcome`: ту литералом
/// собирает MCP-крейт (`handlers/crud.rs`), и новое поле сломало бы его.
///
/// # Errors
/// Ошибка запроса к FTS-таблице или к `nodes`.
pub fn search_scored(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<(SearchOutcome, Vec<f64>)> {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return recent_scored(conn, limit);
    }
    // Строка от человека — текст, а не выражение FTS5 (см. crate::fts).
    let parsed = crate::fts::parse(trimmed);
    if parsed.expr.is_empty() {
        return recent_scored(conn, limit);
    }
    // T015: `rank` (bm25) выходит наружу вместе с узлом — раньше он жил
    // только внутри SQL (в `ORDER BY`), в SELECT-лист не входил и наружу не
    // выходил вовсе, нормировать было нечего.
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash,
                n.created_by, n.updated_by, n.deleted_at, n.sync_seq, rank
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1 AND n.deleted_at IS NULL
         ORDER BY rank
         LIMIT ?2",
    )?;
    let mut fetched = stmt
        .query_map(params![parsed.expr, (limit * OVERFETCH) as i64], |row| {
            let node = row_to_node(row)?;
            let raw_rank: f64 = row.get(16)?;
            Ok((node, raw_rank))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    fetched.retain(|(n, _)| !crate::secret::is_secret_ref(n) && !is_technical(n));

    // Посевы упорядочиваются нормированным bm25 и ничем иным (FR-008): ни
    // числа обращений (снято из `ORDER BY` — множитель `A` в `rank::score`
    // заменяет его, а не складывается с ним, T018), ни веса полей
    // (`rank_by_matched_terms`, снят с этого пути выше).
    let raw_ranks: Vec<f64> = fetched.iter().map(|(_, r)| *r).collect();
    let normalized = super::rank::normalize_bm25(&raw_ranks);
    let mut scored: Vec<(Node, f64)> = fetched
        .into_iter()
        .zip(normalized)
        .map(|((node, _), r)| (node, r))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(limit);
    let (nodes, scores): (Vec<Node>, Vec<f64>) = scored.into_iter().unzip();

    Ok((
        SearchOutcome {
            nodes,
            unmatched_terms: unmatched_terms(conn, &parsed.terms)?,
            terms: parsed.terms,
        },
        scores,
    ))
}

/// Гигиена выдачи поверх уже посчитанного порядка: скоры не трогает, а
/// значит, не меняет и того, что считается релевантным — только сколько
/// строк уходит наружу и в каком порядке при ничьей.
///
/// - Выкидывает строку, которую читатель увидел бы дважды: тот же тип, та
///   же метка И то же тело (`note`); остаётся первая, то есть старшая по
///   скору. Тело входит в ключ нарочно: одна метка при разных телах — это
///   разное знание (в живом графе 19.09.2026 таких групп 281 из 550 групп с
///   общей меткой, например 15 `work_log` с одной меткой и 15 разными
///   телами), и склейка по одной метке выбросила бы его.
/// - При равном скоре пустые записи (ни `claim`, ни тела — например, узел
///   зависимости `rust-embed` от индексатора) идут после содержательных.
/// - Узел прогона (`run`) — бухгалтерия улик, а не знание: в выдачу идёт
///   самое большее один, и только на место, которое знанию нечем занять.
///   Каждый `verify-run` пишет такой узел, и они вытесняли знание из выдачи
///   тех самых слов, которыми их проверяли (19.09.2026: два `["run"] прогон:
///   …` из пяти строк по запросу «embed socket bge-m3», 396 узлов в графе).
/// - Оставляет первые `limit`.
///
/// `scored` — узлы со скором, по убыванию скора.
#[must_use]
pub fn tidy_for_emission(scored: Vec<(Node, f64)>, limit: usize) -> Vec<Node> {
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<(Node, f64)> = scored
        .into_iter()
        .filter(|(n, _)| {
            let node_type = serde_json::to_string(&n.node_type).unwrap_or_default();
            seen.insert((node_type, n.label.clone(), n.note.clone()))
        })
        .collect();
    // Устойчивая сортировка: вне ничьих порядок входа не меняется.
    kept.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| is_bare(&a.0).cmp(&is_bare(&b.0)))
    });
    let (runs, mut knowledge): (Vec<_>, Vec<_>) = kept.into_iter().partition(|(n, _)| is_run(n));
    knowledge.truncate(limit);
    if knowledge.len() < limit {
        knowledge.extend(runs.into_iter().take(1));
    }
    knowledge.into_iter().map(|(n, _)| n).collect()
}

/// Узел прогона — и нынешний `NodeType::Run`, и старая форма
/// `Custom("run")`, которая ещё лежит в живом графе (тот же разбор, что у
/// `rank::type_weight`).
#[must_use]
pub fn is_run(node: &Node) -> bool {
    match &node.node_type {
        NodeType::Run => true,
        NodeType::Custom(s) => s == "run",
        _ => false,
    }
}

/// Запись без содержания: ни утверждения, ни тела — одна метка.
pub(crate) fn is_bare(node: &Node) -> bool {
    let blank = |s: Option<&str>| s.is_none_or(|s| s.trim().is_empty());
    let claim = crate::provenance::Provenance::from_data(&node.data).claim;
    blank(claim.as_deref()) && blank(node.note.as_deref())
}

/// Бухгалтерия, а не знание: узел прогона и зависимость без содержания (ни
/// `claim`, ни тела — то, что пишет индексатор: манифест в `data`, сама
/// строка манифеста). В выдачу поиска и в векторы такие не идут — на
/// 19.09.2026 по запросу «embed socket bge-m3» MCP отдавал два прогона и два
/// `rust-embed` из двадцати строк. Зависимость с телом — записанное человеком
/// знание («почему закреплена версия»), она остаётся.
///
/// Явный поиск по типу (`search_typed` с `run` или `dependency`) их находит:
/// там это не шум, а сам вопрос.
#[must_use]
pub fn is_technical(node: &Node) -> bool {
    is_run(node) || (matches!(node.node_type, NodeType::Dependency) && is_bare(node))
}

/// [`is_technical`] в SQL для узла под псевдонимом `n` — там, где узлы не
/// читаются в память. Обе формы сверяет тест
/// `technical_sql_agrees_with_is_technical`.
pub const TECHNICAL_NODE_SQL: &str = "(n.node_type IN ('\"run\"', '{\"custom\":\"run\"}') \
     OR (n.node_type = '\"dependency\"' \
         AND TRIM(COALESCE(json_extract(n.data, '$.claim'), '')) = '' \
         AND TRIM(COALESCE(n.note, '')) = ''))";

/// Убрать из очереди векторов технические узлы ([`is_technical`]) — до
/// того, как демон потратит на них модель. Очередь пополняют три двери
/// (запись узла и оба `reindex-embeddings`, CLI и MCP), разбирает одна —
/// поэтому отсев здесь, а не у каждой двери.
///
/// # Errors
/// Ошибка запроса к `embedding_queue` или `nodes`.
pub fn drop_technical_from_queue(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            "DELETE FROM embedding_queue WHERE node_id IN (
                SELECT n.id FROM embedding_queue q JOIN nodes n ON n.id = q.node_id
                 WHERE {TECHNICAL_NODE_SQL})"
        ),
        [],
    )?)
}

/// Вес совпадения по полям. Заголовок — то, что автор счёл сутью записи;
/// `claim` — утверждение в одну строку; `note` — всё остальное, включая
/// простыни сессионных итогов.
///
/// Без этих весов bm25 считает по всему документу, и длинный `note`, где нужные
/// слова рассыпаны по тексту, перевешивал заголовок, состоящий из них почти
/// целиком: запись «Горячая перезагрузка флагов доведена до алертов и разбита
/// на модули» не попадала в топ-3 по запросу из тех же слов.
const W_LABEL: usize = 3;
const W_CLAIM: usize = 2;
const W_NOTE: usize = 1;

/// Взвешенное совпадение: за каждое слово берётся вес самого «сильного» поля,
/// в котором оно нашлось.
fn matched_score(node: &Node, terms: &[String]) -> usize {
    let label = node.label.to_lowercase();
    let claim = crate::provenance::Provenance::from_data(&node.data)
        .claim
        .map(|c| c.to_lowercase());
    let note = node.note.as_ref().map(|n| n.to_lowercase());

    terms
        .iter()
        .map(|t| {
            let t = t.to_lowercase();
            if label.contains(&t) {
                W_LABEL
            } else if claim.as_ref().is_some_and(|c| c.contains(&t)) {
                W_CLAIM
            } else if note.as_ref().is_some_and(|n| n.contains(&t)) {
                W_NOTE
            } else {
                0
            }
        })
        .sum()
}

/// Выше тот, у кого совпало больше и в более значимом поле. Сортировка
/// устойчивая, поэтому внутри одной группы сохраняется порядок bm25 из SQL.
fn rank_by_matched_terms(nodes: &mut [Node], terms: &[String]) {
    if terms.is_empty() {
        return;
    }
    nodes.sort_by_key(|n| std::cmp::Reverse(matched_score(n, terms)));
}

/// Слова запроса и те из них, что не встретились в индексе ни разу.
///
/// Отдельная функция, потому что диагностика нужна и там, где выдача собрана
/// не `search_ranked`: у поиска с фильтром по типу вопрос «почему пусто» тот же.
///
/// # Errors
/// Ошибка подготовки или исполнения запроса к FTS-таблице.
pub fn query_terms(conn: &Connection, query: &str) -> Result<(Vec<String>, Vec<String>)> {
    let terms = crate::fts::parse(query).terms;
    let unmatched = unmatched_terms(conn, &terms)?;
    Ok((terms, unmatched))
}

/// Спрашивается по одному слову: только так видно, какое именно слово увело
/// запрос в пустоту.
fn unmatched_terms(conn: &Connection, terms: &[String]) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT 1 FROM nodes_fts JOIN nodes n ON nodes_fts.rowid = n.rowid
          WHERE nodes_fts MATCH ?1 AND n.deleted_at IS NULL LIMIT 1",
    )?;
    let mut out = Vec::new();
    for term in terms {
        let found = stmt
            .query_map(params![crate::fts::term_expr(term)], |_| Ok(()))?
            .next()
            .is_some();
        if !found {
            out.push(term.clone());
        }
    }
    Ok(out)
}

/// Повод, по которому [`refuse_topic`] остановил recall до обхода графа
/// (FR-006, `contracts/mcp.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// В теме есть терм, встречающийся более чем в `DF_VETO_RATIO` живых узлов.
    TopicTooBroad,
    /// После отсева термов выше вето значимых термов осталось меньше
    /// `MIN_SIGNIFICANT_TERMS`.
    TopicTooNarrow,
}

/// df одного терма темы: доля живых узлов, которые он матчит, и абсолютное
/// число узлов — то же число, что несёт `Refusal::matched` для термов выше
/// вето.
#[derive(Debug, Clone)]
pub struct TermDf {
    pub term: String,
    pub df: f64,
    pub nodes: i64,
}

/// Отказ recall на слишком широкую или слишком узкую тему (FR-006).
///
/// Решение принимается ДО обхода графа, поэтому `matched` — число узлов
/// индекса по терму, а не `matched_knowledge` после BFS: путать их нельзя,
/// они меряют разное.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub reason: RefusalReason,
    pub matched: i64,
    /// Термы выше вето (`TopicTooBroad`) или значимые термы темы
    /// (`TopicTooNarrow`) — в зависимости от `reason`.
    pub terms: Vec<TermDf>,
    pub advice: String,
}

/// df терма — тот же запрос, что уже стоит в [`unmatched_terms`], но
/// `COUNT(*)` вместо `SELECT 1` и без `LIMIT`: вето стоит на числе узлов, а
/// не на факте хотя бы одного совпадения.
fn term_document_count(conn: &Connection, term: &str) -> Result<i64> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM nodes_fts JOIN nodes n ON nodes_fts.rowid = n.rowid
          WHERE nodes_fts MATCH ?1 AND n.deleted_at IS NULL",
    )?;
    let count: i64 = stmt.query_row(params![crate::fts::term_expr(term)], |row| row.get(0))?;
    Ok(count)
}

/// Живых узлов всего — знаменатель df.
fn live_node_count(conn: &Connection) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE deleted_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// «3,5 %» — одна цифра после запятой, запятая вместо точки: формат совета
/// из `contracts/cli.md` §4.
fn format_pct(df: f64) -> String {
    format!("{:.1}", df * 100.0).replace('.', ",")
}

/// Терм совпал с меткой живого узла типа `project` (Edge Case `guard`,
/// spec.md: терм одновременно стоп-слово базы и имя сущности). Регистр не
/// учитывается: `fts`-термы приходят строчными, метка проекта — как её ввели.
fn matches_project_label(conn: &Connection, term: &str) -> Result<Option<String>> {
    let type_str = serde_json::to_string(&NodeType::Project)?;
    let mut stmt = conn.prepare(
        "SELECT label FROM nodes
          WHERE node_type = ?1 AND LOWER(label) = LOWER(?2) AND deleted_at IS NULL LIMIT 1",
    )?;
    let label = stmt
        .query_map(params![type_str, term], |row| row.get::<_, String>(0))?
        .next()
        .transpose()?;
    Ok(label)
}

/// Кандидаты для замены широкого терма: пары значимых термов, не менее двух
/// в каждой (T022 — предложение из одного терма само упёрлось бы в
/// `topic_too_narrow` того же детектора, из которого пришло). Нечётный
/// остаток без пары отбрасывается `chunks_exact`, а не оставляется
/// одиночкой.
fn suggestion_pairs(narrow: &[TermDf]) -> Vec<(String, String)> {
    narrow
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0].term.clone(), pair[1].term.clone()))
        .collect()
}

/// Совет на `topic_too_broad`. Каноническая форма — `contracts/cli.md` §4
/// («stdout: отказ по ширине темы»); неизменная часть (первое предложение,
/// объяснение OR, разбор на пары, хвост области) взята оттуда дословно, а не
/// сочинена заново. Меняются только сам широкий терм, его доля и то, что
/// нашлось ниже вето у этой темы.
///
/// Источник пар — только оставшиеся термы САМОЙ темы (`narrow`), а не
/// словарь базы целиком: у темы из одного терма (как «au» в нормативном
/// примере) ниже вето не остаётся ничего, и совет прямо говорит это и
/// предлагает только `--project` — та же форма, что и у Edge Case `guard`
/// («Сузить здесь можно только область», spec.md). Иллюстративные пары
/// канонического примера («стемминг recognize», «judge фикстура») в код не
/// перенесены: их df на живой базе не снят (сам документ это оговаривает),
/// а источник, из которого их можно было бы получить для темы без
/// собственных оставшихся термов, нигде в пакете не описан — см. отчёт
/// агента к T016.
fn broad_advice(
    conn: &Connection,
    worst: &TermDf,
    narrow: &[TermDf],
    project: Option<&str>,
) -> Result<String> {
    let mut advice = format!(
        "тема слишком широкая: терм «{}» встречается в {} % узлов и тему не сужает. \
         Второе слово рядом не поможет — термы склеиваются через OR",
        worst.term,
        format_pct(worst.df)
    );
    match narrow.first() {
        Some(second) => advice.push_str(&format!(
            ", поэтому «{} {}» шире, чем «{}».",
            worst.term, second.term, second.term
        )),
        None => advice.push('.'),
    }

    let pairs = suggestion_pairs(narrow);
    if pairs.is_empty() {
        advice.push_str(" Ниже вето слов не осталось — сузьте область");
    } else {
        let joined = pairs
            .iter()
            .map(|(a, b)| format!("«{a} {b}»"))
            .collect::<Vec<_>>()
            .join(", ");
        advice.push_str(&format!(
            " Замените широкий терм парой узких: {joined}. Либо сузьте область"
        ));
    }
    match project {
        Some(p) => advice.push_str(&format!(": --project {p}.")),
        None => advice.push_str(": --project <p>."),
    }

    if let Some(label) = matches_project_label(conn, &worst.term)? {
        advice.push_str(&format!(" Если ты про проект {label} — укажи project."));
    }

    Ok(advice)
}

/// Совет на `topic_too_narrow`. Текст этого повода не канонизирован ни одним
/// контрактом (OQ-10, `contracts/eval-cases.md` §2.3: «ожидание ставит
/// владелец») — в отличие от `topic_too_broad`, здесь нет дословной формы,
/// которую нельзя сочинять заново. Число совпадений в прозу не идёт (FR-006:
/// оно уже несётся полем `Refusal::matched`).
fn narrow_advice(term: &TermDf) -> String {
    format!(
        "тема слишком узкая: значимых термов меньше двух — «{}» единственный. \
         Добавь второе значимое слово: термы складываются через OR, поэтому оно \
         не заменит первое, а сузит выдачу вместе с ним",
        term.term
    )
}

/// Отказ recall по ширине темы (FR-006, `contracts/mcp.md` §2.1). Решение
/// принимается до обхода графа — `walk` (`traverse.rs`) не вызывается вовсе,
/// если эта функция вернула `Some`.
///
/// `project` — уже разрешённая область вызывающего (`--project` CLI /
/// параметр `memory_recall`); используется только в хвосте совета и не
/// влияет на сам вердикт `refused`.
///
/// # Errors
/// Ошибка подготовки или исполнения запроса к индексу или к таблице `nodes`.
pub fn refuse_topic(
    conn: &Connection,
    terms: &[String],
    project: Option<&str>,
) -> Result<Option<Refusal>> {
    if terms.is_empty() {
        return Ok(None);
    }
    let total = live_node_count(conn)?;
    let mut broad = Vec::new();
    let mut narrow = Vec::new();
    for term in terms {
        let nodes = term_document_count(conn, term)?;
        let df = if total > 0 {
            nodes as f64 / total as f64
        } else {
            0.0
        };
        let entry = TermDf {
            term: term.clone(),
            df,
            nodes,
        };
        if df > DF_VETO_RATIO {
            broad.push(entry);
        } else {
            narrow.push(entry);
        }
    }

    // `topic_too_broad` проверяется первым и побеждает `topic_too_narrow`,
    // когда верны оба разом (FR-006; нормативный кейс `refuse-topic-au`
    // держится именно на этом порядке).
    if !broad.is_empty() {
        let worst_idx = broad
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.df.total_cmp(&b.df))
            .map_or(0, |(i, _)| i);
        let worst = broad[worst_idx].clone();
        let advice = broad_advice(conn, &worst, &narrow, project)?;
        return Ok(Some(Refusal {
            reason: RefusalReason::TopicTooBroad,
            matched: worst.nodes,
            terms: broad,
            advice,
        }));
    }

    if narrow.len() < MIN_SIGNIFICANT_TERMS {
        let matched = narrow.first().map_or(0, |t| t.nodes);
        let advice = narrow.first().map_or_else(
            || "тема слишком узкая: значимых термов меньше двух".to_owned(),
            narrow_advice,
        );
        return Ok(Some(Refusal {
            reason: RefusalReason::TopicTooNarrow,
            matched,
            terms: narrow,
            advice,
        }));
    }

    Ok(None)
}

pub fn search_typed(
    conn: &Connection,
    query: &str,
    node_type: &NodeType,
    limit: usize,
) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(node_type)?;
    let trimmed = query.trim();
    let parsed = crate::fts::parse(trimmed);
    let expr = parsed.expr;
    if trimmed.is_empty() || trimmed == "*" || expr.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                    memory_kind, last_accessed_at, access_count, content_hash,
                    created_by, updated_by, deleted_at, sync_seq
             FROM nodes WHERE node_type = ?1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT ?2",
        )?;
        let mut nodes = stmt
            .query_map(params![type_str, limit as i64], row_to_node)?
            .collect::<Result<Vec<_>, _>>()?;
        // Находка 11: тип "config" наравне с обычными настройками включает
        // координаты секретов — им сюда нельзя (FR-027).
        nodes.retain(|n| !crate::secret::is_secret_ref(n));
        return Ok(nodes);
    }
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash,
                n.created_by, n.updated_by, n.deleted_at, n.sync_seq
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1 AND n.node_type = ?2 AND n.deleted_at IS NULL
         ORDER BY rank
         LIMIT ?3",
    )?;
    let mut nodes = stmt
        .query_map(
            params![expr, type_str, (limit * OVERFETCH) as i64],
            row_to_node,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    nodes.retain(|n| !crate::secret::is_secret_ref(n));
    rank_by_matched_terms(&mut nodes, &parsed.terms);
    nodes.truncate(limit);
    Ok(nodes)
}

/// Статусы незакрытой работы.
///
/// `backlog` входит намеренно: `task_create` создаёт задачу именно в нём, а
/// выборки спрашивали только `active,blocked`. Свежесозданная задача была
/// невидима и в снапшоте, и в `memory_status` до ручной активации — то есть
/// завести задачу означало потерять её.
pub const OPEN_TASK_STATUSES: &str = "active,blocked,backlog";

/// Колонки узла в порядке, которого ждёт [`row_to_node`].
const NODE_COLS: &str = "n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, \
                         n.updated_at, n.memory_kind, n.last_accessed_at, n.access_count, \
                         n.content_hash, n.created_by, n.updated_by, n.deleted_at, n.sync_seq";

/// SQL-условие «узел принадлежит проекту». Один позиционный параметр `?idx`
/// (имя проекта) переиспользуется внутри условия.
///
/// Принадлежность исторически кодировалась ТОЛЬКО префиксом метки `[проект]`.
/// Но `memory_add` кладёт узел с голой меткой, а связь с проектом ставится
/// отдельным ребром через `memory_relate`, которое ни одна выборка не читала.
/// Документированный способ записи знания создавал узлы, невидимые для любого
/// проектного запроса; симптом — снапшот отдавал только служебные слои 7-8 при
/// полном графе. Считаются оба способа: метка и ребро.
///
/// Ребро засчитывается в обе стороны и при любом типе связи. Направление здесь
/// не несёт смысла: `memory_relate` ставит `узел -> проект`, а индексатор
/// связывает `проект -> файл`. Требовать конкретный тип связи тоже нельзя —
/// словарь отношений открыт, и промах в нём снова означал бы тихую потерю
/// знания. Ложное срабатывание ограничено фильтром по типу узла у вызывающего.
fn project_scope_sql(alias: &str, idx: u32) -> String {
    format!(
        "({alias}.label LIKE '[' || ?{idx} || ']%' \
          OR {alias}.label = ?{idx} \
          OR EXISTS (SELECT 1 FROM edges pe \
                       JOIN nodes pn ON pn.id = pe.to_id \
                      WHERE pe.from_id = {alias}.id \
                        AND pe.deleted_at IS NULL \
                        AND pn.deleted_at IS NULL \
                        AND pn.label = ?{idx}) \
          OR EXISTS (SELECT 1 FROM edges pe2 \
                       JOIN nodes pn2 ON pn2.id = pe2.from_id \
                      WHERE pe2.to_id = {alias}.id \
                        AND pe2.deleted_at IS NULL \
                        AND pn2.deleted_at IS NULL \
                        AND pn2.label = ?{idx}))"
    )
}

/// Свежие узлы одного типа в области проекта (или глобально при `None`).
///
/// Заменяет два прежних приёма: полнотекстовый поиск по литералу `"[проект]"`
/// и фильтрацию уже вычитанных узлов по префиксу метки на стороне Rust. Оба
/// видели только метку и оба молчали про связанное ребром.
pub fn typed_in_project(
    conn: &Connection,
    node_type: &NodeType,
    project: Option<&str>,
    limit: usize,
) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(node_type)?;
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(type_str)];
    let scope = match project {
        Some(p) => {
            params_vec.push(Box::new(p.to_string()));
            format!(" AND {}", project_scope_sql("n", 2))
        }
        None => String::new(),
    };
    let limit_idx = params_vec.len() + 1;
    let sql = format!(
        "SELECT {NODE_COLS}
           FROM nodes n
          WHERE n.node_type = ?1 AND n.deleted_at IS NULL{scope}
          ORDER BY n.updated_at DESC
          LIMIT ?{limit_idx}"
    );
    params_vec.push(Box::new(limit as i64));

    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let nodes = stmt
        .query_map(params_refs.as_slice(), row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Факты о владельце для первого слоя снапшота: принадлежащие проекту по
/// [`project_scope_sql`] — первыми, затем глобальные, каждая группа свежими
/// первыми.
///
/// Глобальный факт — тот, что не связан ни с одним проектом: ни префикса
/// `[проект]`, ни ребра к узлу типа `project`. Факт, связанный с ДРУГИМ
/// проектом, не попадает никогда. Раньше слой брал 12 свежих фактов без
/// области вовсе; замер 19.09.2026 на области aurelius — ни одна из двенадцати
/// строк не касалась aurelius (xhub, boostix, diktor, ulika). Принадлежность
/// решается связями, а не угадыванием по тексту.
///
/// `project` = `None` — только глобальные.
pub fn owner_facts(conn: &Connection, project: Option<&str>, limit: usize) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(&NodeType::UserFact)?;
    let project_type = serde_json::to_string(&NodeType::Project)?;
    let unassociated = "(n.label NOT LIKE '[%]%' \
          AND NOT EXISTS (SELECT 1 FROM edges ae \
                            JOIN nodes ap ON ap.id = CASE WHEN ae.from_id = n.id \
                                                          THEN ae.to_id ELSE ae.from_id END \
                           WHERE (ae.from_id = n.id OR ae.to_id = n.id) \
                             AND ae.deleted_at IS NULL \
                             AND ap.deleted_at IS NULL \
                             AND ap.node_type = ?2))";
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(type_str), Box::new(project_type)];
    let (filter, order) = match project {
        Some(p) => {
            params_vec.push(Box::new(p.to_string()));
            let scope = project_scope_sql("n", 3);
            (
                format!("({scope} OR {unassociated})"),
                format!("{scope} DESC, n.updated_at DESC"),
            )
        }
        None => (unassociated.to_owned(), "n.updated_at DESC".to_owned()),
    };
    let limit_idx = params_vec.len() + 1;
    let sql = format!(
        "SELECT {NODE_COLS}
           FROM nodes n
          WHERE n.node_type = ?1 AND n.deleted_at IS NULL AND {filter}
          ORDER BY {order}
          LIMIT ?{limit_idx}"
    );
    params_vec.push(Box::new(limit as i64));

    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let nodes = stmt
        .query_map(params_refs.as_slice(), row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Проблемы без связанного решения. `project` = `None` — глобально.
pub fn get_unsolved_problems(
    conn: &Connection,
    project: Option<&str>,
    limit: usize,
) -> Result<Vec<Node>> {
    let problem_type = serde_json::to_string(&NodeType::Problem)?;
    let solution_type = serde_json::to_string(&NodeType::Solution)?;
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(problem_type), Box::new(solution_type)];
    let scope = match project {
        Some(p) => {
            params_vec.push(Box::new(p.to_string()));
            format!(" AND {}", project_scope_sql("n", 3))
        }
        None => String::new(),
    };
    let limit_idx = params_vec.len() + 1;
    let sql = format!(
        "SELECT {NODE_COLS}
           FROM nodes n
          WHERE n.node_type = ?1
            AND n.deleted_at IS NULL
            AND NOT EXISTS (
              SELECT 1 FROM edges e
              JOIN nodes sol ON sol.id = e.from_id AND sol.node_type = ?2
              WHERE e.to_id = n.id AND e.relation = 'solves' AND e.deleted_at IS NULL
            ){scope}
          ORDER BY n.created_at DESC
          LIMIT ?{limit_idx}"
    );
    params_vec.push(Box::new(limit as i64));

    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let nodes = stmt
        .query_map(params_refs.as_slice(), row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Get tasks filtered by project (label prefix or `belongs_to`-style edge),
/// status, and priority (from JSON `data` column).
/// Results sorted by priority (critical > high > medium > low), then by created_at desc.
pub fn get_tasks_filtered(
    conn: &Connection,
    project: Option<&str>,
    status: Option<&str>,
    priority: Option<&str>,
    limit: usize,
) -> Result<Vec<Node>> {
    let task_type = serde_json::to_string(&NodeType::Task)?;
    let mut conditions = vec![
        "n.node_type = ?1".to_string(),
        "n.deleted_at IS NULL".to_string(),
    ];
    let mut param_idx = 2u32;

    // We'll build dynamic SQL with positional params
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(task_type)];

    if let Some(proj) = project {
        conditions.push(project_scope_sql("n", param_idx));
        params_vec.push(Box::new(proj.to_string()));
        param_idx += 1;
    }

    if let Some(st) = status {
        // Support comma-separated statuses
        let statuses: Vec<&str> = st.split(',').map(|s| s.trim()).collect();
        let placeholders: Vec<String> = statuses
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", param_idx + i as u32))
            .collect();
        conditions.push(format!(
            "json_extract(n.data, '$.status') IN ({})",
            placeholders.join(", ")
        ));
        for s in statuses {
            params_vec.push(Box::new(s.to_string()));
            param_idx += 1;
        }
    }

    if let Some(pri) = priority {
        conditions.push(format!("json_extract(n.data, '$.priority') = ?{param_idx}"));
        params_vec.push(Box::new(pri.to_string()));
        param_idx += 1;
    }
    let _ = param_idx; // suppress unused warning

    let sql = format!(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash,
                n.created_by, n.updated_by, n.deleted_at, n.sync_seq
         FROM nodes n
         WHERE {}
         ORDER BY
           CASE json_extract(n.data, '$.priority')
             WHEN 'critical' THEN 0
             WHEN 'high' THEN 1
             WHEN 'medium' THEN 2
             WHEN 'low' THEN 3
             ELSE 4
           END,
           n.created_at DESC
         LIMIT ?{}",
        conditions.join(" AND "),
        params_vec.len() + 1
    );

    params_vec.push(Box::new(limit as i64));

    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let nodes = stmt
        .query_map(params_refs.as_slice(), row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Сколько ждать ответа от демона на embed-сокете, прежде чем сдаться и
/// уйти на чистый FTS5 (спека 011, `data-model.md` §6).
///
/// Пять секунд, а не две: демон поднимает модель лениво и снимает после
/// простоя, холодная загрузка измерена в 1.5–3.7 с, и при двух секундах
/// первый поиск после паузы молча уходил на полнотекстовый. Мёртвый сокет
/// этого запаса не стоит: нет файла или отказ в соединении возвращаются
/// сразу, ждёт только живой демон, который грузит веса.
const EMBED_SOCKET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Просит у демона вектор запроса и превращает любой отказ в понятную
/// причину — никогда в ошибку, которую вызывающий обязан разбирать.
///
/// Общая точка пяти входных точек (спека 011, волна «hybrid everywhere»):
/// MCP `memory_search`/`memory_recall`/`memory_context` и CLI `au search`
/// (обе формы вывода)/`au context` — просят вектор ровно здесь, а не каждый
/// у себя. Лесенка отказа (нет сокета, отказано в соединении, таймаут, демон
/// ответил `{"error": ...}`) одна и та же у всех пяти; пять её копий
/// разошлись бы молча при первой же правке таймаута или текста причины.
///
/// `home` — каталог, где лежит сокет (`db_path().parent()`), тот же, что
/// держит и саму базу.
///
/// Возвращает `(Some(vector), None)` на успехе и `(None, Some(reason))` на
/// любом отказе — никогда `Err`: деградация к FTS5-only обязана быть явной,
/// а не остановить вызывающего (spec.md, ограничение №2). `reason` уже готов
/// к показу — и человеку, и в поле JSON-ответа.
pub async fn query_vector_for_search(
    home: &std::path::Path,
    query: &str,
) -> (Option<Vec<f32>>, Option<String>) {
    let socket = crate::embed_socket::socket_path(home);
    match crate::embed_socket::request_vector(&socket, query, EMBED_SOCKET_TIMEOUT).await {
        Ok(vector) => (Some(vector), None),
        Err(reason) => (
            None,
            Some(format!(
                "векторная половина недоступна, отвечаю по полнотекстовому — {reason}"
            )),
        ),
    }
}

/// Сколько записей берёт каждая из двух сторон гибридного поиска до слияния
/// (спека 011, «Архитектура пайплайна»: FTS5 top-50 AND dense KNN top-50).
pub const FUSION_POOL: usize = 50;

/// Ближайшие соседи вектора запроса в `node_embeddings` (миграция v15→v16,
/// `db.rs`), ближайший первым. Пустой список — не ошибка, ни когда таблицы
/// нет вовсе (фикстура на старой схеме), ни когда она есть, но пуста (векторы
/// ещё не досчитаны `db reindex-embeddings`): деградация к FTS-only обязана
/// быть тихой (спека 011, «Graceful degradation без векторов»).
///
/// # Errors
/// Ошибка исполнения запроса к `node_embeddings` или к `nodes` — не считая
/// самого отсутствия таблицы, которое ловится раньше и молча даёт `Ok(vec![])`.
pub fn dense_search(conn: &Connection, query_vector: &[f32], limit: usize) -> Result<Vec<Node>> {
    if !node_embeddings_available(conn)? {
        return Ok(Vec::new());
    }
    let bytes = f32_le_bytes(query_vector);
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash,
                n.created_by, n.updated_by, n.deleted_at, n.sync_seq
         FROM (
             SELECT rowid, distance FROM node_embeddings
              WHERE embedding MATCH vec_quantize_int8(?1, 'unit') AND k = ?2
              ORDER BY distance
         ) ve
         JOIN nodes n ON n.rowid = ve.rowid
         WHERE n.deleted_at IS NULL
         ORDER BY ve.distance",
    )?;
    let mut nodes = stmt
        .query_map(params![bytes, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    // Находка 11 (FR-027) — тот же фильтр, что у обоих FTS-путей: координата
    // секрета не должна утечь через смысловое сходство точно так же, как не
    // утекает через совпадение слов. Технический узел — тоже: вектор у него
    // мог появиться до того, как очередь перестала их принимать.
    nodes.retain(|n| !crate::secret::is_secret_ref(n) && !is_technical(n));
    Ok(nodes)
}

/// Таблица векторов существует и не пуста.
///
/// Отдельная проверка `sqlite_master` до самого KNN-запроса, а не перехват
/// «no such table» из результата `prepare`: перехват ошибки по тексту или коду
/// не отличил бы «таблицы нет» от кривого SQL в этом же запросе, а плодить
/// для этого собственный код ошибки — цена, которую отсутствие таблицы на
/// старой фикстуре не стоит.
fn node_embeddings_available(conn: &Connection) -> Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'node_embeddings')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM node_embeddings", [], |row| row.get(0))?;
    Ok(count > 0)
}

/// `f32` в little-endian байты — формат, который `vec_quantize_int8` ждёт на
/// входе того же вида, каким его пишет `INSERT` на стороне записи
/// (`commands.rs::db_reindex_embeddings_cli`, `bytemuck::cast_slice`).
/// `aurelius-core` не тянет `bytemuck` ради одного места — стандартной
/// `to_le_bytes` на четыре байта достаточно.
fn f32_le_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Слияние FTS5 и векторного поиска в посев обхода (спека 011,
/// `data-model.md` §2, `spec.md` «Архитектура пайплайна»): top-50 с каждой
/// стороны → RRF (`rank::rrf_score`, `rank::RRF_K`) → верхние `limit`.
///
/// RRF складывает вклад по РАНГУ в каждом из двух списков, не по самой
/// релевантности — оба списка совместимы уже по построению, нормализация
/// между FTS5 и dense не нужна. Когда `dense_search` отдаёт пустой список
/// (таблицы нет или она пуста), RRF вырождается в `1 / (k + rank_fts)` —
/// строго убывающую функцию от ранга FTS, поэтому сортировка по RRF даёт
/// РОВНО тот же порядок, что и чистый bm25-порядок `search_ranked`: слияние
/// не меняет ни состав, ни порядок посева там, где векторов нет (та же
/// гарантия деградации, что и в `dense_search`, но уже для слияния целиком).
///
/// Возвращает узлы вместе с их нормированным RRF-скором — мин-макс
/// (`rank::normalize_minmax`, НЕ `rank::normalize_bm25`: медианная нормировка
/// той топит узкий разброс RRF-скора в одну точку у `0.5`, см. её
/// доккомментарий) по всему пулу кандидатов, растянутый в `[r_traversed, 1]`,
/// а не в `[0, 1]`: попадание в слияние хотя бы одним из двух движков — сигнал
/// сильнее, чем «узел дошёл только обходом графа», и не имеет права стоить
/// МЕНЬШЕ той же фиксированной константы. Это и есть `r`, который
/// `graph::traverse` подставляет в `rank::score` для посевных узлов вместо
/// фиксированного `RankWeights::r_traversed`.
///
/// # Errors
/// Ошибка `search_ranked` или `dense_search`.
pub fn hybrid_seeds(
    conn: &Connection,
    query: &str,
    query_vector: &[f32],
    limit: usize,
) -> Result<(Vec<Node>, std::collections::HashMap<uuid::Uuid, f64>)> {
    hybrid_seeds_pooled(conn, query, query_vector, limit, FUSION_POOL)
}

/// [`hybrid_seeds`] с пулом каждой стороны `pool` вместо [`FUSION_POOL`] —
/// для выдачи, которую попросили длиннее, чем влезает в слияние двух
/// top-50 (`au search --limit`): иначе она молча обрезалась бы на сотне.
///
/// # Errors
/// Ошибка `search_ranked` или `dense_search`.
pub fn hybrid_seeds_pooled(
    conn: &Connection,
    query: &str,
    query_vector: &[f32],
    limit: usize,
    pool: usize,
) -> Result<(Vec<Node>, std::collections::HashMap<uuid::Uuid, f64>)> {
    let fts_nodes = search_ranked(conn, query, pool)?.nodes;
    let dense_nodes = dense_search(conn, query_vector, pool)?;

    let mut by_id: std::collections::HashMap<uuid::Uuid, Node> = std::collections::HashMap::new();
    let mut fts_rank: std::collections::HashMap<uuid::Uuid, usize> =
        std::collections::HashMap::new();
    for (i, node) in fts_nodes.into_iter().enumerate() {
        fts_rank.insert(node.id, i + 1);
        by_id.insert(node.id, node);
    }
    let mut dense_rank: std::collections::HashMap<uuid::Uuid, usize> =
        std::collections::HashMap::new();
    for (i, node) in dense_nodes.into_iter().enumerate() {
        dense_rank.insert(node.id, i + 1);
        by_id.entry(node.id).or_insert(node);
    }

    let pool: Vec<(uuid::Uuid, f64)> = by_id
        .keys()
        .map(|id| {
            let blind = by_id
                .get(id)
                .is_some_and(|n| super::fusion::fts_blind_to(query, n));
            let score = super::rank::rrf_score(
                fts_rank.get(id).copied(),
                dense_rank.get(id).copied(),
                blind,
            );
            (*id, score)
        })
        .collect();
    // Мин-макс на весь пул (до сотни кандидатов: объединение двух top-50), а
    // не только на верхнюю дюжину после отсечения: нормировка на одном лишь
    // топ-12 меряет узел против одиннадцати лучших конкурентов, а не против
    // всего круга, что нашли оба движка.
    let raw: Vec<f64> = pool.iter().map(|(_, score)| *score).collect();
    let normalized = super::rank::normalize_minmax(&raw);

    // Растянуто в [r_traversed, 1], не в [0, 1]: посев — это узел, который
    // нашёл хотя бы один из двух движков, и это факт СИЛЬНЕЕ, чем «узел
    // пришёл только обходом графа» — попадание в слияние не имеет права
    // стоить МЕНЬШЕ фиксированного `r_traversed`, которым и так награждён
    // любой сосед по графу без единого совпадения ни в одном из движков.
    // Худший из пула получает ровно `r_traversed` (нейтрально, как и раньше),
    // лучший — максимум шкалы; линейно между ними — единственный рычаг,
    // который эта правка вообще трогает.
    let r_floor = super::rank::RankWeights::default().r_traversed;
    let mut scored: Vec<(uuid::Uuid, f64)> = pool
        .into_iter()
        .zip(normalized)
        .map(|((id, _), norm)| (id, r_floor + (1.0 - r_floor) * norm))
        .collect();
    // Тай-брейк по id: `HashMap` не гарантирует порядок обхода, а RRF-скор
    // двух узлов может совпасть (например, один нашёлся только первым в FTS,
    // другой — только первым в dense: у обоих 1/(k+1)). Без явного тай-брейка
    // порядок между такими узлами был бы недетерминирован между прогонами.
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(limit);

    let mut nodes = Vec::with_capacity(scored.len());
    let mut r = std::collections::HashMap::with_capacity(scored.len());
    for (id, norm) in scored {
        if let Some(node) = by_id.remove(&id) {
            r.insert(id, norm);
            nodes.push(node);
        }
    }
    Ok((nodes, r))
}

pub fn get_recent_nodes(conn: &Connection, limit: usize) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT ?1",
    )?;
    let nodes = stmt
        .query_map(params![limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (std::path::PathBuf, Connection) {
        let path =
            std::env::temp_dir().join(format!("aurelius-search-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).expect("open temp db");
        (path, conn)
    }

    fn cleanup(path: &std::path::Path, conn: Connection) {
        drop(conn);
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(p));
        }
    }

    /// Гигиена выдачи: строка, которую читатель увидел бы дважды, уходит
    /// один раз; одна метка при разных телах — разные записи; пустая запись
    /// уступает содержательной только при ничьей, а не при более высоком
    /// скоре; `limit` режет уже после склейки.
    #[test]
    fn tidy_drops_repeated_rows_and_puts_bare_ones_last_only_at_a_tie() {
        let (path, conn) = temp_db();
        let add = |label: &str, note: Option<&str>, data: serde_json::Value| {
            super::super::add_node(&conn, NodeType::Dependency, label, note, "test", data)
                .expect("add node")
        };
        let bare = add("rust-embed", None, serde_json::json!({}));
        let bare_twin = add("rust-embed", None, serde_json::json!({}));
        let with_note = add("fastembed", Some("bge-m3 runtime"), serde_json::json!({}));
        let other_note = add("fastembed", Some("pinned to 6.1"), serde_json::json!({}));
        let with_claim = add("ort", None, serde_json::json!({"claim": "load-dynamic"}));
        let bare_high = add("hf-hub", None, serde_json::json!({}));

        let scored = vec![
            (bare_high.clone(), 1.0),
            (bare.clone(), 0.9),
            (bare_twin, 0.9),
            (with_note.clone(), 0.9),
            (with_claim.clone(), 0.9),
            (other_note.clone(), 0.5),
        ];
        let ids: Vec<_> = tidy_for_emission(scored.clone(), 10)
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                bare_high.id,
                with_note.id,
                with_claim.id,
                bare.id,
                other_note.id
            ]
        );

        let top: Vec<_> = tidy_for_emission(scored, 3).iter().map(|n| n.id).collect();
        assert_eq!(top, vec![bare_high.id, with_note.id, with_claim.id]);

        cleanup(&path, conn);
    }

    /// Прогон — бухгалтерия, а не знание: даже с высшим скором он не вытесняет
    /// знание, а на свободное место встаёт самое большее один — и старой
    /// формы `Custom("run")` это касается так же.
    #[test]
    fn run_rows_yield_to_knowledge_and_fill_at_most_one_free_slot() {
        let (path, conn) = temp_db();
        let add = |t: NodeType, label: &str| {
            super::super::add_node(&conn, t, label, Some(label), "test", serde_json::json!({}))
                .expect("add node")
        };
        let run_new = add(NodeType::Run, "прогон: cargo test");
        let run_old = add(NodeType::Custom("run".into()), "прогон: au search");
        let a = add(NodeType::Decision, "решение про сокет");
        let b = add(NodeType::Concept, "понятие про сокет");

        let scored = vec![
            (run_new.clone(), 1.0),
            (run_old, 0.95),
            (a.clone(), 0.5),
            (b.clone(), 0.4),
        ];
        let full: Vec<_> = tidy_for_emission(scored.clone(), 2)
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(full, vec![a.id, b.id], "знания хватает — прогонов нет");

        let short: Vec<_> = tidy_for_emission(scored, 5).iter().map(|n| n.id).collect();
        assert_eq!(
            short,
            vec![a.id, b.id, run_new.id],
            "на свободное место — один прогон, старший по скору"
        );

        cleanup(&path, conn);
    }

    /// SQL-форма и Rust-форма «технического узла» обязаны совпадать: первая
    /// решает, кому не считать вектор, вторая — кого не показывать.
    #[test]
    fn technical_sql_agrees_with_is_technical() {
        let (path, conn) = temp_db();
        let add = |t: NodeType, note: Option<&str>, data: serde_json::Value| {
            super::super::add_node(&conn, t, "узел", note, "test", data).expect("add node")
        };
        let nodes = [
            add(NodeType::Run, None, serde_json::json!({})),
            add(NodeType::Custom("run".into()), None, serde_json::json!({})),
            add(
                NodeType::Dependency,
                None,
                serde_json::json!({"manifests": {}}),
            ),
            add(NodeType::Dependency, Some("  "), serde_json::json!({})),
            add(
                NodeType::Dependency,
                None,
                serde_json::json!({"claim": "закреплена"}),
            ),
            add(
                NodeType::Dependency,
                Some("почему 6.1"),
                serde_json::json!({}),
            ),
            add(NodeType::Concept, None, serde_json::json!({})),
        ];

        let dropped = drop_technical_from_queue(&conn).expect("отсев очереди");

        let queued: std::collections::HashSet<String> = conn
            .prepare("SELECT node_id FROM embedding_queue")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        let expected: std::collections::HashSet<String> = nodes
            .iter()
            .filter(|n| !is_technical(n))
            .map(|n| n.id.to_string())
            .collect();
        assert_eq!(queued, expected);
        assert_eq!(dropped, 4);

        cleanup(&path, conn);
    }

    /// Поиск и лента по `*` технических узлов не отдают; явный фильтр по
    /// типу — отдаёт, там это сам вопрос.
    #[test]
    fn search_hides_technical_nodes_but_a_type_filter_finds_them() {
        let (path, conn) = temp_db();
        let add = |t: NodeType, label: &str, note: Option<&str>| {
            super::super::add_node(&conn, t, label, note, "test", serde_json::json!({}))
                .expect("add node")
        };
        let dep = add(NodeType::Dependency, "rust-embed", None);
        add(NodeType::Run, "прогон: embed socket", None);
        let concept = add(NodeType::Concept, "embed socket демона", Some("сокет"));

        let found: Vec<_> = search(&conn, "embed", 10)
            .expect("поиск")
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(found, vec![concept.id]);

        let recent: Vec<_> = search(&conn, "*", 10)
            .expect("лента")
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(recent, vec![concept.id]);

        let typed: Vec<_> = search_typed(&conn, "embed", &NodeType::Dependency, 10)
            .expect("поиск по типу")
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(typed, vec![dep.id]);

        cleanup(&path, conn);
    }

    /// Дефис в FTS5 — оператор `NOT`, и запрос `skills-store` отвечал «no such
    /// column: store». Так названы почти все скиллы, так что поиск падал на
    /// самых обычных словах.
    #[test]
    fn hyphenated_query_finds_instead_of_failing() {
        let (path, conn) = temp_db();
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "скилл rust-clean-code",
            Some("карточка лежит в .claude/skills проекта"),
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let found = search(&conn, "rust-clean-code", 5).expect("поиск не должен падать");
        assert_eq!(found.len(), 1, "дефисное имя обязано находиться");

        let typed = search_typed(&conn, "rust-clean-code", &NodeType::Concept, 5)
            .expect("типизированный поиск не должен падать");
        assert_eq!(typed.len(), 1);

        cleanup(&path, conn);
    }

    /// Ради чего менялся разбор: одно слово в неудачной форме обнуляло выдачу
    /// целиком, потому что пробел в FTS5 — это AND. Теперь оно портит порядок,
    /// а не результат, и лучшее совпадение стоит первым.
    #[test]
    fn one_bad_word_no_longer_empties_the_result() {
        let (path, conn) = temp_db();
        for (label, note) in [
            ("отправка алерта в телеграм", "и то и другое слово тут есть"),
            ("телеграм-бот", "только одно слово из запроса"),
            ("совсем про другое", "ни одного слова запроса"),
        ] {
            super::super::add_node(
                &conn,
                NodeType::Concept,
                label,
                Some(note),
                "test",
                serde_json::json!({}),
            )
            .expect("add node");
        }

        // «алертов» нет в индексе ни в какой форме — раньше это давало ноль.
        let found = search(&conn, "телеграм алертов", 5).expect("поиск");
        assert!(
            !found.is_empty(),
            "неудачная форма одного слова не должна обнулять выдачу"
        );

        let ranked = search_ranked(&conn, "телеграм алерта", 5).expect("поиск");
        assert_eq!(
            ranked.nodes.first().map(|n| n.label.as_str()),
            Some("отправка алерта в телеграм"),
            "первым обязано идти совпадение по двум словам, а не по одному: {:?}",
            ranked.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
        );

        cleanup(&path, conn);
    }

    /// Инструмент обязан отличать «знания нет» от «запрос не сработал».
    #[test]
    fn a_word_that_matched_nothing_is_named() {
        let (path, conn) = temp_db();
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "отправка алерта в телеграм",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let outcome = search_ranked(&conn, "телеграм алертов", 5).expect("поиск");
        assert_eq!(
            outcome.unmatched_terms,
            vec!["алертов".to_owned()],
            "слово, не давшее ни одного попадания, обязано быть названо"
        );

        assert!(
            outcome
                .diagnosis()
                .is_some_and(|d| d.contains("словоформе")),
            "не нашлось слово — назвать причиной форму, а не отсутствие знания"
        );

        let clean = search_ranked(&conn, "телеграм", 5).expect("поиск");
        assert!(
            clean.unmatched_terms.is_empty(),
            "у сработавшего запроса жаловаться не на что"
        );
        assert!(clean.diagnosis().is_none());

        cleanup(&path, conn);
    }

    /// Поиск знает про свою строку, а не про мир. «Знания здесь просто нет» —
    /// утверждение, которого у него на руках нет: по «воркеры» выдача была
    /// пустой, а «~20 воркеров» лежало в трёх узлах. Ложное успокоение дороже
    /// молчания: читатель идёт делать заново уже сделанное.
    #[test]
    fn a_miss_never_claims_the_knowledge_is_absent() {
        let (path, conn) = temp_db();
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "воркеров примерно двадцать",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let outcome = search_ranked(&conn, "воркеры", 5).expect("поиск");
        let d = outcome
            .diagnosis()
            .expect("пустая выдача обязана объясниться");
        assert!(
            !d.contains("знания здесь просто нет") && !d.contains("знания нет —"),
            "поиск не имеет права утверждать про мир: {d}"
        );
        assert!(d.contains("словоформе"), "{d}");
        assert!(d.contains("(воркер*)"), "совет обязан снять окончание: {d}");

        cleanup(&path, conn);
    }

    /// Совет должен работать в первую очередь на коротких словах: у них
    /// окончание и есть вся разница. Фиксированные пять символов возвращали
    /// «батчи» → «батчи*» — то же слово со звёздочкой и те же ноль попаданий.
    #[test]
    fn the_suggested_prefix_is_shorter_than_the_word_it_replaces() {
        assert_eq!(prefix_hint("батчи").as_deref(), Some("батч"));
        assert_eq!(prefix_hint("воркеры").as_deref(), Some("воркер"));
        // Короче корня префикс ничего не расширяет — советовать нечего.
        assert_eq!(prefix_hint("код"), None);
        assert_eq!(prefix_hint("тест"), None);
    }

    /// Заголовок — то, что автор счёл сутью записи, и весит больше простыни.
    /// Раньше bm25 считал по всему документу, и длинная сессионная запись, где
    /// те же слова рассыпаны по тексту, обходила точный заголовок.
    #[test]
    fn a_precise_label_outranks_a_long_note() {
        let (path, conn) = temp_db();
        super::super::add_node(
            &conn,
            NodeType::Session,
            "итог сессии 2026-08-17",
            Some(
                "за день сделано много: горячая перезагрузка кое-где упоминалась, \
                 флаги трогали, алерты обсуждали, модули двигали, а ещё чинили поиск, \
                 правили пробы, выпускали релиз, обновляли документацию и бинарники",
            ),
            "test",
            serde_json::json!({}),
        )
        .expect("add node");
        super::super::add_node(
            &conn,
            NodeType::Decision,
            "Горячая перезагрузка флагов доведена до алертов и разбита на модули",
            Some("короткая записка"),
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let found = search(&conn, "горячая перезагрузка флагов алерты модули", 5).expect("поиск");
        assert_eq!(
            found.first().map(|n| n.label.as_str()),
            Some("Горячая перезагрузка флагов доведена до алертов и разбита на модули"),
            "запись, у которой запрос почти целиком в заголовке, обязана быть первой: {:?}",
            found.iter().map(|n| &n.label).collect::<Vec<_>>()
        );

        cleanup(&path, conn);
    }

    /// Находка 11: координата секрета — обычный узел `Config`, и старый
    /// `nodes_fts` индексирует его наравне со всеми (label+note+data). До
    /// фильтра в `search_ranked`/`search_typed` запрос по слову из назначения
    /// или места хранения секрета отдавал координату целиком — то, что
    /// FR-025/FR-027 разрешают доставать только явным `secret_list`.
    #[test]
    fn secret_ref_never_surfaces_through_general_search() {
        let (path, conn) = temp_db();
        super::super::add_secret_ref(
            &conn,
            Some("proj-leak"),
            "STRIPE_SECRET_KEY",
            Some("charge webhooks"),
            "1password://Private/Stripe/api-key",
        )
        .expect("add secret ref");
        // Обычный узел с тем же словом — контроль, что фильтр не глушит
        // поиск целиком, а бьёт только по координате секрета.
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "интеграция с Stripe обсуждалась на созвоне",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let found = search(&conn, "stripe", 10).expect("поиск");
        assert!(
            !found.is_empty(),
            "фильтр не должен глушить поиск целиком: {found:?}"
        );
        assert!(
            found
                .iter()
                .all(|n| n.data.get("kind").and_then(|v| v.as_str()) != Some("secret_ref")),
            "координата секрета не должна попадать в общий поиск: {found:?}"
        );

        let typed =
            search_typed(&conn, "stripe", &NodeType::Config, 10).expect("типизированный поиск");
        assert!(
            typed.is_empty(),
            "координата секрета не должна попадать и в поиск по типу config: {typed:?}"
        );

        cleanup(&path, conn);
    }

    /// Строка из одних операторов и кавычек не должна ни падать, ни выдумывать
    /// результат: это «искать нечего», а не «база сломалась».
    #[test]
    fn punctuation_only_query_is_harmless() {
        let (path, conn) = temp_db();
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "любой узел",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        assert!(search(&conn, "\"", 5).is_ok());
        assert!(search_typed(&conn, "\"", &NodeType::Concept, 5).is_ok());

        cleanup(&path, conn);
    }

    /// T015: посевы упорядочиваются нормированным bm25 и ничем иным —
    /// `rank_by_matched_terms` (веса `W_LABEL`/`W_CLAIM`/`W_NOTE`) больше не
    /// участвует в этом пути. Подобранный контрпример: по `matched_score`
    /// выиграл бы узел с одним словом в заголовке (вес 3) — тут решает
    /// только совокупный bm25, и он отдаёт узел с двумя словами в `note`
    /// (вес был бы 1+1=2, то есть меньше). Измерено отдельным прогоном FTS5
    /// вне cargo (`sqlite3` CLI на той же паре документов).
    #[test]
    fn seeds_follow_bm25_alone_not_field_weighted_matched_terms() {
        let (path, conn) = temp_db();
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "alpha widget",
            Some("ничего особенного тут нет вообще"),
            "test",
            serde_json::json!({}),
        )
        .expect("add node");
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "что-то другое совсем",
            Some("beta and gamma details are noted here"),
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let ranked = search_ranked(&conn, "alpha beta gamma", 5).expect("поиск");
        assert_eq!(
            ranked.nodes.first().map(|n| n.label.as_str()),
            Some("что-то другое совсем"),
            "по matched_score выиграл бы узел с одним словом в заголовке — \
             здесь решает только bm25, и он отдаёт узел с двумя словами в \
             note: {:?}",
            ranked.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
        );

        cleanup(&path, conn);
    }

    /// T016: сколько живых узлов нужно фильтром, помимо тех, что несут
    /// значимые термы теста — раздвигает знаменатель df так, чтобы один
    /// матч у узкого терма оставался ниже вето (`1 / N < DF_VETO_RATIO`).
    fn add_filler_nodes(conn: &Connection, count: usize) {
        for i in 0..count {
            super::super::add_node(
                conn,
                NodeType::Concept,
                &format!("шум {i}"),
                None,
                "test",
                serde_json::json!({}),
            )
            .expect("add filler node");
        }
    }

    /// Терм выше вето отказывает темой `topic_too_broad`, а совет заменяет
    /// его парой узких термов из того, что осталось в самой теме (FR-006,
    /// `contracts/cli.md` §4).
    #[test]
    fn df_veto_refuses_a_broad_term_and_offers_narrow_pairs() {
        let (path, conn) = temp_db();
        add_filler_nodes(&conn, 120);
        for i in 0..5 {
            super::super::add_node(
                &conn,
                NodeType::Concept,
                &format!("everywhere штука {i}"),
                None,
                "test",
                serde_json::json!({}),
            )
            .expect("add node");
        }
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "needle1 factoid",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "needle2 factoid",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");
        // 120 + 5 + 1 + 1 = 127 живых узлов; everywhere: 5/127 ≈ 3,9 % > 1 %
        // (широкий); needle1/needle2: 1/127 ≈ 0,8 % < 1 % (значимые).

        let terms = vec![
            "everywhere".to_owned(),
            "needle1".to_owned(),
            "needle2".to_owned(),
        ];
        let refusal = refuse_topic(&conn, &terms, None)
            .expect("запрос df")
            .expect("тема обязана отказать");
        assert_eq!(refusal.reason, RefusalReason::TopicTooBroad);
        assert_eq!(refusal.matched, 5);
        assert!(
            refusal.advice.contains("не сужает"),
            "неразрывная подстрока канонического совета: {}",
            refusal.advice
        );
        assert!(
            refusal.advice.contains("«needle1 needle2»"),
            "совет обязан предложить пару из оставшихся значимых термов: {}",
            refusal.advice
        );
        assert!(
            refusal.advice.contains("--project <p>."),
            "без разрешённого project — плейсхолдер флага: {}",
            refusal.advice
        );

        cleanup(&path, conn);
    }

    /// Тема из одного широкого терма (как «au» и «guard» в спеке) не имеет
    /// собственных термов ниже вето — совет говорит это прямо и предлагает
    /// только `--project`, а не сочиняет пару из чужого словаря (FR-006).
    #[test]
    fn single_broad_term_with_nothing_below_veto_offers_project_only() {
        let (path, conn) = temp_db();
        add_filler_nodes(&conn, 120);
        for i in 0..5 {
            super::super::add_node(
                &conn,
                NodeType::Concept,
                &format!("widetopic штука {i}"),
                None,
                "test",
                serde_json::json!({}),
            )
            .expect("add node");
        }

        let terms = vec!["widetopic".to_owned()];
        let refusal = refuse_topic(&conn, &terms, None)
            .expect("запрос df")
            .expect("тема обязана отказать");
        assert_eq!(refusal.reason, RefusalReason::TopicTooBroad);
        assert!(refusal.advice.contains("не сужает"));
        assert!(
            refusal.advice.contains("Ниже вето слов не осталось"),
            "у темы без своих значимых термов совет обязан сказать это прямо: {}",
            refusal.advice
        );
        assert!(
            !refusal.advice.contains("Замените широкий терм парой узких"),
            "предлагать пару неоткуда — своих термов ниже вето нет: {}",
            refusal.advice
        );

        cleanup(&path, conn);
    }

    /// Тема из одного редкого слова — `topic_too_narrow`, а не `_broad`.
    #[test]
    fn single_rare_term_is_topic_too_narrow() {
        let (path, conn) = temp_db();
        add_filler_nodes(&conn, 120);
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "raretermxyz here",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let terms = vec!["raretermxyz".to_owned()];
        let refusal = refuse_topic(&conn, &terms, None)
            .expect("запрос df")
            .expect("тема обязана отказать");
        assert_eq!(refusal.reason, RefusalReason::TopicTooNarrow);
        assert_eq!(refusal.matched, 1);
        assert!(refusal.advice.contains("тема слишком узкая"));

        cleanup(&path, conn);
    }

    /// Двух значимых термов достаточно — тема НЕ отказывает.
    #[test]
    fn two_significant_terms_pass_without_refusal() {
        let (path, conn) = temp_db();
        add_filler_nodes(&conn, 120);
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "alpha1 here",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");
        super::super::add_node(
            &conn,
            NodeType::Concept,
            "beta2 here",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let terms = vec!["alpha1".to_owned(), "beta2".to_owned()];
        let refusal = refuse_topic(&conn, &terms, None).expect("запрос df");
        assert!(refusal.is_none(), "два значимых терма отказывать не должны");

        cleanup(&path, conn);
    }

    /// T022-правило, проверенное локально: каждая предложенная детектором
    /// пара сама проходит тот же детектор — ни `topic_too_broad` (в паре нет
    /// вето-термов), ни `topic_too_narrow` (в паре их ровно два).
    #[test]
    fn suggested_pairs_pass_their_own_detector() {
        let (path, conn) = temp_db();
        add_filler_nodes(&conn, 120);
        for i in 0..5 {
            super::super::add_node(
                &conn,
                NodeType::Concept,
                &format!("everywhere2 штука {i}"),
                None,
                "test",
                serde_json::json!({}),
            )
            .expect("add node");
        }
        for word in ["n1", "n2", "n3", "n4"] {
            super::super::add_node(
                &conn,
                NodeType::Concept,
                &format!("{word} factoid"),
                None,
                "test",
                serde_json::json!({}),
            )
            .expect("add node");
        }

        let terms = vec![
            "everywhere2".to_owned(),
            "n1".to_owned(),
            "n2".to_owned(),
            "n3".to_owned(),
            "n4".to_owned(),
        ];
        let refusal = refuse_topic(&conn, &terms, None)
            .expect("запрос df")
            .expect("тема обязана отказать");
        assert!(refusal.advice.contains("«n1 n2»"));
        assert!(refusal.advice.contains("«n3 n4»"));

        for pair in [["n1", "n2"], ["n3", "n4"]] {
            let sub_terms: Vec<String> = pair.iter().map(|s| (*s).to_owned()).collect();
            let sub = refuse_topic(&conn, &sub_terms, None).expect("запрос df");
            assert!(
                sub.is_none(),
                "предложенная пара сама не должна получать отказ: {pair:?}"
            );
        }

        cleanup(&path, conn);
    }

    /// Edge Case `guard`: терм совпал с меткой узла типа `project` — совет
    /// получает хвост с подсказкой `--project` под этим именем (spec.md,
    /// Edge Cases).
    #[test]
    fn guard_edge_case_appends_project_tail() {
        let (path, conn) = temp_db();
        add_filler_nodes(&conn, 60);
        super::super::add_node(
            &conn,
            NodeType::Project,
            "Guard",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add project node");
        // 60 + 1 = 61 живых узлов; guard: 1/61 ≈ 1,6 % > 1 % — широкий.

        let terms = vec!["guard".to_owned()];
        let refusal = refuse_topic(&conn, &terms, None)
            .expect("запрос df")
            .expect("тема обязана отказать");
        assert!(
            refusal
                .advice
                .contains("Если ты про проект Guard — укажи project."),
            "хвост про совпавший проект обязан быть дословным именем метки: {}",
            refusal.advice
        );

        cleanup(&path, conn);
    }

    /// `db::open` уже накатывает миграцию v15→v16 (`node_embeddings` через
    /// `vec0`), но ни строки в неё этот тест не пишет — тот же пустой случай,
    /// что и таблица, которой вовсе нет на старой фикстуре (спека 011,
    /// «Graceful degradation без векторов»).
    #[test]
    fn dense_search_on_empty_table_yields_empty_not_error() {
        let (path, conn) = temp_db();
        let query = vec![0.1f32; 1024];
        let found = dense_search(&conn, &query, 10).expect("деградация — не ошибка");
        assert!(found.is_empty());
        cleanup(&path, conn);
    }

    /// Слияние без вектора запроса вырождается в чистый FTS-порядок: RRF при
    /// пустом dense-списке — строго убывающая функция от ранга FTS, поэтому
    /// состав и порядок посева совпадают с `search_ranked` побайтово. Это и
    /// есть доказательство, на котором держится сохранение эталонного
    /// digest `au eval` на замороженной фикстуре без таблицы векторов.
    #[test]
    fn hybrid_seeds_degrades_to_plain_fts_order_when_dense_is_empty() {
        let (path, conn) = temp_db();
        for (label, note) in [
            ("отправка алерта в телеграм", "и то и другое слово тут есть"),
            ("телеграм-бот", "только одно слово из запроса"),
            ("совсем про другое", "ни одного слова запроса"),
        ] {
            super::super::add_node(
                &conn,
                NodeType::Concept,
                label,
                Some(note),
                "test",
                serde_json::json!({}),
            )
            .expect("add node");
        }

        let plain = search_ranked(&conn, "телеграм алерта", 5)
            .expect("поиск")
            .nodes;
        let query = vec![0.1f32; 1024];
        let (fused, r) =
            hybrid_seeds(&conn, "телеграм алерта", &query, 5).expect("слияние без dense");

        assert_eq!(
            fused.iter().map(|n| n.id).collect::<Vec<_>>(),
            plain.iter().map(|n| n.id).collect::<Vec<_>>(),
            "без dense-выдачи слияние обязано дать тот же порядок, что и чистый FTS"
        );
        assert_eq!(r.len(), fused.len());
        // Мин-макс, не медианная нормировка: лучший из посева получает
        // ровно 1, не асимптотическое приближение к нему.
        for v in r.values() {
            assert!(*v >= 0.0 && *v <= 1.0, "r ∈ [0, 1]: {v}");
        }
        assert!(
            r.values().any(|v| (*v - 1.0).abs() < 1e-9),
            "лучший узел посева обязан получить максимум шкалы"
        );

        cleanup(&path, conn);
    }
}
