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
