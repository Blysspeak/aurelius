//! Bulk import of an external `graph.json` document graph (spec 008, FR-001..FR-006).
//!
//! One call writes hundreds of nodes and edges that would otherwise cost one
//! `memory_add`/`memory_relate` round trip each — the whole point per the
//! spec's "Почему это вообще заводится" (§ intro): a crawl of a vendor's docs
//! must not eat a session. Everything here runs inside a single
//! `BEGIN IMMEDIATE .. COMMIT`: a half-imported graph is worse than none,
//! because a partially-linked `next_step` chain reads as a shorter path than
//! the real one instead of failing loudly.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::identity;
use crate::models::{MemoryKind, NodeType, Relation};

use super::{add_edge, add_node_full, find_edge, find_node_by_data_field, find_project_by_label};

/// Where the imported nodes came from. `project` must already exist as a
/// project node (FR: no silent project creation) — see [`validate`].
#[derive(Debug, Deserialize)]
pub struct ImportSource {
    pub id: String,
    pub project: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub fetched_at: Option<String>,
}

/// One page/anchor. `bodies` may be empty for a node with only an explicit
/// `note` (e.g. a section that is a pure structural marker).
#[derive(Debug, Deserialize)]
pub struct ImportNode {
    pub subject: String,
    pub label: String,
    #[serde(rename = "type", default = "default_node_type")]
    pub r#type: String,
    #[serde(default)]
    pub bodies: BTreeMap<String, String>,
    #[serde(default)]
    pub locale_default: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub data: Map<String, Value>,
}

fn default_node_type() -> String {
    "doc".to_owned()
}

/// `from`/`to` are `subject` strings, resolved against this file's own nodes
/// first and the live graph second (FR-005).
#[derive(Debug, Deserialize)]
pub struct ImportEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    #[serde(default)]
    pub step: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ImportFile {
    pub source: ImportSource,
    #[serde(default)]
    pub nodes: Vec<ImportNode>,
    #[serde(default)]
    pub edges: Vec<ImportEdge>,
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub nodes_created: usize,
    pub nodes_updated: usize,
    pub nodes_unchanged: usize,
    pub edges_created: usize,
    pub edges_existing: usize,
    pub elapsed_ms: u64,
}

