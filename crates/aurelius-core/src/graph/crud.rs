use crate::identity;
use crate::models::{Edge, MemoryKind, Node, NodeType, Relation};
use crate::provenance::Provenance;
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::{row_to_edge, row_to_node};

pub fn add_node(
    conn: &Connection,
    node_type: NodeType,
    label: &str,
    note: Option<&str>,
    source: &str,
    data: serde_json::Value,
) -> Result<Node> {
    add_node_full(
        conn,
        node_type,
        label,
        note,
        source,
        data,
        MemoryKind::Semantic,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn add_node_full(
    conn: &Connection,
    node_type: NodeType,
    label: &str,
    note: Option<&str>,
    source: &str,
    data: serde_json::Value,
    memory_kind: MemoryKind,
    content_hash: Option<&str>,
) -> Result<Node> {
    let now = Utc::now();
    let author = identity::current().map(|i| i.as_author());
    let node = Node {
        id: Uuid::new_v4(),
        node_type,
        label: label.to_owned(),
        note: note.map(str::to_owned),
        source: source.to_owned(),
        data,
        created_at: now,
        updated_at: now,
        memory_kind,
        last_accessed_at: now,
        access_count: 0,
        content_hash: content_hash.map(str::to_owned),
        created_by: author.clone(),
        updated_by: author,
        deleted_at: None,
        sync_seq: None,
    };
    conn.execute(
        "INSERT INTO nodes (id, node_type, label, note, source, data, created_at, updated_at, memory_kind, last_accessed_at, access_count, content_hash, created_by, updated_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            node.id.to_string(),
            serde_json::to_string(&node.node_type)?,
            node.label,
            node.note,
            node.source,
            serde_json::to_string(&node.data)?,
            node.created_at.to_rfc3339(),
            node.updated_at.to_rfc3339(),
            node.memory_kind.to_string(),
            node.last_accessed_at.to_rfc3339(),
            node.access_count,
            node.content_hash,
            node.created_by,
            node.updated_by,
        ],
    )?;
    // The one place every record insert passes through, from both binaries:
    // `add_node`, `upsert_node_by_key` and `record_session` all land here, and
    // the MCP handlers call the same functions the CLI does. Marking here is
    // what keeps the marker honest for memory written over MCP, which no hook
    // and no CLI wrapper can observe.
    crate::db::mark_write(conn);
    // Queue the new node for embedding instead of computing one here: the
    // daemon is the only process allowed to hold bge-m3 in memory (spec
    // `011-dense-retrieval`, phase D), and this call site runs from
    // short-lived processes (`au note` is a fresh process per invocation,
    // and a hook fires one per file edit) where loading ~2.2 GB of weights
    // per write would turn a sub-10ms insert into seconds. `INSERT OR
    // REPLACE` on the `node_id` primary key: this row is always freshly
    // created here, so there is nothing to preserve, but the same statement
    // is what keeps a node re-queued elsewhere from ever duplicating.
    conn.execute(
        "INSERT OR REPLACE INTO embedding_queue (node_id, queued_at, attempts) VALUES (?1, ?2, 0)",
        params![node.id.to_string(), now.to_rfc3339()],
    )?;
    Ok(node)
}

/// Идемпотентная запись под пользовательским ключом, который ложится в
/// `data.key`. Повторный вызов с тем же ключом переписывает узел целиком
/// вместо того, чтобы завести близнеца — так хук, сработавший дважды за один
/// повод (авто- и ручная компакция), оставляет одну запись, а не две.
///
/// `expected_type` — граница между машинным и человеческим вызывающим.
/// `find_node_by_data_field` ищет совпадение только по значению ключа, без
/// фильтра по типу или источнику, поэтому найденный узел может оказаться
/// записью совсем другого автора. `Some(t)` — вызывающий знает свой тип как
/// константу (`link_evidence_run`, `record_session`) и не ждёт здесь ничего,
/// кроме него: узел другого типа — не близнец, а чужая запись, и это ошибка,
/// называющая оба типа, без переписи. `None` — вызывающий сам назвал тип
/// явным флагом (`au note --type`) и подмена для него легитимна; тогда
/// разница просто обязана быть видна вызывающему — отсюда третий элемент
/// возврата.
///
/// Второй элемент — `true`, если узел создан, `false`, если обновлён.
/// Третий — узел, каким он был ДО перезаписи (`None` при создании): его берёт
/// уже прочитанный `existing`, а не повторный запрос после `UPDATE` — между
/// чтением и записью есть гонка, и раздача читателю честного «было» до того,
/// как поле исчезнет под записью, часть того же исправления, что и сам гард.
/// `data` принимается объектом, а не любым `Value`: ключ должен быть куда
/// положить, иначе следующий вызов не нашёл бы запись и молча создал вторую.
#[allow(clippy::too_many_arguments)]
pub fn upsert_node_by_key(
    conn: &Connection,
    key: &str,
    node_type: NodeType,
    expected_type: Option<NodeType>,
    label: &str,
    note: Option<&str>,
    source: &str,
    mut data: serde_json::Map<String, serde_json::Value>,
    memory_kind: MemoryKind,
) -> Result<(Node, bool, Option<Node>)> {
    data.insert("key".to_owned(), serde_json::Value::String(key.to_owned()));
    let data = serde_json::Value::Object(data);

    let Some(existing) = find_node_by_data_field(conn, "key", key)? else {
        let node = add_node_full(
            conn,
            node_type,
            label,
            note,
            source,
            data,
            memory_kind,
            None,
        )?;
        return Ok((node, true, None));
    };

    if let Some(expected) = &expected_type {
        let found_kind = format!("{:?}", existing.node_type).to_lowercase();
        let expected_kind = format!("{:?}", expected).to_lowercase();
        if found_kind != expected_kind {
            anyhow::bail!(
                "ключ '{key}' уже занят узлом {} типа {found_kind}, а не {expected_kind} — \
                 отказ вместо молчаливой подмены чужой записи",
                existing.id,
            );
        }
    }

    let now = Utc::now();
    let author = identity::current().map(|i| i.as_author());
    // ВНИМАНИЕ: тот же UPDATE переписывает и `source` — совпадение типов не
    // защищает от смены источника. Человек, переписавший ключ машинного узла
    // тем же типом, молча меняет `source` с машинного на `"manual"`, а
    // источник — то поле, по которому потом меряют состав графа. Отдельный
    // дефект, вне этой задачи.
    conn.execute(
        "UPDATE nodes SET node_type = ?1, label = ?2, note = ?3, source = ?4, data = ?5,
                memory_kind = ?6, updated_at = ?7, updated_by = ?8
         WHERE id = ?9",
        params![
            serde_json::to_string(&node_type)?,
            label,
            note,
            source,
            serde_json::to_string(&data)?,
            memory_kind.to_string(),
            now.to_rfc3339(),
            author,
            existing.id.to_string(),
        ],
    )?;
    let updated = get_node(conn, &existing.id.to_string())?
        .ok_or_else(|| anyhow::anyhow!("узел {} исчез между поиском и обновлением", existing.id))?;
    Ok((updated, false, Some(existing)))
}

pub fn add_edge(
    conn: &Connection,
    from_id: Uuid,
    to_id: Uuid,
    relation: Relation,
    weight: f32,
) -> Result<Edge> {
    let edge = Edge {
        id: Uuid::new_v4(),
        from_id,
        to_id,
        relation,
        weight,
        created_at: Utc::now(),
        created_by: identity::current().map(|i| i.as_author()),
        deleted_at: None,
        sync_seq: None,
    };
    conn.execute(
        "INSERT OR IGNORE INTO edges (id, from_id, to_id, relation, weight, created_at, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            edge.id.to_string(),
            edge.from_id.to_string(),
            edge.to_id.to_string(),
            edge.relation.to_string(),
            edge.weight,
            edge.created_at.to_rfc3339(),
            edge.created_by,
        ],
    )?;
    Ok(edge)
}

