//! Ступень 4 «Бит-и-Дело»: дифф-судья исхода. Единственное место, где память
//! меняет мнение о себе, — и в нём нет ни одного вызова LLM.
//!
//! При закрытии лабильного окна следы сессии, лексически пересёкшиеся с узлом,
//! становятся уликами: успешные действия — reinforce, ошибки и отмены — erode,
//! и то и другое — fork (конфликт живёт в графе, а не решается на месте).
//! Применение вердикта — append-ревизия в node_version; erode дебетует ПУТЬ
//! извлечения и чеканит коррекцию, а не стирает знание.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Reinforce,
    Erode,
    Fork,
    Null,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Reinforce => "reinforce",
            Verdict::Erode => "erode",
            Verdict::Fork => "fork",
            Verdict::Null => "null",
        }
    }
}

/// Улика: след, атрибутированный окну.
#[derive(Debug)]
pub struct AttributedTrace {
    pub trace_id: i64,
    pub kind: String,
    pub exit_code: Option<i64>,
    pub payload: String,
}

/// Чистая функция вердикта — сердце судьи, полностью юнит-тестируема.
///
/// Правила (детерминированные, по спеке):
/// - success-улика: tool_call с exit 0, file_edit (файл реально менялся) или commit;
/// - fail-улика: kind=error, tool_call с exit != 0, user_correction с отрицанием;
/// - есть и то и другое — fork; только успех — reinforce; только провал — erode;
/// - улик нет — null (прочитано-но-бесполезно: тоже сигнал, для GC).
pub fn judge(traces: &[AttributedTrace]) -> Verdict {
    let negation = |p: &str| {
        [
            "не так",
            "нет,",
            "неверно",
            "уже не",
            "неправда",
            "устарело",
        ]
        .iter()
        .any(|m| p.to_lowercase().contains(m))
    };
    let mut ok = false;
    let mut fail = false;
    for t in traces {
        match t.kind.as_str() {
            "error" => fail = true,
            "user_correction" if negation(&t.payload) => fail = true,
            "tool_call" => match t.exit_code {
                Some(0) | None => ok = true,
                Some(_) => fail = true,
            },
            "file_edit" | "commit" | "msg_sent" => ok = true,
            _ => {}
        }
    }
    match (ok, fail) {
        (true, true) => Verdict::Fork,
        (true, false) => Verdict::Reinforce,
        (false, true) => Verdict::Erode,
        (false, false) => Verdict::Null,
    }
}

struct OpenWindow {
    id: i64,
    node_id: String,
    session_id: String,
    opened_at: i64,
}

