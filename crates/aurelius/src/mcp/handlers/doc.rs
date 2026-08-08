use anyhow::Result;
use aurelius_core::{
    graph,
    models::{NodeType, Relation},
};
use rusqlite::Connection;
use serde_json::json;

use super::open_db;
use crate::doc::{self, cache, convert};

/// Markdown at or under this many characters comes back in the response;
/// anything larger spills to a file and returns an outline instead. A
/// 200-page PDF must not be able to fill an agent's context in one call.
const DEFAULT_MAX_INLINE_CHARS: usize = 40_000;
const DEFAULT_MAX_FILES: usize = 200;
const DEFAULT_READ_LIMIT: usize = 40_000;
const PREVIEW_CHARS: usize = 2_000;
const OUTLINE_MAX: usize = 50;

/// Convert a file — or every supported file in a directory — to Markdown.
pub fn doc_convert(params: &serde_json::Value) -> Result<serde_json::Value> {
    let path = params
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;

    let opts = Options {
        recursive: params
            .get("recursive")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        max_files: params
            .get("max_files")
            .and_then(serde_json::Value::as_u64)
            .map_or(DEFAULT_MAX_FILES, |n| n as usize),
        max_inline_chars: params
            .get("max_inline_chars")
            .and_then(serde_json::Value::as_u64)
            .map_or(DEFAULT_MAX_INLINE_CHARS, |n| n as usize),
        force: params
            .get("force")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        save_to_graph: params
            .get("save_to_graph")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        project: params
            .get("project")
            .and_then(|p| p.as_str())
            .map(str::to_owned),
    };

    let target = std::path::Path::new(path);
    let conn = open_db()?;

    if target.is_dir() {
        return convert_directory(&conn, target, &opts);
    }
    convert_one(&conn, target, &opts)
}

