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
    /// Состояний три, а не два, и путать их дорого: «часть слов не нашлась» —
    /// это про форму слова и означает «спроси иначе»; «не нашлось ни одного
    /// слова» — это про отсутствие знания и означает «иди и выясняй».
    #[must_use]
    pub fn diagnosis(&self) -> Option<String> {
        if self.unmatched_terms.is_empty() {
            return None;
        }
        if self.unmatched_terms.len() == self.terms.len() {
            return Some(format!(
                "ни одно слово запроса не встречается в памяти ({}) — похоже, этого знания здесь просто нет",
                self.terms.join(", ")
            ));
        }
        let stem: String = self
            .unmatched_terms
            .first()
            .map_or_else(String::new, |w| w.chars().take(5).collect());
        Some(format!(
            "ни одного попадания у слов: {} — дело может быть в форме слова, а не в отсутствии знания (попробуй {stem}*)",
            self.unmatched_terms.join(", ")
        ))
    }
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
pub fn search_ranked(conn: &Connection, query: &str, limit: usize) -> Result<SearchOutcome> {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return Ok(SearchOutcome {
            nodes: get_recent_nodes(conn, limit)?,
            terms: Vec::new(),
            unmatched_terms: Vec::new(),
        });
    }
    // Строка от человека — текст, а не выражение FTS5 (см. crate::fts).
    let parsed = crate::fts::parse(trimmed);
    if parsed.expr.is_empty() {
        return Ok(SearchOutcome {
            nodes: get_recent_nodes(conn, limit)?,
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

    rank_by_matched_terms(&mut nodes, &parsed.terms);
    nodes.truncate(limit);

    Ok(SearchOutcome {
        nodes,
        unmatched_terms: unmatched_terms(conn, &parsed.terms)?,
        terms: parsed.terms,
    })
}

/// Сколько слов запроса встретилось в тексте узла.
fn matched_count(node: &Node, terms: &[String]) -> usize {
    let mut hay = node.label.to_lowercase();
    if let Some(note) = &node.note {
        hay.push(' ');
        hay.push_str(&note.to_lowercase());
    }
    terms
        .iter()
        .filter(|t| hay.contains(&t.to_lowercase()))
        .count()
}

/// Больше совпавших слов — выше. Сортировка устойчивая, поэтому внутри одной
/// группы сохраняется порядок bm25, пришедший из SQL.
fn rank_by_matched_terms(nodes: &mut [Node], terms: &[String]) {
    if terms.len() < 2 {
        return;
    }
    nodes.sort_by_key(|n| std::cmp::Reverse(matched_count(n, terms)));
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
        let nodes = stmt
            .query_map(params![type_str, limit as i64], row_to_node)?
            .collect::<Result<Vec<_>, _>>()?;
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
                .is_some_and(|d| d.contains("форме слова")),
            "часть слов не нашлась — это про форму, а не про отсутствие знания"
        );

        let clean = search_ranked(&conn, "телеграм", 5).expect("поиск");
        assert!(
            clean.unmatched_terms.is_empty(),
            "у сработавшего запроса жаловаться не на что"
        );
        assert!(clean.diagnosis().is_none());

        // Третье состояние: не нашлось НИ ОДНОГО слова. Это уже не форма
        // запроса, а отсутствие знания, и путать их дорого — первое означает
        // «спроси иначе», второе «иди и выясняй».
        let nothing = search_ranked(&conn, "зурбаган лисс", 5).expect("поиск");
        assert!(
            nothing
                .diagnosis()
                .is_some_and(|d| d.contains("просто нет")),
            "ни одного знакомого слова — знания нет, а не форма подвела"
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