/// Найти уже существующее ребро. `add_edge` вставляет через `OR IGNORE`, то
/// есть повтор молча ничего не делает и всё равно возвращает свежий `Edge` —
/// вызывающему, который сообщает пользователю «связь создана», нужен способ
/// отличить новое ребро от уже бывшего.
pub fn find_edge(
    conn: &Connection,
    from_id: Uuid,
    to_id: Uuid,
    relation: &Relation,
) -> Result<Option<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq
         FROM edges
         WHERE from_id = ?1 AND to_id = ?2 AND relation = ?3 AND deleted_at IS NULL",
    )?;
    let mut rows = stmt.query_map(
        params![from_id.to_string(), to_id.to_string(), relation.to_string()],
        row_to_edge,
    )?;
    Ok(rows.next().transpose()?)
}

pub fn update_node(
    conn: &Connection,
    id: Uuid,
    note: Option<&str>,
    data: Option<serde_json::Value>,
) -> Result<bool> {
    let now = Utc::now();
    let author = identity::current().map(|i| i.as_author());
    let mut updates = vec!["updated_at = ?1".to_string(), "updated_by = ?2".to_string()];
    let mut param_idx = 3;

    if note.is_some() {
        updates.push(format!("note = ?{param_idx}"));
        param_idx += 1;
    }
    if data.is_some() {
        updates.push(format!("data = ?{param_idx}"));
        param_idx += 1;
    }
    let _ = param_idx;

    let sql = format!(
        "UPDATE nodes SET {} WHERE id = ?{}",
        updates.join(", "),
        updates.len() + 1
    );

    let now_str = now.to_rfc3339();
    let id_str = id.to_string();
    let note_str = note.map(str::to_owned);
    let data_str = data.map(|d| serde_json::to_string(&d).unwrap_or_default());

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(now_str), Box::new(author)];
    if let Some(n) = note_str {
        param_values.push(Box::new(n));
    }
    if let Some(d) = data_str {
        param_values.push(Box::new(d));
    }
    param_values.push(Box::new(id_str));

    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let affected = conn.execute(&sql, params.as_slice())?;
    Ok(affected > 0)
}

/// Rename a node — the one column [`update_node`] cannot reach.
///
/// A task title is not stored in `data`: it lives in the `label` column as
/// `[{project}] {title}`, and until `au task update --title` there was no
/// writer for it at all. Widening [`update_node`] with a `label` parameter
/// would have touched all sixteen of its call sites for the benefit of one,
/// so the column gets its own writer instead — same `updated_at`/`updated_by`
/// stamping, because `sync::merge::apply_push` picks a winner by comparing
/// `updated_at` and a rename that left it alone would lose to the server's
/// older copy.
pub fn update_node_label(conn: &Connection, id: Uuid, label: &str) -> Result<bool> {
    let now_str = Utc::now().to_rfc3339();
    let author = identity::current().map(|i| i.as_author());
    let affected = conn.execute(
        "UPDATE nodes SET label = ?1, updated_at = ?2, updated_by = ?3
         WHERE id = ?4 AND deleted_at IS NULL",
        params![label, now_str, author, id.to_string()],
    )?;
    Ok(affected > 0)
}

/// Soft-delete: sets `deleted_at` (rather than issuing a `DELETE`) so the
/// tombstone can propagate through sync, and cascades the same timestamp
/// onto the node's edges instead of deleting them. Also bumps `updated_at`
/// (and stamps `updated_by`) to the same instant: `sync::merge::apply_push`
/// picks a winner by comparing `updated_at`, so without this a delete of a
/// node whose `updated_at` hasn't otherwise changed since its last push
/// would lose to the server's existing "live" copy instead of propagating.
pub fn delete_node(conn: &Connection, id: Uuid) -> Result<bool> {
    let now_str = Utc::now().to_rfc3339();
    let id_str = id.to_string();
    let author = identity::current().map(|i| i.as_author());
    let affected = conn.execute(
        "UPDATE nodes SET deleted_at = ?1, updated_at = ?1, updated_by = ?2
         WHERE id = ?3 AND deleted_at IS NULL",
        params![now_str, author, id_str],
    )?;
    if affected > 0 {
        conn.execute(
            "UPDATE edges SET deleted_at = ?1
             WHERE (from_id = ?2 OR to_id = ?2) AND deleted_at IS NULL",
            params![now_str, id_str],
        )?;
        drop_node_vector(conn, &id_str)?;
    }
    Ok(affected > 0)
}

/// Whether the vec0 table `node_embeddings` exists: old fixtures and
/// databases below schema v16 have none. Local twin of the private check in
/// `search.rs`.
fn node_embeddings_exist(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'node_embeddings')",
        [],
        |row| row.get(0),
    )?)
}