/// Read a slice of an already-converted document out of the cache.
pub fn doc_read(params: &serde_json::Value) -> Result<serde_json::Value> {
    let reference = params
        .get("ref")
        .and_then(|r| r.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'ref' parameter (a file path or a sha256)"))?;

    let offset = params
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let limit = params
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map_or(DEFAULT_READ_LIMIT, |n| n as usize);

    let conn = open_db()?;
    let Some(cached) = cache::get(&conn, reference)? else {
        anyhow::bail!("nothing converted for '{reference}' — call doc_convert on it first");
    };

    let total = cached.char_count as usize;
    let chunk = convert::slice_chars(&cached.markdown, offset, limit);
    let next_offset = offset + chunk.chars().count();

    Ok(json!({
        "path": cached.source_path,
        "sha256": cached.sha256,
        "format": cached.format,
        "offset": offset,
        "returned_chars": chunk.chars().count(),
        "total_chars": total,
        "has_more": next_offset < total,
        "next_offset": if next_offset < total { Some(next_offset) } else { None },
        "markdown": chunk,
    }))
}

/// Full-text search across every document ever converted.
pub fn doc_recall(params: &serde_json::Value) -> Result<serde_json::Value> {
    let query = params
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
    let limit = params
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map_or(10, |n| n as usize);

    let conn = open_db()?;
    let hits = cache::recall(&conn, query, limit)?;

    Ok(json!({
        "query": query,
        "matches": hits.iter().map(|h| json!({
            "file": h.file_name,
            "path": h.source_path,
            "sha256": h.sha256,
            "format": h.format,
            "total_chars": h.char_count,
            "converted_at": h.created_at,
            "snippet": h.snippet,
        })).collect::<Vec<_>>(),
        "read_with": "doc_read(ref=<sha256 or path>)",
    }))
}

struct Options {
    recursive: bool,
    max_files: usize,
    max_inline_chars: usize,
    force: bool,
    save_to_graph: bool,
    project: Option<String>,
}

fn convert_one(
    conn: &Connection,
    path: &std::path::Path,
    opts: &Options,
) -> Result<serde_json::Value> {
    let source_path = path.display().to_string();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document")
        .to_owned();

    // Read once. The hash decides whether this file has been converted
    // before, and the same bytes are what get converted if it hasn't.
    let source = convert::read_source(path)?;
    let hit = if opts.force {
        None
    } else {
        cache::get_by_sha(conn, &source.sha256)?
    };

    let (present, from_cache) = match hit {
        Some(cached) => {
            let spill_path = existing_or_fresh_spill(&cached, opts)?;
            (
                Present {
                    markdown: cached.markdown,
                    format: cached.format,
                    sha256: cached.sha256,
                    spill_path,
                },
                true,
            )
        }
        None => {
            let converted = convert::convert_source(source)?;
            let spill_path = if converted.markdown.chars().count() > opts.max_inline_chars {
                Some(doc::spill(
                    &converted.sha256,
                    &file_name,
                    &converted.markdown,
                )?)
            } else {
                None
            };
            cache::put(
                conn,
                &converted,
                &source_path,
                &file_name,
                spill_path.as_deref().and_then(std::path::Path::to_str),
            )?;
            (
                Present {
                    markdown: converted.markdown,
                    format: converted.format,
                    sha256: converted.sha256,
                    spill_path: spill_path.map(|p| p.display().to_string()),
                },
                false,
            )
        }
    };

    if opts.save_to_graph {
        save_document_to_graph(conn, &present, &source_path, &file_name, opts)?;
    }

    let total_chars = present.markdown.chars().count();
    let mut result = json!({
        "path": source_path,
        "file": file_name,
        "format": present.format,
        "sha256": present.sha256,
        "total_chars": total_chars,
        "cached": from_cache,
    });

    let Some(object) = result.as_object_mut() else {
        anyhow::bail!("internal: result is not an object");
    };

    if total_chars <= opts.max_inline_chars {
        object.insert("markdown".into(), json!(present.markdown));
    } else {
        object.insert("truncated".into(), json!(true));
        object.insert(
            "preview".into(),
            json!(convert::slice_chars(&present.markdown, 0, PREVIEW_CHARS)),
        );
        object.insert(
            "outline".into(),
            json!(convert::outline(&present.markdown, OUTLINE_MAX)),
        );
        object.insert("saved_to".into(), json!(present.spill_path));
        object.insert(
            "read_more".into(),
            json!(format!(
                "doc_read(ref='{}', offset={PREVIEW_CHARS})",
                present.sha256
            )),
        );
    }

    Ok(result)
}

/// What a conversion yields, whether it just ran or came back from the cache.
struct Present {
    markdown: String,
    format: String,
    sha256: String,
    spill_path: Option<String>,
}

/// Where a cached document's Markdown file is, writing it again if needed.
///
/// The inline threshold can differ between calls, so a document cached when it
/// fit inline may need spilling now — and a file spilled earlier may have been
/// deleted since.
fn existing_or_fresh_spill(cached: &cache::CachedDoc, opts: &Options) -> Result<Option<String>> {
    if let Some(existing) = &cached.spill_path {
        if std::path::Path::new(existing).exists() {
            return Ok(Some(existing.clone()));
        }
    }
    if cached.markdown.chars().count() > opts.max_inline_chars {
        let written = doc::spill(&cached.sha256, &cached.file_name, &cached.markdown)?;
        return Ok(Some(written.display().to_string()));
    }
    Ok(None)
}

fn convert_directory(
    conn: &Connection,
    dir: &std::path::Path,
    opts: &Options,
) -> Result<serde_json::Value> {
    let files = doc::collect_files(dir, opts.recursive, opts.max_files);

    let mut converted = Vec::new();
    let mut skipped = Vec::new();

    // Directory mode never inlines: a folder of contracts would bury the
    // caller. Each entry reports where its Markdown is and how to read it.
    let per_file = Options {
        recursive: opts.recursive,
        max_files: opts.max_files,
        max_inline_chars: 0,
        force: opts.force,
        save_to_graph: opts.save_to_graph,
        project: opts.project.clone(),
    };

    for file in &files {
        match convert_one(conn, file, &per_file) {
            Ok(value) => converted.push(value),
            Err(e) => skipped.push(json!({
                "path": file.display().to_string(),
                "reason": e.to_string(),
            })),
        }
    }

    Ok(json!({
        "directory": dir.display().to_string(),
        "recursive": opts.recursive,
        "files_seen": files.len(),
        "converted": converted,
        "skipped": skipped,
        "read_with": "doc_read(ref=<sha256 or path>)",
    }))
}

fn save_document_to_graph(
    conn: &Connection,
    present: &Present,
    source_path: &str,
    file_name: &str,
    opts: &Options,
) -> Result<()> {
    let project = opts.project.as_deref().unwrap_or("unknown");
    let label = format!("[{project}] {file_name}");
    let note = convert::slice_chars(&present.markdown, 0, PREVIEW_CHARS);

    // Metadata only. The Markdown body lives in doc_cache: node payloads
    // travel over sync, and a shared graph must not start shipping megabytes
    // of document text to every collaborator.
    let node = graph::add_node(
        conn,
        NodeType::Custom("document".to_owned()),
        &label,
        Some(&note),
        "mcp-doc",
        json!({
            "path": source_path,
            "sha256": present.sha256,
            "format": present.format,
            "total_chars": present.markdown.chars().count(),
            "spill_path": present.spill_path,
            "project": project,
        }),
    )?;

    let project_node = match graph::find_project_by_label(conn, project) {
        Ok(Some(n)) => n,
        _ => graph::add_node(
            conn,
            NodeType::Project,
            project,
            None,
            "mcp-doc",
            json!({"auto_created": true}),
        )?,
    };
    graph::add_edge(conn, node.id, project_node.id, Relation::BelongsTo, 1.0)?;

    Ok(())
}
