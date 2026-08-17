//! Persistence for converted documents.
//!
//! Conversion is fast enough that this is not a speed optimisation. It exists
//! so a document read once stays searchable afterwards: `doc_recall` queries
//! the FTS mirror, not the original files, which may be long gone.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use super::convert::Converted;

#[derive(Debug)]
pub struct CachedDoc {
    pub sha256: String,
    pub source_path: String,
    pub file_name: String,
    pub format: String,
    pub markdown: String,
    pub char_count: i64,
    pub byte_size: i64,
    pub spill_path: Option<String>,
    pub created_at: String,
}

/// One search hit. Carries a snippet rather than the body — a recall over a
/// shelf of PDFs must stay cheap to read.
#[derive(Debug)]
pub struct DocHit {
    pub sha256: String,
    pub source_path: String,
    pub file_name: String,
    pub format: String,
    pub char_count: i64,
    pub created_at: String,
    pub snippet: String,
}

const COLUMNS: &str = "sha256, source_path, file_name, format, markdown, char_count, byte_size, spill_path, created_at";

fn row_to_doc(row: &rusqlite::Row<'_>) -> rusqlite::Result<CachedDoc> {
    Ok(CachedDoc {
        sha256: row.get(0)?,
        source_path: row.get(1)?,
        file_name: row.get(2)?,
        format: row.get(3)?,
        markdown: row.get(4)?,
        char_count: row.get(5)?,
        byte_size: row.get(6)?,
        spill_path: row.get(7)?,
        created_at: row.get(8)?,
    })
}

/// Look up by content hash — the identity that survives a rename.
pub fn get_by_sha(conn: &Connection, sha256: &str) -> Result<Option<CachedDoc>> {
    let sql = format!("SELECT {COLUMNS} FROM doc_cache WHERE sha256 = ?1");
    let found = conn
        .prepare(&sql)?
        .query_row(params![sha256], row_to_doc)
        .optional()?;
    Ok(found)
}

/// Look up by the path it was last converted from. Most recent wins: the same
/// path can hold different content over time, and the newest conversion is the
/// one a caller asking by path means.
pub fn get_by_path(conn: &Connection, source_path: &str) -> Result<Option<CachedDoc>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM doc_cache WHERE source_path = ?1 ORDER BY created_at DESC LIMIT 1"
    );
    let found = conn
        .prepare(&sql)?
        .query_row(params![source_path], row_to_doc)
        .optional()?;
    Ok(found)
}

/// Resolve either identifier form: a content hash or a source path.
pub fn get(conn: &Connection, reference: &str) -> Result<Option<CachedDoc>> {
    if let Some(doc) = get_by_sha(conn, reference)? {
        return Ok(Some(doc));
    }
    get_by_path(conn, reference)
}

