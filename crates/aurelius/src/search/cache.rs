use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use super::brave::SearchResult;

#[derive(Debug)]
pub struct CachedSearch {
    pub id: String,
    pub query: String,
    pub results: Vec<SearchResult>,
    pub source: String,
    pub created_at: String,
}

/// Look up cached results for an exact query. Returns None if expired or missing.
pub fn get(conn: &Connection, query: &str) -> Result<Option<CachedSearch>> {
    let now = Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id, query, results, source, created_at
         FROM search_cache
         WHERE query = ?1 AND expires_at > ?2
         ORDER BY created_at DESC
         LIMIT 1",
    )?;

    let row = stmt
        .query_row(params![query, now], |row| {
            let results_json: String = row.get(2)?;
            Ok(CachedSearch {
                id: row.get(0)?,
                query: row.get(1)?,
                results: serde_json::from_str(&results_json).unwrap_or_default(),
                source: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .optional()?;

    Ok(row)
}

/// Store search results in cache with expiration.
pub fn put(
    conn: &Connection,
    query: &str,
    results: &[SearchResult],
    source: &str,
    cache_days: i64,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires = now + Duration::days(cache_days);
    let results_json = serde_json::to_string(results)?;

    conn.execute(
        "INSERT INTO search_cache (id, query, results, source, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            query,
            results_json,
            source,
            now.to_rfc3339(),
            expires.to_rfc3339(),
        ],
    )?;

    Ok(id)
}

/// FTS search through cached results. Returns matching cached searches.
pub fn recall(conn: &Connection, query: &str, limit: usize) -> Result<Vec<CachedSearch>> {
    let now = Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT sc.id, sc.query, sc.results, sc.source, sc.created_at
         FROM search_fts
         JOIN search_cache sc ON search_fts.rowid = sc.rowid
         WHERE search_fts MATCH ?1 AND sc.expires_at > ?2
         ORDER BY rank
         LIMIT ?3",
    )?;

    let rows = stmt
        .query_map(params![query, now, limit as i64], |row| {
            let results_json: String = row.get(2)?;
            Ok(CachedSearch {
                id: row.get(0)?,
                query: row.get(1)?,
                results: serde_json::from_str(&results_json).unwrap_or_default(),
                source: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Remove expired cache entries.
pub fn cleanup(conn: &Connection) -> Result<usize> {
    let now = Utc::now().to_rfc3339();
    let deleted = conn.execute("DELETE FROM search_cache WHERE expires_at <= ?1", params![now])?;
    Ok(deleted)
}

trait OptionalRow {
    fn optional(self) -> Result<Option<CachedSearch>, rusqlite::Error>;
}

impl OptionalRow for Result<CachedSearch, rusqlite::Error> {
    fn optional(self) -> Result<Option<CachedSearch>, rusqlite::Error> {
        match self {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
