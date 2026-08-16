use crate::identity;
use crate::models::{Edge, MemoryKind, Node, NodeType, Relation};
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
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
    Ok(node)
}

/// Идемпотентная запись под пользовательским ключом, который ложится в
/// `data.key`. Повторный вызов с тем же ключом переписывает узел целиком
/// вместо того, чтобы завести близнеца — так хук, сработавший дважды за один
/// повод (авто- и ручная компакция), оставляет одну запись, а не две.
///
/// Второй элемент — `true`, если узел создан, `false`, если обновлён.
/// `data` принимается объектом, а не любым `Value`: ключ должен быть куда
/// положить, иначе следующий вызов не нашёл бы запись и молча создал вторую.
#[allow(clippy::too_many_arguments)]
pub fn upsert_node_by_key(
    conn: &Connection,
    key: &str,
    node_type: NodeType,
    label: &str,
    note: Option<&str>,
    source: &str,
    mut data: serde_json::Map<String, serde_json::Value>,
    memory_kind: MemoryKind,
) -> Result<(Node, bool)> {
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
        return Ok((node, true));
    };

    let now = Utc::now();
    let author = identity::current().map(|i| i.as_author());
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
    Ok((updated, false))
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
    }
    Ok(affected > 0)
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

    let rewired_from = conn.execute(
        "UPDATE edges SET from_id = ?1 WHERE from_id = ?2",
        params![&tgt_str, &src_str],
    )?;
    let rewired_to = conn.execute(
        "UPDATE edges SET to_id = ?1 WHERE to_id = ?2",
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

        let (first, created) = upsert_node_by_key(
            &conn,
            key,
            NodeType::Session,
            "снимок перед компакцией",
            Some("первый заход"),
            "hook",
            serde_json::Map::new(),
            MemoryKind::Episodic,
        )
        .expect("first upsert");
        assert!(created, "первый вызов создаёт узел");

        let (second, created) = upsert_node_by_key(
            &conn,
            key,
            NodeType::Session,
            "снимок перед компакцией",
            Some("второй заход"),
            "hook",
            serde_json::Map::new(),
            MemoryKind::Episodic,
        )
        .expect("second upsert");
        assert!(!created, "второй вызов обновляет, а не создаёт");
        assert_eq!(first.id, second.id, "id должен остаться тем же");
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
        let (a, _) = upsert_node_by_key(
            &conn,
            "snapshot:a",
            NodeType::Session,
            "a",
            None,
            "hook",
            serde_json::Map::new(),
            MemoryKind::Episodic,
        )
        .expect("upsert a");
        let (b, created) = upsert_node_by_key(
            &conn,
            "snapshot:b",
            NodeType::Session,
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
}