/// Remove a node's vector and its pending embedding job. Called wherever a
/// node becomes deleted: `dense_search` takes the KNN top k before filtering
/// `deleted_at`, so a dead vector steals a slot from a live node, and a
/// vector keyed by a freed rowid could later attach to an unrelated node.
pub fn drop_node_vector(conn: &Connection, id_str: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM embedding_queue WHERE node_id = ?1",
        params![id_str],
    )?;
    if !node_embeddings_exist(conn)? {
        return Ok(());
    }
    let rowid: Option<i64> = conn
        .query_row(
            "SELECT rowid FROM nodes WHERE id = ?1",
            params![id_str],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(rowid) = rowid {
        conn.execute(
            "DELETE FROM node_embeddings WHERE rowid = ?1",
            params![rowid],
        )?;
    }
    Ok(())
}

/// Remove every vector whose node is soft-deleted or gone, returning how many
/// were removed. One-off cleanup for vectors orphaned before `delete_node`
/// started dropping them. Rowids are collected first and deleted one by one:
/// vec0 handles point deletes by rowid, not arbitrary WHERE clauses.
pub fn purge_orphan_embeddings(conn: &Connection) -> Result<usize> {
    if !node_embeddings_exist(conn)? {
        return Ok(0);
    }
    let orphans: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT e.rowid FROM node_embeddings e
              LEFT JOIN nodes n ON n.rowid = e.rowid
              WHERE n.rowid IS NULL OR n.deleted_at IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for rowid in &orphans {
        conn.execute(
            "DELETE FROM node_embeddings WHERE rowid = ?1",
            params![rowid],
        )?;
    }
    Ok(orphans.len())
}

#[derive(Debug, Default)]
pub struct MergeStats {
    pub edges_rewired: usize,
    pub self_loops_removed: usize,
    pub duplicate_edges_removed: usize,
    pub note_merged: bool,
}

/// Merge `source` into `target`: rewires all edges from/to source onto target,
/// removes self-loops and duplicate edges created by the rewire, optionally
/// appends source's note to target's, then deletes source.
pub fn merge_nodes(conn: &Connection, source: Uuid, target: Uuid) -> Result<MergeStats> {
    if source == target {
        anyhow::bail!("source and target are the same node");
    }
    let src_str = source.to_string();
    let tgt_str = target.to_string();

    let src_node = get_node(conn, &src_str)?
        .ok_or_else(|| anyhow::anyhow!("source node not found: {src_str}"))?;
    let tgt_node = get_node(conn, &tgt_str)?
        .ok_or_else(|| anyhow::anyhow!("target node not found: {tgt_str}"))?;

    let mut stats = MergeStats::default();

    // `OR IGNORE`, not plain `UPDATE`: `idx_edges_unique` is a total index
    // over `(from_id, to_id, relation)`, blind to `deleted_at`, so rewiring
    // an edge onto a triple the target already carries is not a rare corner
    // case — two facts about the same project both carry a `belongs_to` edge
    // to that project, so any merge of two such facts collides on the very
    // first edge. A plain `UPDATE` aborts the whole statement on that
    // constraint violation (rolling the merge back and leaving it unusable
    // for exactly the case it exists for); `OR IGNORE` skips only the
    // colliding row and keeps the target's existing edge, which is the
    // correct dedup outcome — target and source agreed on that edge, so
    // there is nothing to add. The skipped source row stays pointed at
    // `source` and is soft-deleted along with it by `delete_node` below, so
    // it does not linger as an orphan.
    let rewired_from = conn.execute(
        "UPDATE OR IGNORE edges SET from_id = ?1 WHERE from_id = ?2",
        params![&tgt_str, &src_str],
    )?;
    let rewired_to = conn.execute(
        "UPDATE OR IGNORE edges SET to_id = ?1 WHERE to_id = ?2",
        params![&tgt_str, &src_str],
    )?;
    stats.edges_rewired = rewired_from + rewired_to;

    stats.self_loops_removed = conn.execute("DELETE FROM edges WHERE from_id = to_id", [])?;

    stats.duplicate_edges_removed = conn.execute(
        "DELETE FROM edges WHERE id NOT IN (
            SELECT MIN(id) FROM edges GROUP BY from_id, to_id, relation
        )",
        [],
    )?;

    // Merge notes: append source note to target if target has none, or both present
    if let Some(src_note) = src_node.note.as_deref() {
        let merged = match tgt_node.note.as_deref() {
            None => Some(src_note.to_owned()),
            Some(tgt_note) if !tgt_note.contains(src_note) => {
                Some(format!("{tgt_note}\n\n---\n{src_note}"))
            }
            _ => None,
        };
        if let Some(note) = merged {
            update_node(conn, target, Some(&note), None)?;
            stats.note_merged = true;
        }
    }

    delete_node(conn, source)?;
    Ok(stats)
}

pub fn touch_node(conn: &Connection, id: Uuid) -> Result<()> {
    let now = Utc::now();
    conn.execute(
        "UPDATE nodes SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
        params![now.to_rfc3339(), id.to_string()],
    )?;
    Ok(())
}

pub fn get_node(conn: &Connection, id: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE id = ?1 AND deleted_at IS NULL",
    )?;
    let mut rows = stmt.query_map(params![id], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn find_node_by_label(conn: &Connection, label: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE label = ?1 AND deleted_at IS NULL",
    )?;
    let mut rows = stmt.query_map(params![label], row_to_node)?;
    Ok(rows.next().transpose()?)
}

/// Живой узел по уникальному префиксу его `id` (не короче 8 hex-символов).
///
/// Правило живёт здесь, а не в CLI/MCP по отдельности, ровно затем, чтобы
/// `au task …` и `resolve_task_node` (MCP) не разошлись в том, что считать
/// однозначным префиксом: два независимых порога "восемь символов" рано или
/// поздно перестанут совпадать.
///
/// Короче 8 символов или с посторонним символом (не hex-цифра, не дефис) —
/// `Ok(None)` без обращения к базе: это заведомо не полный и не уникальный
/// префикс id, а не "искали и не нашли". Одно совпадение — `Ok(Some(node))`.
/// Два и больше — ошибка с обоими id-кандидатами: тихо выбрать первый
/// попавшийся значило бы иногда молча промахнуться мимо нужного узла.
pub fn find_node_by_id_prefix(conn: &Connection, prefix: &str) -> Result<Option<Node>> {
    if prefix.len() < 8 || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE id LIKE ?1 || '%' AND deleted_at IS NULL LIMIT 2",
    )?;
    let candidates = stmt
        .query_map(params![prefix], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.into_iter().next()),
        _ => {
            anyhow::bail!(
                "ambiguous id prefix '{prefix}': matches {} and {} — use more characters",
                candidates[0].id,
                candidates[1].id
            )
        }
    }
}

