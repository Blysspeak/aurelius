use crate::models::{Node, NodeType};
use anyhow::Result;
use rusqlite::{params, Connection};

use super::row_to_node;

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
/// числу совпавших слов можно только имея запас — иначе `LIMIT` отрежет лучшее
/// ещё до ранжирования.
const OVERFETCH: usize = 5;

pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Node>> {
    Ok(search_ranked(conn, query, limit)?.nodes)
}

/// Поиск с ранжированием по числу совпавших слов и с диагностикой запроса.
///
/// Слова соединены через `OR` (см. [`crate::fts`]), поэтому порядок здесь и
/// делает выдачу осмысленной: сначала записи, где совпало больше слов, внутри
/// группы — по релевантности bm25.
///
/// Координаты секретов (`secret::is_secret_ref`) отфильтрованы на всех трёх
/// путях выдачи (находка 11, FR-027): узел-координата — обычный `Config`,
/// который FTS индексирует наравне со всеми (label+note+data, см. `db.rs`),
/// и без фильтра здесь он утекал в любой запрос, задевший его метку,
/// назначение или место хранения. Явный `secret_list` идёт другим путём
/// (`typed_in_project` внутри `list_secret_refs`) и фильтр не задевает.
pub fn search_ranked(conn: &Connection, query: &str, limit: usize) -> Result<SearchOutcome> {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == "*" {
        let mut nodes = get_recent_nodes(conn, limit)?;
        nodes.retain(|n| !crate::secret::is_secret_ref(n));
        return Ok(SearchOutcome {
            nodes,
            terms: Vec::new(),
            unmatched_terms: Vec::new(),
        });
    }
    // Строка от человека — текст, а не выражение FTS5 (см. crate::fts).
    let parsed = crate::fts::parse(trimmed);
    if parsed.expr.is_empty() {
        let mut nodes = get_recent_nodes(conn, limit)?;
        nodes.retain(|n| !crate::secret::is_secret_ref(n));
        return Ok(SearchOutcome {
            nodes,
            terms: Vec::new(),
            unmatched_terms: Vec::new(),
        });
    }
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash,
                n.created_by, n.updated_by, n.deleted_at, n.sync_seq
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1 AND n.deleted_at IS NULL
         ORDER BY rank - (n.access_count * 0.1)
         LIMIT ?2",
    )?;
    let mut nodes = stmt
        .query_map(
            params![parsed.expr, (limit * OVERFETCH) as i64],
            row_to_node,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    nodes.retain(|n| !crate::secret::is_secret_ref(n));

    rank_by_matched_terms(&mut nodes, &parsed.terms);
    nodes.truncate(limit);

    Ok(SearchOutcome {
        nodes,
        unmatched_terms: unmatched_terms(conn, &parsed.terms)?,
        terms: parsed.terms,
    })
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
}
