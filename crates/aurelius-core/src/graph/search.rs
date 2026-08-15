use crate::models::{Node, NodeType};
use anyhow::Result;
use rusqlite::{params, Connection};

use super::row_to_node;

pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Node>> {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return get_recent_nodes(conn, limit);
    }
    let mut stmt = conn.prepare(
        "SELECT n.id, n.node_type, n.label, n.note, n.source, n.data, n.created_at, n.updated_at,
                n.memory_kind, n.last_accessed_at, n.access_count, n.content_hash,
                n.created_by, n.updated_by, n.deleted_at, n.sync_seq
         FROM nodes_fts
         JOIN nodes n ON nodes_fts.rowid = n.rowid
         WHERE nodes_fts MATCH ?1 AND n.deleted_at IS NULL
         ORDER BY rank + (n.access_count * 0.1) DESC
         LIMIT ?2",
    )?;
    let nodes = stmt
        .query_map(params![trimmed, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn search_typed(
    conn: &Connection,
    query: &str,
    node_type: &NodeType,
    limit: usize,
) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(node_type)?;
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed == "*" {
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
         LIMIT ?3",
    )?;
    let nodes = stmt
        .query_map(params![trimmed, type_str, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
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