/// Рабочий каталог проекта — из первого его узла, у которого путь вообще
/// записан. Отдельный запрос, а не `find_project_by_label(...).data.path`:
/// узлов проекта с одной меткой в живой базе несколько (индексатор заводит
/// свой, автосоздание при заведении задачи — свой), и путь есть не у всех.
/// Взяв первый попавшийся, мы получали бы каталог то есть, то нет — в
/// зависимости от порядка строк, а от него зависит, соберутся ли файлы в
/// способ решения.
pub fn find_project_path(conn: &Connection, label: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT json_extract(data,'$.path') AS path
           FROM nodes
          WHERE label = ?1 AND (node_type = 'project' OR node_type = '\"project\"')
            AND deleted_at IS NULL AND path IS NOT NULL AND path != ''
          ORDER BY created_at
          LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![label], |row| row.get::<_, String>(0))?;
    Ok(rows.next().transpose()?)
}

pub fn find_project_by_label(conn: &Connection, label: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE label = ?1 AND (node_type = 'project' OR node_type = '\"project\"')
           AND deleted_at IS NULL",
    )?;
    let mut rows = stmt.query_map(params![label], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn find_node_by_content_hash(conn: &Connection, hash: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE content_hash = ?1 AND deleted_at IS NULL",
    )?;
    let mut rows = stmt.query_map(params![hash], row_to_node)?;
    Ok(rows.next().transpose()?)
}

pub fn find_node_by_data_field(conn: &Connection, key: &str, value: &str) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE json_extract(data, ?1) = ?2 AND deleted_at IS NULL",
    )?;
    let json_path = format!("$.{key}");
    let mut rows = stmt.query_map(params![json_path, value], row_to_node)?;
    Ok(rows.next().transpose()?)
}

/// Все живые узлы с этим значением поля `data`, свежие первыми.
///
/// Множественная форма нужна ровно для одного: обнаружения противоречий. Узел с
/// тем же `subject` — не «дубль, который можно проигнорировать», а второе
/// утверждение о том же предмете, и вызывающему надо показать их ВСЕ, чтобы он
/// решил, что с ними делать.
pub fn find_nodes_by_data_field(
    conn: &Connection,
    key: &str,
    value: &str,
    limit: usize,
) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE json_extract(data, ?1) = ?2 AND deleted_at IS NULL
         ORDER BY created_at DESC LIMIT ?3",
    )?;
    let json_path = format!("$.{key}");
    let nodes = stmt
        .query_map(params![json_path, value, limit as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Одна семья фасета: все живые узлы с одним и тем же `subject`, сведённые в
/// count плюс самый свежий член. `au recall --prefix` группирует по точному
/// значению `subject`, а не отдаёт голый список узлов — иначе повторяющийся
/// subject (пример из задачи: `xhub:bank131:refunds`, несколько записей за
/// август) шумел бы в выдаче копиями вместо одной строки с count.
#[derive(Debug)]
pub struct SubjectFamily {
    pub subject: String,
    pub count: usize,
    pub newest: Node,
}

/// Живые узлы, чей `subject` начинается с `prefix`, сгруппированные в семьи.
///
/// Диапазонная форма условия — измерено на живой базе (15419 строк, 2568 с
/// subject, 2238 различных): `LIKE 'prefix%'` и `GLOB 'prefix*'` обе НЕ
/// используют партиальный индекс `idx_nodes_subject`
/// (`ON nodes(json_extract(data,'$.subject')) WHERE json_extract(data,'$.subject')
/// IS NOT NULL`) и уходят в скан, а форма `>= prefix AND < prefix ||
/// char(1114111)` — используют: `EXPLAIN QUERY PLAN` подтверждает
/// `SEARCH nodes USING INDEX idx_nodes_subject`. `char(1114111)` — верхняя
/// граница Unicode (U+10FFFF); она делает верхнюю границу диапазона правильной
/// для любого префикса без ручного инкремента байтов.
///
/// `deleted_at IS NULL` в том же WHERE всё равно ставится: без него мёртвые
/// узлы просочились бы в выдачу. Измерено, что из-за этого условия
/// планировщик (нет статистики ANALYZE) переключается на
/// `idx_nodes_deleted_at`, посчитав равенство более избирательным — это не
/// так (живых узлов почти все 15419), но скан такого масштаба стоит
/// миллисекунды. `INDEXED BY` не ставится: это стало бы жёсткой ошибкой,
/// если индекс когда-нибудь переименуют.
///
/// Порядок строк из SQL — по subject, внутри subject свежие первыми; это даёт
/// готовую границу группы и готового «самого свежего» без второго запроса на
/// семью. Порядок результата — по свежести самого нового члена семьи, тоже
/// новый первым: это контракт команды, а не порядок группировки в SQL.
pub fn find_subject_families_by_prefix(
    conn: &Connection,
    prefix: &str,
) -> Result<Vec<SubjectFamily>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes
         WHERE json_extract(data, '$.subject') >= ?1
           AND json_extract(data, '$.subject') < ?1 || char(1114111)
           AND deleted_at IS NULL
         ORDER BY json_extract(data, '$.subject') ASC, created_at DESC",
    )?;
    let nodes = stmt
        .query_map(params![prefix], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;

    let mut families: Vec<SubjectFamily> = Vec::new();
    for node in nodes {
        // Гарантировано WHERE выше (сравнение с NULL ложно), но
        // `Provenance::from_data` читается снисходительно и без паники —
        // узел без подписи молча пропускается, а не рушит всю выдачу.
        let Some(subject) = Provenance::from_data(&node.data).subject else {
            continue;
        };
        match families.last_mut() {
            Some(family) if family.subject == subject => family.count += 1,
            _ => families.push(SubjectFamily {
                subject,
                count: 1,
                newest: node,
            }),
        }
    }

    families.sort_by_key(|f| std::cmp::Reverse(f.newest.created_at));
    Ok(families)
}

pub fn get_all_nodes(conn: &Connection) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE deleted_at IS NULL ORDER BY created_at DESC",
    )?;
    let nodes = stmt
        .query_map([], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn get_all_edges(conn: &Connection) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq
         FROM edges WHERE deleted_at IS NULL ORDER BY created_at DESC",
    )?;
    let edges = stmt
        .query_map([], row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

pub fn get_nodes_paginated(conn: &Connection, offset: usize, limit: usize) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
    )?;
    let nodes = stmt
        .query_map(params![limit as i64, offset as i64], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

pub fn get_edges_paginated(conn: &Connection, offset: usize, limit: usize) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_id, to_id, relation, weight, created_at, created_by, deleted_at, sync_seq
         FROM edges WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
    )?;
    let edges = stmt
        .query_map(params![limit as i64, offset as i64], row_to_edge)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges)
}