/// Store a conversion. Replaces any earlier row for the same content, which
/// keeps `source_path` pointing at where the document was most recently seen.
pub fn put(
    conn: &Connection,
    converted: &Converted,
    source_path: &str,
    file_name: &str,
    spill_path: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO doc_cache
             (sha256, source_path, file_name, format, markdown, char_count, byte_size, spill_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            converted.sha256,
            source_path,
            file_name,
            converted.format,
            converted.markdown,
            converted.markdown.chars().count() as i64,
            converted.byte_size as i64,
            spill_path,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Full-text search across everything ever converted.
pub fn recall(conn: &Connection, query: &str, limit: usize) -> Result<Vec<DocHit>> {
    // Тот же санитайзер, что и у поиска по графу: дефис в запросе — оператор
    // FTS5, а не буква.
    let expr = aurelius_core::fts::sanitize(query);
    if expr.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT d.sha256, d.source_path, d.file_name, d.format, d.char_count, d.created_at,
                snippet(doc_fts, 1, '**', '**', ' … ', 24)
         FROM doc_fts
         JOIN doc_cache d ON doc_fts.rowid = d.rowid
         WHERE doc_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let hits = stmt
        .query_map(params![expr, limit as i64], |row| {
            Ok(DocHit {
                sha256: row.get(0)?,
                source_path: row.get(1)?,
                file_name: row.get(2)?,
                format: row.get(3)?,
                char_count: row.get(4)?,
                created_at: row.get(5)?,
                snippet: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn converted(sha: &str, markdown: &str) -> Converted {
        Converted {
            markdown: markdown.to_owned(),
            format: "docx".to_owned(),
            sha256: sha.to_owned(),
            byte_size: 512,
        }
    }

    /// A real temp-file database, not `:memory:` — SQLite's in-memory mode
    /// can never report journal_mode=WAL, which `db::open` hard-rejects.
    /// Cleans up its `-wal`/`-shm` siblings on drop.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn open() -> (Self, Connection) {
            let path = std::env::temp_dir()
                .join(format!("aurelius-doc-cache-{}.db", uuid::Uuid::new_v4()));
            let conn = aurelius_core::db::open(&path).expect("open temp db");
            (Self(path), conn)
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

    #[test]
    fn round_trips_by_hash_and_by_path() {
        let (_tmp, conn) = TmpDb::open();
        put(
            &conn,
            &converted("aa11", "# Quarterly report"),
            "A:/docs/q3.docx",
            "q3.docx",
            None,
        )
        .expect("put");

        let by_sha = get_by_sha(&conn, "aa11").expect("query").expect("row");
        assert_eq!(by_sha.markdown, "# Quarterly report");
        assert_eq!(by_sha.char_count, 18);

        let by_path = get_by_path(&conn, "A:/docs/q3.docx")
            .expect("query")
            .expect("row");
        assert_eq!(by_path.sha256, "aa11");

        assert!(get_by_sha(&conn, "missing").expect("query").is_none());
    }

    #[test]
    fn get_accepts_either_identifier() {
        let (_tmp, conn) = TmpDb::open();
        put(
            &conn,
            &converted("bb22", "body"),
            "A:/docs/a.pdf",
            "a.pdf",
            None,
        )
        .expect("put");

        assert!(get(&conn, "bb22").expect("query").is_some());
        assert!(get(&conn, "A:/docs/a.pdf").expect("query").is_some());
        assert!(get(&conn, "A:/docs/nope.pdf").expect("query").is_none());
    }

    #[test]
    fn reconversion_replaces_rather_than_duplicates() {
        let (_tmp, conn) = TmpDb::open();
        put(
            &conn,
            &converted("cc33", "first"),
            "A:/a.docx",
            "a.docx",
            None,
        )
        .expect("put");
        put(
            &conn,
            &converted("cc33", "second"),
            "A:/moved/a.docx",
            "a.docx",
            None,
        )
        .expect("put");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM doc_cache", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1);

        let doc = get_by_sha(&conn, "cc33").expect("query").expect("row");
        assert_eq!(doc.markdown, "second");
        assert_eq!(doc.source_path, "A:/moved/a.docx");
    }

    #[test]
    fn recall_finds_by_body_text_and_returns_a_snippet() {
        let (_tmp, conn) = TmpDb::open();
        put(
            &conn,
            &converted("dd44", "The lease terminates in November"),
            "A:/docs/lease.pdf",
            "lease.pdf",
            None,
        )
        .expect("put");
        put(
            &conn,
            &converted("ee55", "Invoice for hosting"),
            "A:/docs/invoice.pdf",
            "invoice.pdf",
            None,
        )
        .expect("put");

        let hits = recall(&conn, "lease", 10).expect("recall");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].sha256, "dd44");
        assert!(hits[0].snippet.contains("**lease**"), "{}", hits[0].snippet);
    }

    /// The FTS mirror is trigger-driven; a replaced row must not leave its old
    /// text behind and keep matching.
    #[test]
    fn replaced_content_leaves_no_stale_index_entry() {
        let (_tmp, conn) = TmpDb::open();
        put(
            &conn,
            &converted("ff66", "aardvark"),
            "A:/a.docx",
            "a.docx",
            None,
        )
        .expect("put");
        put(
            &conn,
            &converted("ff66", "buffalo"),
            "A:/a.docx",
            "a.docx",
            None,
        )
        .expect("put");

        assert!(recall(&conn, "aardvark", 10).expect("recall").is_empty());
        assert_eq!(recall(&conn, "buffalo", 10).expect("recall").len(), 1);
    }
}