/// Атрибуция улик окну: следы его сессии после открытия, лексически
/// пересёкшиеся с текстом узла (FTS по токенам узла).
fn attributed(conn: &Connection, w: &OpenWindow, node_text: &str) -> Result<Vec<AttributedTrace>> {
    let fts: String = node_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 3)
        .take(8)
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ");
    if fts.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT t.id, t.kind, t.exit_code, t.payload
           FROM act_trace_fts f
           JOIN act_trace t ON t.id = f.rowid
          WHERE act_trace_fts MATCH ?1
            AND t.session_id = ?2 AND t.ts >= ?3
          LIMIT 50",
    )?;
    let rows = stmt.query_map(rusqlite::params![fts, w.session_id, w.opened_at], |r| {
        Ok(AttributedTrace {
            trace_id: r.get(0)?,
            kind: r.get(1)?,
            exit_code: r.get(2)?,
            payload: r.get(3)?,
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

pub struct JudgeStats {
    pub closed: usize,
    pub reinforced: usize,
    pub eroded: usize,
    pub forked: usize,
}

/// Закрыть созревшие окна (старше `min_age_secs`) и применить вердикты.
/// Реконсолидатор: ревизия в node_version + дебет путей + чеканка коррекций.
pub fn close_ripe_windows(conn: &Connection, min_age_secs: i64) -> Result<JudgeStats> {
    let now = Utc::now().timestamp();
    let mut stats = JudgeStats {
        closed: 0,
        reinforced: 0,
        eroded: 0,
        forked: 0,
    };

    let windows: Vec<OpenWindow> = {
        let mut stmt = conn.prepare(
            "SELECT id, node_id, session_id, opened_at FROM labile_window
              WHERE closed_at IS NULL AND opened_at <= ?1 LIMIT 200",
        )?;
        let rows = stmt.query_map([now - min_age_secs], |r| {
            Ok(OpenWindow {
                id: r.get(0)?,
                node_id: r.get(1)?,
                session_id: r.get(2)?,
                opened_at: r.get(3)?,
            })
        })?;
        rows.filter_map(std::result::Result::ok).collect()
    };

    for w in &windows {
        let node: Option<(String, String)> = conn
            .query_row(
                "SELECT label, COALESCE(note, '') FROM nodes WHERE id = ?1 AND deleted_at IS NULL",
                [&w.node_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((label, note)) = node else {
            conn.execute(
                "UPDATE labile_window SET closed_at = ?1, verdict = 'null' WHERE id = ?2",
                rusqlite::params![now, w.id],
            )?;
            continue;
        };
        let text = format!("{label} {note}");
        let traces = attributed(conn, w, &text)?;
        let verdict = judge(&traces);

        for t in &traces {
            let overlap = 1.0; // лексическое совпадение уже отфильтровано FTS
            let _ = conn.execute(
                "INSERT OR IGNORE INTO trace_attribution (window_id, trace_id, overlap_score)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![w.id, t.trace_id, overlap],
            );
        }

        match verdict {
            Verdict::Reinforce => {
                stats.reinforced += 1;
                append_revision(conn, &w.node_id, &note, w.id, 1)?;
            }
            Verdict::Erode => {
                stats.eroded += 1;
                append_revision(conn, &w.node_id, &note, w.id, -1)?;
                // Дебет всех путей, ведших к узлу: промах бьёт по маршруту.
                conn.execute(
                    "UPDATE pathways SET misfires = misfires + 1,
                            blocked = CASE WHEN misfires + 1 >= 3 THEN 1 ELSE blocked END
                      WHERE node_id = ?1",
                    [&w.node_id],
                )?;
                let pattern: String = label
                    .split_whitespace()
                    .filter(|t| t.chars().count() > 3)
                    .take(6)
                    .collect::<Vec<_>>()
                    .join(" ");
                crate::window::mint_correction(
                    conn,
                    &w.node_id,
                    &pattern,
                    &format!(
                        "исход против записи «{}»: действия по ней провалились",
                        label
                    ),
                    None,
                )?;
            }
            Verdict::Fork => stats.forked += 1,
            Verdict::Null => {}
        }
        if verdict == Verdict::Reinforce {
            conn.execute(
                "UPDATE pathways SET confirms = confirms + 1 WHERE node_id = ?1",
                [&w.node_id],
            )?;
        }
        conn.execute(
            "UPDATE labile_window SET closed_at = ?1, verdict = ?2 WHERE id = ?3",
            rusqlite::params![now, verdict.as_str(), w.id],
        )?;
        stats.closed += 1;
    }
    Ok(stats)
}

/// Append-ревизия контента с причиной-окном; уровень консолидации двигается
/// вердиктом, контент узла не переписывается втихую.
fn append_revision(
    conn: &Connection,
    node_id: &str,
    content: &str,
    window_id: i64,
    level_delta: i64,
) -> Result<()> {
    let (next_rev, level): (i64, i64) = conn.query_row(
        "SELECT COALESCE(MAX(rev), 0) + 1,
                COALESCE((SELECT consolidation_level FROM node_version
                           WHERE node_id = ?1 ORDER BY rev DESC LIMIT 1), 0)
           FROM node_version WHERE node_id = ?1",
        [node_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    conn.execute(
        "INSERT INTO node_version (node_id, rev, content, consolidation_level, cause_window_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            node_id,
            next_rev,
            content,
            (level + level_delta).max(0),
            window_id,
            Utc::now().timestamp(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(kind: &str, exit: Option<i64>, payload: &str) -> AttributedTrace {
        AttributedTrace {
            trace_id: 0,
            kind: kind.into(),
            exit_code: exit,
            payload: payload.into(),
        }
    }

    #[test]
    fn judge_reinforces_on_success_traces() {
        assert_eq!(
            judge(&[t("tool_call", Some(0), "cargo build")]),
            Verdict::Reinforce
        );
        assert_eq!(judge(&[t("commit", None, "fix")]), Verdict::Reinforce);
    }

    #[test]
    fn judge_erodes_on_failures_and_negation() {
        assert_eq!(judge(&[t("error", None, "boom")]), Verdict::Erode);
        assert_eq!(
            judge(&[t("tool_call", Some(1), "cargo test")]),
            Verdict::Erode
        );
        assert_eq!(
            judge(&[t("user_correction", None, "нет, это уже не так")]),
            Verdict::Erode
        );
    }

    #[test]
    fn judge_forks_on_contradiction_and_nulls_on_silence() {
        assert_eq!(
            judge(&[
                t("tool_call", Some(0), "ok"),
                t("error", None, "later boom")
            ]),
            Verdict::Fork
        );
        assert_eq!(judge(&[]), Verdict::Null);
    }
}