pub fn count_nodes(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE deleted_at IS NULL",
        [],
        |r| r.get::<_, usize>(0),
    )?)
}

pub fn count_edges(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM edges WHERE deleted_at IS NULL",
        [],
        |r| r.get::<_, usize>(0),
    )?)
}

pub fn get_nodes_by_type(conn: &Connection, node_type: &NodeType) -> Result<Vec<Node>> {
    let type_str = serde_json::to_string(node_type)?;
    let mut stmt = conn.prepare(
        "SELECT id, node_type, label, note, source, data, created_at, updated_at,
                memory_kind, last_accessed_at, access_count, content_hash,
                created_by, updated_by, deleted_at, sync_seq
         FROM nodes WHERE node_type = ?1 AND deleted_at IS NULL ORDER BY created_at DESC",
    )?;
    let nodes = stmt
        .query_map(params![type_str], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes)
}

/// Проект записи, когда его не назвали: префикс метки `[проект]`, иначе
/// git-репозиторий каталога `cwd` — то же каноническое имя, что у
/// `hook_project` (рабочее дерево носит имя основного репозитория). Голое имя
/// каталога без git проектом не считается: из `/tmp` или домашнего каталога
/// иначе рождались бы проекты, которых никто не называл.
pub fn infer_project(
    conn: &Connection,
    label: &str,
    cwd: Option<&std::path::Path>,
) -> Option<String> {
    if let Some((project, _)) = label
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
    {
        if !project.is_empty() {
            return Some(project.to_owned());
        }
    }
    crate::git::locate(conn, cwd, None).map(|repo| repo.name)
}

/// Что досталось записи при привязке ([`attach_on_write`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Attached {
    /// Проект, к узлу которого поставлено ребро `belongs_to`.
    pub project: Option<String>,
    /// Прежняя запись того же `subject`, с которой поставлено `related_to`.
    pub subject_peer: Option<Uuid>,
}

