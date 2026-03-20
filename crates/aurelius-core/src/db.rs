use anyhow::Result;
use rusqlite::{Connection, params};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?
    ;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("
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
    ")?;
    Ok(())
}
