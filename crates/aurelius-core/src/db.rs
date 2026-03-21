use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    migrate(&conn)?;
    Ok(conn)
}

fn get_schema_version(conn: &Connection) -> i32 {
    conn.query_row(
        "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

fn set_schema_version(conn: &Connection, version: i32) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        params![version],
    )?;
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    // Create version tracking table first
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        );",
    )?;

    let current = get_schema_version(conn);

    if current < 1 {
        migrate_v1(conn)?;
        set_schema_version(conn, 1)?;
    }

    if current < 2 {
        migrate_v2(conn)?;
        set_schema_version(conn, 2)?;
    }

    if current < 3 {
        migrate_v3(conn)?;
        set_schema_version(conn, 3)?;
    }

    if current < 4 {
        migrate_v4(conn)?;
        set_schema_version(conn, 4)?;
    }

    if current < 5 {
        migrate_v5(conn)?;
        set_schema_version(conn, 5)?;
    }

    Ok(())
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS nodes (
            id          TEXT PRIMARY KEY,
            node_type   TEXT NOT NULL,
            label       TEXT NOT NULL,
            note        TEXT,
            source      TEXT NOT NULL DEFAULT 'manual',
            data        TEXT NOT NULL DEFAULT '{}',
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS edges (
            id          TEXT PRIMARY KEY,
            from_id     TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            to_id       TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            relation    TEXT NOT NULL,
            weight      REAL NOT NULL DEFAULT 1.0,
            created_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
        CREATE INDEX IF NOT EXISTS idx_edges_to   ON edges(to_id);

        CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
            label,
            note,
            data,
            content='nodes',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS nodes_ai AFTER INSERT ON nodes BEGIN
            INSERT INTO nodes_fts(rowid, label, note, data)
            VALUES (new.rowid, new.label, new.note, new.data);
        END;

        CREATE TRIGGER IF NOT EXISTS nodes_ad AFTER DELETE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note, data)
            VALUES ('delete', old.rowid, old.label, old.note, old.data);
        END;

        CREATE TRIGGER IF NOT EXISTS nodes_au AFTER UPDATE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note, data)
            VALUES ('delete', old.rowid, old.label, old.note, old.data);
            INSERT INTO nodes_fts(rowid, label, note, data)
            VALUES (new.rowid, new.label, new.note, new.data);
        END;
    ",
    )?;
    Ok(())
}

fn migrate_v2(conn: &Connection) -> Result<()> {
    // Add new columns for extended node metadata
    let columns = [
        "ALTER TABLE nodes ADD COLUMN memory_kind TEXT NOT NULL DEFAULT 'semantic'",
        "ALTER TABLE nodes ADD COLUMN last_accessed_at TEXT",
        "ALTER TABLE nodes ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE nodes ADD COLUMN content_hash TEXT",
    ];
    for sql in &columns {
        // ALTER TABLE ADD COLUMN IF NOT EXISTS not supported in SQLite,
        // so we silently ignore "duplicate column" errors
        match conn.execute(sql, []) {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => return Err(e.into()),
        }
    }

    // Backfill last_accessed_at from updated_at where NULL
    conn.execute(
        "UPDATE nodes SET last_accessed_at = updated_at WHERE last_accessed_at IS NULL",
        [],
    )?;

    Ok(())
}

fn migrate_v4(conn: &Connection) -> Result<()> {
    // Rebuild FTS5 without the `data` column — raw JSON creates search noise
    conn.execute_batch(
        "
        DROP TRIGGER IF EXISTS nodes_ai;
        DROP TRIGGER IF EXISTS nodes_ad;
        DROP TRIGGER IF EXISTS nodes_au;
        DROP TABLE IF EXISTS nodes_fts;

        CREATE VIRTUAL TABLE nodes_fts USING fts5(
            label, note,
            content='nodes',
            content_rowid='rowid'
        );

        CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN
            INSERT INTO nodes_fts(rowid, label, note)
            VALUES (new.rowid, new.label, new.note);
        END;

        CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note)
            VALUES ('delete', old.rowid, old.label, old.note);
        END;

        CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN
            INSERT INTO nodes_fts(nodes_fts, rowid, label, note)
            VALUES ('delete', old.rowid, old.label, old.note);
            INSERT INTO nodes_fts(rowid, label, note)
            VALUES (new.rowid, new.label, new.note);
        END;

        INSERT INTO nodes_fts(rowid, label, note)
        SELECT rowid, label, note FROM nodes;
    ",
    )?;
    Ok(())
}

fn migrate_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS search_cache (
            id          TEXT PRIMARY KEY,
            query       TEXT NOT NULL,
            results     TEXT NOT NULL DEFAULT '[]',
            source      TEXT NOT NULL DEFAULT 'brave',
            created_at  TEXT NOT NULL,
            expires_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_search_cache_query
            ON search_cache(query);

        CREATE INDEX IF NOT EXISTS idx_search_cache_expires
            ON search_cache(expires_at);

        CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
            query, results,
            content='search_cache',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS search_cache_ai AFTER INSERT ON search_cache BEGIN
            INSERT INTO search_fts(rowid, query, results)
            VALUES (new.rowid, new.query, new.results);
        END;

        CREATE TRIGGER IF NOT EXISTS search_cache_ad AFTER DELETE ON search_cache BEGIN
            INSERT INTO search_fts(search_fts, rowid, query, results)
            VALUES ('delete', old.rowid, old.query, old.results);
        END;

        CREATE TRIGGER IF NOT EXISTS search_cache_au AFTER UPDATE ON search_cache BEGIN
            INSERT INTO search_fts(search_fts, rowid, query, results)
            VALUES ('delete', old.rowid, old.query, old.results);
            INSERT INTO search_fts(rowid, query, results)
            VALUES (new.rowid, new.query, new.results);
        END;
    ",
    )?;
    Ok(())
}

fn migrate_v3(conn: &Connection) -> Result<()> {
    // Clean up duplicate edges BEFORE creating unique index
    conn.execute(
        "DELETE FROM edges WHERE id NOT IN (
            SELECT MIN(id) FROM edges GROUP BY from_id, to_id, relation
        )",
        [],
    )?;

    conn.execute_batch(
        "
        -- Edge dedup: prevent duplicate (from, to, relation) triples
        CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_unique
            ON edges(from_id, to_id, relation);

        -- Fast unsolved problems query
        CREATE INDEX IF NOT EXISTS idx_edges_to_relation
            ON edges(to_id, relation);

        -- Content hash lookup for dedup
        CREATE INDEX IF NOT EXISTS idx_nodes_content_hash
            ON nodes(content_hash) WHERE content_hash IS NOT NULL;

        -- Project-scoped queries by type
        CREATE INDEX IF NOT EXISTS idx_nodes_type_created
            ON nodes(node_type, created_at DESC);

        -- Source filtering (e.g. find all mcp-session nodes)
        CREATE INDEX IF NOT EXISTS idx_nodes_source
            ON nodes(source);
    ",
    )?;
    Ok(())
}