/// Привязать только что записанное знание в момент записи: ребро
/// `belongs_to` к узлу проекта и `related_to` к самой свежей прежней записи
/// того же `subject`. Совет «свяжи через `au relate`» не исполнялся: на
/// 19.09.2026 без единого ребра лежали 182 решения, 138 понятий, 103 решения
/// проблем и 75 проблем, почти все — записи без `project`.
///
/// Узел проекта здесь не заводится — только находится: пустая заглушка
/// проекта сама по себе мусор. С записью того же предмета, уже связанной
/// (ребро `supersedes`/`refines` от разрешения противоречия), второе ребро не
/// ставится. Всё fail-soft: узел к этому моменту уже записан, и отказ
/// привязки не имеет права выглядеть отказом записи — непривязанное просто
/// не попадает в ответ.
pub fn attach_on_write(conn: &Connection, node: &Node, project: Option<&str>) -> Attached {
    let project = project.and_then(|name| {
        let hub = find_project_by_label(conn, name).ok().flatten()?;
        if hub.id == node.id {
            return None;
        }
        add_edge(conn, node.id, hub.id, Relation::BelongsTo, 1.0).ok()?;
        Some(name.to_owned())
    });
    let subject_peer = Provenance::from_data(&node.data)
        .subject
        .and_then(|subject| {
            // Свежие первыми; сам узел — самый свежий, поэтому двух хватает.
            find_nodes_by_data_field(conn, crate::provenance::SUBJECT_KEY, &subject, 2).ok()
        })
        .and_then(|peers| peers.into_iter().find(|n| n.id != node.id))
        .and_then(|peer| {
            let linked: bool = conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM edges WHERE deleted_at IS NULL
                        AND ((from_id = ?1 AND to_id = ?2) OR (from_id = ?2 AND to_id = ?1)))",
                    params![node.id.to_string(), peer.id.to_string()],
                    |r| r.get(0),
                )
                .ok()?;
            if linked {
                return None;
            }
            add_edge(conn, node.id, peer.id, Relation::RelatedTo, 1.0).ok()?;
            Some(peer.id)
        });
    Attached {
        project,
        subject_peer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    /// A real temp-file database, not `:memory:` — SQLite's in-memory mode
    /// can never report journal_mode=WAL, and `db::open`'s `ensure_wal` gate
    /// now hard-rejects anything else. Kept alive for the test's duration so
    /// the file isn't removed out from under the open `Connection`; cleans
    /// up its `-wal`/`-shm` siblings on drop.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-crud-test-{tag}-{}.db",
                uuid::Uuid::new_v4()
            )))
        }
    }

    impl Drop for TmpDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.as_os_str().to_owned();
                p.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(p));
            }
        }
    }

    fn setup() -> (TmpDb, Connection) {
        let tmp = TmpDb::new("setup");
        let conn = db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    /// T025: `memory_forget` (MCP) and any future CLI equivalent both go
    /// through this function. `sync::merge::apply_push` picks a winner by
    /// comparing `updated_at`, so a delete that leaves `updated_at`
    /// unchanged from creation would silently lose to the server's existing
    /// live copy on the next push instead of propagating (see doc comment
    /// on `delete_node`).
    #[test]
    fn delete_node_bumps_updated_at_in_lockstep_with_deleted_at() {
        let (_tmp, conn) = setup();
        let node = add_node(
            &conn,
            NodeType::Decision,
            "obsolete decision",
            Some("no longer relevant"),
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let deleted = delete_node(&conn, node.id).expect("delete node");
        assert!(deleted);

        // get_node filters `deleted_at IS NULL`, so read the raw row.
        let (deleted_at, updated_at): (Option<String>, String) = conn
            .query_row(
                "SELECT deleted_at, updated_at FROM nodes WHERE id = ?1",
                params![node.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("row exists");

        assert!(deleted_at.is_some(), "deleted_at must be set");
        assert_eq!(
            deleted_at.as_deref(),
            Some(updated_at.as_str()),
            "updated_at must be bumped to the same instant as deleted_at, \
             so a subsequent push is recognized as the newest change"
        );

        let updated_at: chrono::DateTime<Utc> = updated_at.parse().expect("valid timestamp");
        assert!(
            updated_at >= node.updated_at,
            "updated_at must not move backward from the pre-delete value"
        );
    }

    /// PreCompact срабатывает и автоматически, и на ручной `/compact`. Без
    /// ключа это два узла-близнеца; с ключом — один, переписанный.
    #[test]
    fn upsert_by_key_updates_instead_of_duplicating() {
        let (_tmp, conn) = setup();
        let key = "precompact:session-42";

        let (first, created, replaced) = upsert_node_by_key(
            &conn,
            key,
            NodeType::Session,
            Some(NodeType::Session),
            "снимок перед компакцией",
            Some("первый заход"),
            "hook",
            serde_json::Map::new(),
            MemoryKind::Episodic,
        )
        .expect("first upsert");
        assert!(created, "первый вызов создаёт узел");
        assert!(replaced.is_none(), "создание не заменяет ничего");

        let (second, created, replaced) = upsert_node_by_key(
            &conn,
            key,
            NodeType::Session,
            Some(NodeType::Session),
            "снимок перед компакцией",
            Some("второй заход"),
            "hook",
            serde_json::Map::new(),
            MemoryKind::Episodic,
        )
        .expect("second upsert");
        assert!(!created, "второй вызов обновляет, а не создаёт");
        assert_eq!(first.id, second.id, "id должен остаться тем же");
        assert_eq!(
            replaced.map(|n| n.id),
            Some(first.id),
            "вызывающий обязан увидеть узел, каким он был до перезаписи"
        );
        assert_eq!(second.note.as_deref(), Some("второй заход"));
        assert_eq!(second.memory_kind, MemoryKind::Episodic);
        assert_eq!(
            second.data.get("key").and_then(|v| v.as_str()),
            Some(key),
            "ключ должен пережить обновление, иначе следующий вызов не найдёт узел"
        );

        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE json_extract(data, '$.key') = ?1
                   AND deleted_at IS NULL",
                params![key],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(total, 1, "близнец не должен появиться");
    }

    /// Ключ узла — не то же самое, что ключ его соседа: разные ключи должны
    /// оставаться разными узлами.
    #[test]
    fn upsert_by_key_keeps_distinct_keys_apart() {
        let (_tmp, conn) = setup();
        let (a, ..) = upsert_node_by_key(
            &conn,
            "snapshot:a",
            NodeType::Session,
            Some(NodeType::Session),
            "a",
            None,
            "hook",
            serde_json::Map::new(),
            MemoryKind::Episodic,
        )
        .expect("upsert a");
        let (b, created, _) = upsert_node_by_key(
            &conn,
            "snapshot:b",
            NodeType::Session,
            Some(NodeType::Session),
            "b",
            None,
            "hook",
            serde_json::Map::new(),
            MemoryKind::Episodic,
        )
        .expect("upsert b");
        assert!(created);
        assert_ne!(a.id, b.id);
    }

    /// Ядро дефекта: одноимённый ключ, заведённый под чужим типом, не должен
    /// молча превращаться в узел вызывающего. `Some(expected)` обязан
    /// отказать, назвав оба типа, и не трогать найденную запись.
    #[test]
    fn upsert_by_key_refuses_type_mismatch_when_expected() {
        let (_tmp, conn) = setup();
        let key = "run:xhub:refunds";

        let (note, ..) = upsert_node_by_key(
            &conn,
            key,
            NodeType::Decision,
            None,
            "заметка под чужим ключом",
            Some("текст"),
            "manual",
            serde_json::Map::new(),
            MemoryKind::Semantic,
        )
        .expect("human note");

        let err = upsert_node_by_key(
            &conn,
            key,
            NodeType::Run,
            Some(NodeType::Run),
            "прогон",
            None,
            "au-task-evidence",
            serde_json::Map::new(),
            MemoryKind::Semantic,
        )
        .expect_err("тип не совпал — обязан отказать, а не переписать чужой узел");
        let message = err.to_string();
        assert!(
            message.contains("decision") && message.contains("run"),
            "сообщение обязано назвать оба типа: {message}"
        );

        let untouched = get_node(&conn, &note.id.to_string())
            .expect("lookup")
            .expect("узел обязан остаться на месте");
        assert_eq!(
            untouched.note.as_deref(),
            Some("текст"),
            "отказ не должен трогать найденную запись"
        );
    }

    /// Человеческая граница: без ожидания подмена типа разрешена, а старый
    /// узел возвращается вызывающему целиком — печатать предупреждение не из
    /// чего, если это `None`.
    #[test]
    fn upsert_by_key_allows_type_change_when_no_expectation() {
        let (_tmp, conn) = setup();
        let key = "note:same-key";

        let (first, ..) = upsert_node_by_key(
            &conn,
            key,
            NodeType::Decision,
            None,
            "первая заметка",
            None,
            "manual",
            serde_json::Map::new(),
            MemoryKind::Semantic,
        )
        .expect("first");

        let (second, created, replaced) = upsert_node_by_key(
            &conn,
            key,
            NodeType::Concept,
            None,
            "вторая заметка, другой тип",
            None,
            "manual",
            serde_json::Map::new(),
            MemoryKind::Semantic,
        )
        .expect("second");

        assert!(!created);
        assert_eq!(first.id, second.id);
        let replaced = replaced.expect("должен вернуться узел, каким он был");
        assert!(matches!(replaced.node_type, NodeType::Decision));
        assert!(matches!(second.node_type, NodeType::Concept));
    }

    #[test]
    fn find_by_id_prefix_finds_the_node_by_first_eight_chars() {
        let (_tmp, conn) = setup();
        let node = add_node(
            &conn,
            NodeType::Task,
            "задача с длинным id",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let prefix = &node.id.to_string()[..8];
        let found = find_node_by_id_prefix(&conn, prefix)
            .expect("lookup")
            .expect("must find the node");
        assert_eq!(found.id, node.id);
    }

    #[test]
    fn find_by_id_prefix_rejects_prefix_shorter_than_eight_chars() {
        let (_tmp, conn) = setup();
        let node = add_node(
            &conn,
            NodeType::Task,
            "задача покороче",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node");

        let short_prefix = &node.id.to_string()[..7];
        let found = find_node_by_id_prefix(&conn, short_prefix).expect("lookup must not error");
        assert!(found.is_none(), "7 символов — не префикс, а недоввод");
    }

    #[test]
    fn find_by_id_prefix_rejects_non_hex_characters() {
        let (_tmp, conn) = setup();
        let found =
            find_node_by_id_prefix(&conn, "zzzzzzzz").expect("lookup must not touch the db");
        assert!(found.is_none(), "'z' — не hex-цифра и не дефис");
    }

    /// Реальные id генерируются случайно и общий префикс сами по себе не
    /// дадут — здесь он прописан вручную прямым `UPDATE`, чтобы проверить
    /// именно ветку неоднозначности, а не полагаться на удачу.
    #[test]
    fn find_by_id_prefix_errs_with_both_candidates_on_ambiguity() {
        let (_tmp, conn) = setup();
        let a = add_node(
            &conn,
            NodeType::Task,
            "первый кандидат",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node a");
        let b = add_node(
            &conn,
            NodeType::Task,
            "второй кандидат",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node b");

        let shared_id_a = "deadbeef-0000-4000-8000-000000000001";
        let shared_id_b = "deadbeef-0000-4000-8000-000000000002";
        // `add_node` queues both nodes for embedding, and `embedding_queue.node_id`
        // is `FOREIGN KEY ... REFERENCES nodes(id)` (db.rs, migrate_v18): rewriting
        // a node's id out from under a still-queued row would violate that
        // constraint. Irrelevant to what this test checks (prefix ambiguity, not
        // the queue), so the queue rows are cleared first rather than rewritten.
        conn.execute("DELETE FROM embedding_queue", [])
            .expect("clear embedding queue before id rewrite");
        conn.execute(
            "UPDATE nodes SET id = ?1 WHERE id = ?2",
            params![shared_id_a, a.id.to_string()],
        )
        .expect("rewrite id a");
        conn.execute(
            "UPDATE nodes SET id = ?1 WHERE id = ?2",
            params![shared_id_b, b.id.to_string()],
        )
        .expect("rewrite id b");

        let err = find_node_by_id_prefix(&conn, "deadbeef")
            .expect_err("два узла с общим префиксом обязаны дать ошибку");
        let message = err.to_string();
        assert!(
            message.contains(shared_id_a) && message.contains(shared_id_b),
            "сообщение должно называть оба id-кандидата: {message}"
        );
    }

    fn add_with_subject(conn: &Connection, subject: &str, claim: &str) -> Node {
        add_node(
            conn,
            NodeType::Concept,
            subject,
            None,
            "test",
            serde_json::json!({ "subject": subject, "claim": claim }),
        )
        .expect("add node with subject")
    }

    /// Три семьи под одним префиксом, count и «самый свежий» — на уровне SQL,
    /// до интеграционного теста CLI на `au recall --prefix`.
    #[test]
    fn find_subject_families_groups_counts_and_orders_by_newest_member() {
        let (_tmp, conn) = setup();

        add_with_subject(&conn, "xhub:bank131:refunds", "первая запись августа");
        let newest_refunds = add_with_subject(&conn, "xhub:bank131:refunds", "вторая, свежее");
        add_with_subject(
            &conn,
            "xhub:antifraud:research:data-layer",
            "заметка по антифроду",
        );
        // Другой фасет — не должен попасть в выдачу по префику "xhub:bank131".
        add_with_subject(&conn, "xhub:antifraud:other", "постороннее");

        let families =
            find_subject_families_by_prefix(&conn, "xhub:bank131").expect("lookup must not error");

        assert_eq!(families.len(), 1, "один subject под этим префиксом");
        assert_eq!(families[0].subject, "xhub:bank131:refunds");
        assert_eq!(families[0].count, 2, "count считает узлы, а не заявления");
        assert_eq!(
            families[0].newest.id, newest_refunds.id,
            "самый свежий член семьи — второй вставленный узел"
        );
    }

    /// Порядок результата — по свежести самого нового члена КАЖДОЙ семьи,
    /// а не по алфавиту subject (в котором их выдаёт группировка SQL).
    #[test]
    fn find_subject_families_orders_families_newest_first() {
        let (_tmp, conn) = setup();

        add_with_subject(&conn, "xhub:aaa:older", "давняя заметка");
        let newer = add_with_subject(&conn, "xhub:zzz:newer", "свежая заметка");

        let families =
            find_subject_families_by_prefix(&conn, "xhub:").expect("lookup must not error");

        assert_eq!(families.len(), 2);
        assert_eq!(
            families[0].newest.id, newer.id,
            "свежая семья обязана идти первой, хотя её subject алфавитно позже"
        );
    }

    #[test]
    fn find_subject_families_returns_empty_for_unmatched_prefix() {
        let (_tmp, conn) = setup();
        add_with_subject(&conn, "xhub:bank131:refunds", "не тот префикс");

        let families =
            find_subject_families_by_prefix(&conn, "xhub:refunds").expect("lookup must not error");
        assert!(
            families.is_empty(),
            "нет subject, начинающегося с xhub:refunds"
        );
    }

    /// Ядро дефекта: два факта об одном проекте оба несут `belongs_to` на тот
    /// же узел проекта — рефайр `source`'а сталкивается с уже существующим у
    /// `target` ребром той же тройки `(from, to, relation)` и раньше ронял
    /// `UPDATE` целиком через `idx_edges_unique`. Слияние обязано пережить
    /// это ровно на том случае, ради которого инструмент существует.
    #[test]
    fn merge_survives_a_shared_edge_and_keeps_the_union_without_duplicates_or_self_loops() {
        let (_tmp, conn) = setup();

        let project = add_node(
            &conn,
            NodeType::Project,
            "aurelius",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add project");
        let source = add_node(
            &conn,
            NodeType::Concept,
            "источник",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add source");
        let target = add_node(
            &conn,
            NodeType::Concept,
            "цель",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add target");

        // Общее ребро: обе записи принадлежат одному проекту — то, что раньше
        // ломало merge_nodes.
        add_edge(&conn, source.id, project.id, Relation::BelongsTo, 1.0)
            .expect("source belongs_to");
        add_edge(&conn, target.id, project.id, Relation::BelongsTo, 1.0)
            .expect("target belongs_to");
        // Ребро, уникальное для source — обязано переехать на target.
        let other = add_node(
            &conn,
            NodeType::Concept,
            "третий узел",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add other");
        add_edge(&conn, source.id, other.id, Relation::RelatedTo, 1.0).expect("source related_to");
        // Ребро source -> target напрямую: после рефайра from_id это станет
        // самопетлёй target -> target, которая не должна пережить merge.
        add_edge(&conn, source.id, target.id, Relation::RelatedTo, 1.0).expect("source -> target");

        let stats = merge_nodes(&conn, source.id, target.id).expect("merge must succeed");
        assert_eq!(
            stats.self_loops_removed, 1,
            "source -> target рефайрится в target -> target и обязан быть снят"
        );

        let edges = get_all_edges(&conn).expect("edges after merge");
        assert!(
            edges.iter().any(|e| e.from_id == target.id
                && e.to_id == other.id
                && e.relation.to_string() == Relation::RelatedTo.to_string()),
            "уникальное ребро source обязано переехать на target: {edges:?}"
        );
        assert!(
            edges.iter().any(|e| e.from_id == target.id
                && e.to_id == project.id
                && e.relation.to_string() == Relation::BelongsTo.to_string()),
            "belongs_to на проект обязан остаться на target: {edges:?}"
        );
        assert_eq!(
            edges.iter().filter(|e| e.to_id == project.id).count(),
            1,
            "belongs_to на проект не должен задвоиться: {edges:?}"
        );
        assert!(
            !edges
                .iter()
                .any(|e| e.from_id == target.id && e.to_id == target.id),
            "самопетля target -> target не должна пережить merge: {edges:?}"
        );

        assert!(
            get_node(&conn, &source.id.to_string())
                .expect("lookup source")
                .is_none(),
            "source обязан быть удалён после merge"
        );
    }

    fn knowledge(conn: &Connection, label: &str, subject: Option<&str>) -> Node {
        add_node(
            conn,
            NodeType::Decision,
            label,
            Some("тело"),
            "test",
            serde_json::json!({ "claim": label, "subject": subject }),
        )
        .expect("add knowledge")
    }

    #[test]
    fn attach_on_write_links_project_and_subject_peer() {
        let (_tmp, conn) = setup();
        let hub = add_node(
            &conn,
            NodeType::Project,
            "демо",
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add project");
        let old = knowledge(&conn, "старое", Some("демо:предмет"));
        let new = knowledge(&conn, "новое", Some("демо:предмет"));

        let attached = attach_on_write(&conn, &new, Some("демо"));

        assert_eq!(attached.project.as_deref(), Some("демо"));
        assert_eq!(attached.subject_peer, Some(old.id));
        assert!(find_edge(&conn, new.id, hub.id, &Relation::BelongsTo)
            .expect("edge lookup")
            .is_some());
        assert!(find_edge(&conn, new.id, old.id, &Relation::RelatedTo)
            .expect("edge lookup")
            .is_some());
    }

    /// Нет узла проекта — нет ребра, нет ошибки и нет новой заглушки проекта.
    #[test]
    fn attach_on_write_is_fail_soft_and_creates_no_project() {
        let (_tmp, conn) = setup();
        let node = knowledge(&conn, "одинокое", None);

        let attached = attach_on_write(&conn, &node, Some("нет-такого"));

        assert_eq!(attached, Attached::default());
        assert!(get_nodes_by_type(&conn, &NodeType::Project)
            .expect("projects")
            .is_empty());
    }

    /// Разрешённое противоречие уже связало записи — второе ребро не нужно.
    #[test]
    fn attach_on_write_skips_a_peer_already_linked() {
        let (_tmp, conn) = setup();
        let old = knowledge(&conn, "старое", Some("предмет"));
        let new = knowledge(&conn, "новое", Some("предмет"));
        add_edge(&conn, new.id, old.id, Relation::Supersedes, 1.0).expect("supersedes");

        let attached = attach_on_write(&conn, &new, None);

        assert_eq!(attached.subject_peer, None);
        assert!(find_edge(&conn, new.id, old.id, &Relation::RelatedTo)
            .expect("edge lookup")
            .is_none());
    }

    #[test]
    fn infer_project_takes_the_label_prefix_first() {
        let (_tmp, conn) = setup();
        assert_eq!(
            infer_project(&conn, "[xhub] лимит ретраев", None).as_deref(),
            Some("xhub")
        );
        assert_eq!(infer_project(&conn, "[] пустой префикс", None), None);
        assert_eq!(infer_project(&conn, "без префикса", None), None);
    }

    fn plain(conn: &Connection, label: &str) -> Node {
        add_node(
            conn,
            NodeType::Concept,
            label,
            None,
            "test",
            serde_json::json!({}),
        )
        .expect("add node")
    }

    fn put_vector(conn: &Connection, node: &Node) {
        let bytes: Vec<u8> = (0..1024u32)
            .flat_map(|i| ((i % 7) as f32 + 1.0).to_le_bytes())
            .collect();
        conn.execute(
            "INSERT INTO node_embeddings(rowid, embedding)
             SELECT rowid, vec_quantize_int8(?2, 'unit') FROM nodes WHERE id = ?1",
            params![node.id.to_string(), bytes],
        )
        .expect("insert vector");
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).expect("count")
    }

    #[test]
    fn delete_node_drops_its_vector_and_queue_row() {
        let (_tmp, conn) = setup();
        let keep = plain(&conn, "keep");
        let gone = plain(&conn, "gone");
        put_vector(&conn, &keep);
        put_vector(&conn, &gone);

        assert!(delete_node(&conn, gone.id).expect("delete"));

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM node_embeddings"), 1);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM nodes WHERE deleted_at IS NULL"),
            1
        );
        let queued: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embedding_queue WHERE node_id = ?1",
                params![gone.id.to_string()],
                |r| r.get(0),
            )
            .expect("queue count");
        assert_eq!(queued, 0);
        assert_eq!(purge_orphan_embeddings(&conn).expect("purge"), 0);
    }

    #[test]
    fn merge_leaves_no_orphan_vector() {
        let (_tmp, conn) = setup();
        let source = plain(&conn, "source");
        let target = plain(&conn, "target");
        put_vector(&conn, &source);
        put_vector(&conn, &target);

        merge_nodes(&conn, source.id, target.id).expect("merge");

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM node_embeddings"), 1);
        assert_eq!(purge_orphan_embeddings(&conn).expect("purge"), 0);
    }

    #[test]
    fn purge_removes_preexisting_orphans_once() {
        let (_tmp, conn) = setup();
        let live = plain(&conn, "live");
        let soft = plain(&conn, "soft");
        let hard = plain(&conn, "hard");
        for n in [&live, &soft, &hard] {
            put_vector(&conn, n);
        }
        // Orphans the old way: soft delete without touching vectors, and a
        // hard delete as `memory_gc` does it.
        conn.execute(
            "UPDATE nodes SET deleted_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), soft.id.to_string()],
        )
        .expect("soft delete");
        conn.execute(
            "DELETE FROM nodes WHERE id = ?1",
            params![hard.id.to_string()],
        )
        .expect("hard delete");

        assert_eq!(purge_orphan_embeddings(&conn).expect("purge"), 2);
        assert_eq!(purge_orphan_embeddings(&conn).expect("purge again"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM node_embeddings"), 1);
    }
}