/// Import `file` in one transaction: either every node and edge lands, or
/// none does. See module docs for why a partial import is unacceptable for
/// an ordered chain like `next_step`.
pub fn import_graph(conn: &Connection, file: &ImportFile) -> Result<ImportReport> {
    let start = Instant::now();

    conn.execute_batch("BEGIN IMMEDIATE")?;
    match import_locked(conn, file) {
        Ok(mut report) => {
            conn.execute_batch("COMMIT")?;
            report.elapsed_ms = start.elapsed().as_millis() as u64;
            Ok(report)
        }
        Err(e) => {
            // Roll back regardless of whether COMMIT was ever reached — the
            // original error is what the caller needs, not a rollback failure.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn import_locked(conn: &Connection, file: &ImportFile) -> Result<ImportReport> {
    let project_id = validate(conn, file)?;

    let mut report = ImportReport::default();
    // Keys borrow from `file`, which outlives this whole function.
    let mut subject_ids: HashMap<&str, Uuid> = HashMap::with_capacity(file.nodes.len());

    for node in &file.nodes {
        let id = write_node(conn, &file.source, node, &mut report)?;
        subject_ids.insert(node.subject.as_str(), id);
    }

    for edge in &file.edges {
        write_edge(conn, &subject_ids, project_id, edge, &mut report)?;
    }

    Ok(report)
}

/// Everything that must hold before a single row is written. Validation reads
/// through the same connection the writes will use, inside the caller's
/// `BEGIN IMMEDIATE`, so "subject already in the graph" sees a consistent
/// snapshot instead of racing a concurrent writer.
fn validate(conn: &Connection, file: &ImportFile) -> Result<Uuid> {
    let mut in_file = HashSet::with_capacity(file.nodes.len());
    for (i, node) in file.nodes.iter().enumerate() {
        if node.subject.trim().is_empty() {
            bail!("node[{i}]: subject is empty");
        }
        if !in_file.insert(node.subject.as_str()) {
            bail!(
                "node[{i}]: duplicate subject '{}' within file",
                node.subject
            );
        }
    }

    let project = find_project_by_label(conn, &file.source.project)?.ok_or_else(|| {
        anyhow::anyhow!(
            "project '{}' not found in graph — import refuses to create it silently",
            file.source.project
        )
    })?;

    for edge in &file.edges {
        if Relation::parse_known(&edge.relation).is_none() {
            bail!(
                "edge {} -> {}: unknown relation '{}'",
                edge.from,
                edge.to,
                edge.relation
            );
        }
        for subject in [edge.from.as_str(), edge.to.as_str()] {
            let known = in_file.contains(subject)
                || find_node_by_data_field(conn, "subject", subject)?.is_some();
            if !known {
                bail!(
                    "edge {} -> {}: unknown subject '{subject}'",
                    edge.from,
                    edge.to
                );
            }
        }
    }

    Ok(project.id)
}

/// Create or update one node, identified by `data.subject` (FR-002).
fn write_node(
    conn: &Connection,
    source: &ImportSource,
    node: &ImportNode,
    report: &mut ImportReport,
) -> Result<Uuid> {
    let node_type = NodeType::parse(&node.r#type);
    let note = resolve_note(node);
    let data = build_data(source, node, &node_type);
    let hash = content_hash(&node.label, note.as_deref(), &data);

    match find_node_by_data_field(conn, "subject", &node.subject)? {
        Some(existing) => {
            if existing.content_hash.as_deref() == Some(hash.as_str()) {
                report.nodes_unchanged += 1;
            } else {
                update_doc_node(
                    conn,
                    existing.id,
                    &node.label,
                    note.as_deref(),
                    &data,
                    &hash,
                )?;
                report.nodes_updated += 1;
            }
            Ok(existing.id)
        }
        None => {
            let created = add_node_full(
                conn,
                node_type,
                &node.label,
                note.as_deref(),
                "import",
                data,
                MemoryKind::Semantic,
                Some(&hash),
            )?;
            report.nodes_created += 1;
            Ok(created.id)
        }
    }
}

/// `note` (the FTS-searched body, US3 acceptance scenario 2) comes from
/// `bodies` when there is one: `locale_default`, else `en`, else the
/// alphabetically first locale (`BTreeMap` keeps that deterministic across
/// re-imports of the same file). Only a body-less node falls back to the
/// explicit `note` field.
fn resolve_note(node: &ImportNode) -> Option<String> {
    if node.bodies.is_empty() {
        return node.note.clone();
    }
    if let Some(locale) = node.locale_default.as_deref() {
        if let Some(body) = node.bodies.get(locale) {
            return Some(body.clone());
        }
    }
    node.bodies
        .get("en")
        .or_else(|| node.bodies.values().next())
        .cloned()
}

/// Merge the caller's `data` with the fields the import itself owns. System
/// fields are inserted last so a stray `subject`/`project`/... key in the
/// caller's map can never desync the node from the identity this function
/// just used to find/create it.
fn build_data(source: &ImportSource, node: &ImportNode, node_type: &NodeType) -> Value {
    let mut data = node.data.clone();
    data.insert("subject".to_owned(), Value::String(node.subject.clone()));
    if matches!(node_type, NodeType::Doc) {
        data.insert("layer".to_owned(), Value::String("vendor-docs".to_owned()));
    }
    data.insert("source_id".to_owned(), Value::String(source.id.clone()));
    data.insert("project".to_owned(), Value::String(source.project.clone()));
    data.insert(
        "bodies".to_owned(),
        serde_json::to_value(&node.bodies).unwrap_or(Value::Null),
    );
    data.insert(
        "locale_default".to_owned(),
        node.locale_default
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    data.insert(
        "fetched_at".to_owned(),
        source
            .fetched_at
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    Value::Object(data)
}

/// SHA-256 over exactly the columns [`update_doc_node`] rewrites — label,
/// note, and the fully-merged `data` — the same "hash what decides whether to
/// rewrite" shape as `session::content_hash`. `data`'s key order depends on
/// how the map was built, so it is canonicalized into a `BTreeMap` first;
/// otherwise two semantically-identical imports could hash differently and
/// FR-002's "unchanged" case would never trigger.
fn content_hash(label: &str, note: Option<&str>, data: &Value) -> String {
    let canonical: BTreeMap<&String, &Value> = match data {
        Value::Object(map) => map.iter().collect(),
        _ => BTreeMap::new(),
    };
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(b"\0");
    hasher.update(note.unwrap_or("").as_bytes());
    hasher.update(b"\0");
    hasher.update(
        serde_json::to_string(&canonical)
            .unwrap_or_default()
            .as_bytes(),
    );
    format!("{:x}", hasher.finalize())
}

/// The one UPDATE this module needs that [`super::update_node`] can't do:
/// `label` and `content_hash` in the same statement as `note`/`data`.
/// Widening `update_node` for one caller would have meant threading a new
/// optional label param through every one of its other call sites.
fn update_doc_node(
    conn: &Connection,
    id: Uuid,
    label: &str,
    note: Option<&str>,
    data: &Value,
    hash: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let author = identity::current().map(|i| i.as_author());
    conn.execute(
        "UPDATE nodes SET label = ?1, note = ?2, data = ?3, content_hash = ?4,
                updated_at = ?5, updated_by = ?6
         WHERE id = ?7 AND deleted_at IS NULL",
        params![
            label,
            note,
            serde_json::to_string(data)?,
            hash,
            now,
            author,
            id.to_string(),
        ],
    )?;
    Ok(())
}

/// Create or confirm one edge, identified by (from, to, relation) (FR-003).
fn write_edge(
    conn: &Connection,
    subject_ids: &HashMap<&str, Uuid>,
    project_id: Uuid,
    edge: &ImportEdge,
    report: &mut ImportReport,
) -> Result<()> {
    // Already checked in `validate`; re-parsing here is cheaper than plumbing
    // the parsed `Relation` through a second collection just to avoid it.
    let relation = Relation::parse_known(&edge.relation)
        .ok_or_else(|| anyhow::anyhow!("edge {} -> {}: unknown relation", edge.from, edge.to))?;
    let from_id = resolve_subject(conn, subject_ids, &edge.from)?;
    let to_id = resolve_subject(conn, subject_ids, &edge.to)?;

    // FR-006: a page never gets an edge to the project hub — membership is
    // `data.project`, not a relation, or the hub becomes the neighbor of
    // every single page (the "hub explosion" the spec's clarifications
    // section names as the reason `source` isn't a node at all).
    if from_id == project_id || to_id == project_id {
        bail!(
            "edge {} -> {}: doc nodes may not link to the project node (FR-006); \
             use data.project instead",
            edge.from,
            edge.to
        );
    }

    let weight = match relation {
        Relation::NextStep => edge.step.unwrap_or(1) as f32,
        _ => 1.0,
    };

    match find_edge(conn, from_id, to_id, &relation)? {
        Some(_) => report.edges_existing += 1,
        None => {
            add_edge(conn, from_id, to_id, relation, weight)?;
            report.edges_created += 1;
        }
    }
    Ok(())
}

fn resolve_subject(
    conn: &Connection,
    subject_ids: &HashMap<&str, Uuid>,
    subject: &str,
) -> Result<Uuid> {
    if let Some(id) = subject_ids.get(subject) {
        return Ok(*id);
    }
    find_node_by_data_field(conn, "subject", subject)?
        .map(|n| n.id)
        .ok_or_else(|| anyhow::anyhow!("subject '{subject}' not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use serde_json::json;

    /// Same rationale as `crud`'s `TmpDb`: a real temp-file database, because
    /// `db::open`'s WAL check rejects `:memory:`.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("aurelius-import-test-{tag}-{}.db", Uuid::new_v4())),
            )
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

    fn doc_node(subject: &str, label: &str, body: &str) -> ImportNode {
        let mut bodies = BTreeMap::new();
        bodies.insert("en".to_owned(), body.to_owned());
        ImportNode {
            subject: subject.to_owned(),
            label: label.to_owned(),
            r#type: "doc".to_owned(),
            bodies,
            locale_default: Some("en".to_owned()),
            note: None,
            data: Map::new(),
        }
    }

    fn step_edge(from: &str, to: &str, step: Option<u32>) -> ImportEdge {
        ImportEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            relation: "next_step".to_owned(),
            step,
        }
    }

    fn source(project: &str) -> ImportSource {
        ImportSource {
            id: "bank131-docs".to_owned(),
            project: project.to_owned(),
            url: None,
            fetched_at: None,
        }
    }

    fn seed_project(conn: &Connection, label: &str) {
        add_node_full(
            conn,
            NodeType::Project,
            label,
            None,
            "test",
            json!({}),
            MemoryKind::Semantic,
            None,
        )
        .expect("seed project");
    }

    fn three_node_file() -> ImportFile {
        ImportFile {
            source: source("xhub"),
            nodes: vec![
                doc_node("s1", "Step 1", "body one"),
                doc_node("s2", "Step 2", "body two"),
                doc_node("s3", "Step 3", "body three"),
            ],
            edges: vec![step_edge("s1", "s2", None), step_edge("s2", "s3", Some(2))],
        }
    }

    // (a) 3 nodes, 2 edges -> 3/0/0 nodes, 2/0 edges.
    #[test]
    fn import_creates_all_nodes_and_edges() {
        let (_tmp, conn) = setup();
        seed_project(&conn, "xhub");

        let report = import_graph(&conn, &three_node_file()).expect("import");

        assert_eq!(report.nodes_created, 3);
        assert_eq!(report.nodes_updated, 0);
        assert_eq!(report.nodes_unchanged, 0);
        assert_eq!(report.edges_created, 2);
        assert_eq!(report.edges_existing, 0);
    }

    // (b) same file again -> 0/0/3 nodes, 0/2 edges.
    #[test]
    fn reimporting_unchanged_file_touches_nothing() {
        let (_tmp, conn) = setup();
        seed_project(&conn, "xhub");
        let file = three_node_file();
        import_graph(&conn, &file).expect("first import");

        let report = import_graph(&conn, &file).expect("second import");

        assert_eq!(report.nodes_created, 0);
        assert_eq!(report.nodes_updated, 0);
        assert_eq!(report.nodes_unchanged, 3);
        assert_eq!(report.edges_created, 0);
        assert_eq!(report.edges_existing, 2);
    }

    // (c) change one body -> 0/1/2 nodes, edges untouched (not duplicated).
    #[test]
    fn changed_body_updates_only_that_node() {
        let (_tmp, conn) = setup();
        seed_project(&conn, "xhub");
        import_graph(&conn, &three_node_file()).expect("first import");

        let mut second = three_node_file();
        second.nodes[0] = doc_node("s1", "Step 1", "body one, revised");

        let report = import_graph(&conn, &second).expect("second import");

        assert_eq!(report.nodes_created, 0);
        assert_eq!(report.nodes_updated, 1);
        assert_eq!(report.nodes_unchanged, 2);
        assert_eq!(report.edges_created, 0);
        assert_eq!(report.edges_existing, 2);

        let updated = find_node_by_data_field(&conn, "subject", "s1")
            .unwrap()
            .expect("s1 exists");
        assert_eq!(updated.note.as_deref(), Some("body one, revised"));
    }

    // (d) edge to unknown subject -> whole import rejected, nothing written.
    #[test]
    fn edge_to_unknown_subject_aborts_the_whole_import() {
        let (_tmp, conn) = setup();
        seed_project(&conn, "xhub");
        let file = ImportFile {
            source: source("xhub"),
            nodes: vec![doc_node("s1", "Step 1", "body")],
            edges: vec![step_edge("s1", "ghost", None)],
        };

        let err = import_graph(&conn, &file).expect_err("must fail");
        assert!(err.to_string().contains("ghost"), "error was: {err}");
        assert!(
            find_node_by_data_field(&conn, "subject", "s1")
                .unwrap()
                .is_none(),
            "rejected import must not leave the node behind"
        );
    }

    // (e) duplicate subject within the file -> rejected, not "last wins".
    #[test]
    fn duplicate_subject_in_file_is_rejected() {
        let (_tmp, conn) = setup();
        seed_project(&conn, "xhub");
        let file = ImportFile {
            source: source("xhub"),
            nodes: vec![
                doc_node("s1", "Step 1", "a"),
                doc_node("s1", "Step 1 again", "b"),
            ],
            edges: vec![],
        };

        let err = import_graph(&conn, &file).expect_err("must fail");
        assert!(err.to_string().contains("s1"), "error was: {err}");
        assert!(find_node_by_data_field(&conn, "subject", "s1")
            .unwrap()
            .is_none());
    }

    // (f) next_step step=2 lands as edge weight 2.0.
    #[test]
    fn next_step_weight_equals_step_number() {
        let (_tmp, conn) = setup();
        seed_project(&conn, "xhub");
        let file = ImportFile {
            source: source("xhub"),
            nodes: vec![doc_node("s1", "Step 1", "a"), doc_node("s2", "Step 2", "b")],
            edges: vec![step_edge("s1", "s2", Some(2))],
        };

        import_graph(&conn, &file).expect("import");

        let from = find_node_by_data_field(&conn, "subject", "s1")
            .unwrap()
            .expect("s1 exists");
        let to = find_node_by_data_field(&conn, "subject", "s2")
            .unwrap()
            .expect("s2 exists");
        let edge = find_edge(&conn, from.id, to.id, &Relation::NextStep)
            .unwrap()
            .expect("edge exists");
        assert_eq!(edge.weight, 2.0);
    }

    /// Edge case from the spec: importing into a project that doesn't exist
    /// must refuse rather than silently create one.
    #[test]
    fn unknown_project_is_rejected_without_creating_it() {
        let (_tmp, conn) = setup();
        let file = three_node_file();

        let err = import_graph(&conn, &file).expect_err("must fail");
        assert!(err.to_string().contains("xhub"), "error was: {err}");
        assert!(find_project_by_label(&conn, "xhub").unwrap().is_none());
    }
}
