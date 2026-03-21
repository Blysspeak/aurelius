use crate::connector::Connector;
use crate::graph;
use crate::models::{MemoryKind, NodeType, RawEvent, Relation};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

const TIMEFORGED_BASE: &str = "http://127.0.0.1:6175";

pub struct TimeForgedConnector {
    client: reqwest::Client,
    since: DateTime<Utc>,
}

impl TimeForgedConnector {
    pub fn new(since: DateTime<Utc>) -> Self {
        Self {
            client: reqwest::Client::new(),
            since,
        }
    }
}

#[async_trait]
impl Connector for TimeForgedConnector {
    fn name(&self) -> &str {
        "timeforged"
    }

    async fn pull(&self) -> Result<Vec<RawEvent>> {
        let mut events = vec![];

        // Pull sessions
        match self.pull_sessions().await {
            Ok(session_events) => events.extend(session_events),
            Err(e) => {
                tracing::warn!("TimeForged unavailable (sessions): {e}");
            }
        }

        // Pull summary
        match self.pull_summary().await {
            Ok(summary_events) => events.extend(summary_events),
            Err(e) => {
                tracing::warn!("TimeForged unavailable (summary): {e}");
            }
        }

        Ok(events)
    }
}

impl TimeForgedConnector {
    async fn pull_sessions(&self) -> Result<Vec<RawEvent>> {
        let since_str = self.since.format("%Y-%m-%d").to_string();
        let url = format!("{TIMEFORGED_BASE}/api/v1/reports/sessions?from={since_str}");

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("TimeForged sessions API returned {}", resp.status());
        }

        let body: serde_json::Value = resp.json().await?;
        let mut events = vec![];

        if let Some(sessions) = body.as_array() {
            for session in sessions {
                events.push(RawEvent {
                    source: "timeforged".to_owned(),
                    kind: "session".to_owned(),
                    payload: session.clone(),
                    timestamp: parse_timestamp(session.get("start")).unwrap_or_else(Utc::now),
                });
            }
        }

        Ok(events)
    }

    async fn pull_summary(&self) -> Result<Vec<RawEvent>> {
        let since_str = self.since.format("%Y-%m-%d").to_string();
        let url = format!("{TIMEFORGED_BASE}/api/v1/reports/summary?from={since_str}");

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("TimeForged summary API returned {}", resp.status());
        }

        let body: serde_json::Value = resp.json().await?;
        Ok(vec![RawEvent {
            source: "timeforged".to_owned(),
            kind: "summary".to_owned(),
            payload: body,
            timestamp: Utc::now(),
        }])
    }
}

/// Sync TimeForged events into the graph.
pub fn sync_events(conn: &rusqlite::Connection, events: &[RawEvent]) -> Result<SyncResult> {
    let mut result = SyncResult {
        sessions: 0,
        projects: 0,
        languages: 0,
    };

    for event in events {
        match event.kind.as_str() {
            "session" => sync_session(conn, &event.payload, &mut result)?,
            "summary" => sync_summary(conn, &event.payload, &mut result)?,
            _ => {}
        }
    }

    Ok(result)
}

#[derive(Debug)]
pub struct SyncResult {
    pub sessions: usize,
    pub projects: usize,
    pub languages: usize,
}

fn sync_session(
    conn: &rusqlite::Connection,
    payload: &serde_json::Value,
    result: &mut SyncResult,
) -> Result<()> {
    let project_name = payload
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");
    let language = payload.get("language").and_then(|l| l.as_str());
    let start = payload.get("start").and_then(|s| s.as_str()).unwrap_or("");
    let duration = payload
        .get("duration")
        .and_then(|d| d.as_f64())
        .unwrap_or(0.0);

    // Create a unique session label from project + start time
    let session_label = format!("{project_name}@{start}");

    // Skip if session already exists
    if graph::find_node_by_label(conn, &session_label)?.is_some() {
        return Ok(());
    }

    // Create session node (episodic)
    let session_node = graph::add_node_full(
        conn,
        NodeType::Session,
        &session_label,
        Some(&format!("{duration:.0}s on {project_name}")),
        "timeforged",
        payload.clone(),
        MemoryKind::Episodic,
        None,
    )?;
    result.sessions += 1;

    // Get or create project node
    let project_node = if let Some(existing) = graph::find_node_by_label(conn, project_name)? {
        existing
    } else {
        let node = graph::add_node(
            conn,
            NodeType::Project,
            project_name,
            None,
            "timeforged",
            serde_json::json!({}),
        )?;
        result.projects += 1;
        node
    };

    // Session -> BelongsTo -> Project
    graph::add_edge(
        conn,
        session_node.id,
        project_node.id,
        Relation::BelongsTo,
        1.0,
    )?;

    // Language node
    if let Some(lang) = language {
        if !lang.is_empty() {
            let lang_node = if let Some(existing) = graph::find_node_by_label(conn, lang)? {
                existing
            } else {
                let node = graph::add_node(
                    conn,
                    NodeType::Language,
                    lang,
                    None,
                    "timeforged",
                    serde_json::json!({}),
                )?;
                result.languages += 1;
                node
            };

            // Session -> Uses -> Language
            graph::add_edge(conn, session_node.id, lang_node.id, Relation::Uses, 0.5)?;
        }
    }

    Ok(())
}

fn sync_summary(
    conn: &rusqlite::Connection,
    payload: &serde_json::Value,
    result: &mut SyncResult,
) -> Result<()> {
    // Extract project names from summary and ensure they exist as nodes
    if let Some(projects) = payload.get("projects").and_then(|p| p.as_array()) {
        for project_val in projects {
            let name = project_val
                .get("name")
                .or_else(|| {
                    project_val
                        .as_str()
                        .map(serde_json::Value::from)
                        .as_ref()
                        .map(|_| project_val)
                })
                .and_then(|v| v.as_str());

            if let Some(name) = name {
                if graph::find_node_by_label(conn, name)?.is_none() {
                    graph::add_node(
                        conn,
                        NodeType::Project,
                        name,
                        None,
                        "timeforged",
                        serde_json::json!({}),
                    )?;
                    result.projects += 1;
                }
            }
        }
    }

    // Extract languages
    if let Some(languages) = payload.get("languages").and_then(|l| l.as_array()) {
        for lang_val in languages {
            let name = lang_val
                .get("name")
                .or_else(|| {
                    lang_val
                        .as_str()
                        .map(serde_json::Value::from)
                        .as_ref()
                        .map(|_| lang_val)
                })
                .and_then(|v| v.as_str());

            if let Some(name) = name {
                if graph::find_node_by_label(conn, name)?.is_none() {
                    graph::add_node(
                        conn,
                        NodeType::Language,
                        name,
                        None,
                        "timeforged",
                        serde_json::json!({}),
                    )?;
                    result.languages += 1;
                }
            }
        }
    }

    Ok(())
}

fn parse_timestamp(val: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    val?.as_str()?.parse().ok()
}
