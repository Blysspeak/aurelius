//! Ступень 2 «Бит-и-Дело»: пробы — машинно-проверяемые утверждения памяти.
//!
//! Из текста узла извлекаются проверяемые факты (пути файлов, git-SHA, имена
//! команд) и исполняются против ground truth здесь и сейчас. Память, чьи
//! утверждения проваливают проверку, не должна тихо циркулировать дальше:
//! вызывающий решает, что делать с провалом (advisory-режим волны 2 — только
//! записать; жёсткий гейт рождения включится вместе с судьёй исхода).

use anyhow::Result;
use chrono::Utc;
use regex::Regex;
use rusqlite::Connection;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeKind {
    FileExists,
    GitSha,
    CmdInPath,
}

impl ProbeKind {
    fn as_str(&self) -> &'static str {
        match self {
            ProbeKind::FileExists => "file_exists",
            ProbeKind::GitSha => "git_sha",
            ProbeKind::CmdInPath => "cmd_in_path",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Probe {
    pub kind: ProbeKind,
    pub expr: String,
}

#[derive(Debug)]
pub struct ProbeReport {
    pub total: usize,
    pub failed: Vec<Probe>,
}

// `.expect` на этих трёх регэкспах не бьёт по принципу III: шаблон — литерал,
// известный правильным на этапе написания кода, а не данные прогона. Упасть
// он может только на опечатке в исходнике (поймает любой тест, вызвавший
// пробу), никогда — на вводе пользователя.
#[allow(clippy::expect_used)]
fn path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Абсолютные пути Windows (A:\..., C:/...) и Unix (/home/...). Расширение
    // обязательно: голые каталоги слишком часто упоминаются в прошедшем времени.
    RE.get_or_init(|| {
        Regex::new(r"(?:[A-Za-z]:[/\\]|/)[\w./\\ -]+?\.[A-Za-z0-9]{1,8}\b")
            .expect("статический регэксп")
    })
}

#[allow(clippy::expect_used)]
fn sha_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[0-9a-f]{40}\b").expect("статический регэксп"))
}

#[allow(clippy::expect_used)]
fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Адрес — не путь в файловой системе, но выглядит как два сразу: «https://»
    // подходит под шаблон диска (`s:/`), а всё после хоста — под абсолютный
    // путь. Адреса вырезаются ДО извлечения, а не отсеиваются после: обрывок
    // URL от пути уже неотличим.
    RE.get_or_init(|| Regex::new(r"[A-Za-z][A-Za-z0-9+.\-]*://\S+").expect("статический регэксп"))
}

/// Начинается ли совпадение на границе слова.
///
/// Regex в Rust не умеет lookbehind, а `\b` здесь не годится: перед `/` в
/// `crates/aurelius-core/src/graph/search.rs` граница есть, и путь резался с
/// середины в обрывок `/aurelius-core/...`, которого никто не утверждал.
///
/// `@` и `~` в этом списке — про импорт по алиасу. `@/config/env.js` — не
/// утверждение о файле `/config/env.js`: алиас разворачивается сборщиком по
/// своим правилам (`@/*` → `src/*`), а расширение в импорте вообще может не
/// совпадать с расширением на диске (`.js` в ESM-импорте против `.ts` в
/// файле). Проверять такой токен на диске значит гарантированно проваливать
/// пробу на любой записи, цитирующей импорт.
fn starts_at_boundary(hay: &str, start: usize) -> bool {
    hay[..start].chars().next_back().is_none_or(|c| {
        !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '\\' | '@' | '~'))
    })
}

/// Расширение из одних цифр — это номер версии, а не файл: `v1.11`, `0.3.2`.
fn extension_looks_like_a_file(expr: &str) -> bool {
    expr.rsplit('.')
        .next()
        .is_some_and(|ext| ext.chars().any(|c| c.is_ascii_alphabetic()))
}

/// Извлечь пробы из текста. Детерминированно, без сети.
///
/// Проба обязана быть утверждением, которое автор действительно сделал. Ложная
/// проба хуже отсутствующей: она шумит в ответе на каждую запись и приучает
/// вызывающего не читать предупреждения — а ради предупреждений всё и написано.
pub fn extract(text: &str) -> Vec<Probe> {
    let cleaned = url_re().replace_all(text, " ");
    let mut out = Vec::new();
    for m in path_re().find_iter(&cleaned) {
        if out.len() >= 8 {
            break;
        }
        if !starts_at_boundary(&cleaned, m.start()) {
            continue;
        }
        let expr = m.as_str().trim_end_matches(['.', ',', ';']);
        if !extension_looks_like_a_file(expr) {
            continue;
        }
        out.push(Probe {
            kind: ProbeKind::FileExists,
            expr: expr.to_owned(),
        });
    }
    for m in sha_re().find_iter(&cleaned).take(4) {
        out.push(Probe {
            kind: ProbeKind::GitSha,
            expr: m.as_str().to_owned(),
        });
    }
    out
}

/// Исполнить одну пробу против ground truth. `workdir` — контекст git-проверок.
pub fn run(probe: &Probe, workdir: &Path) -> bool {
    match probe.kind {
        ProbeKind::FileExists => Path::new(&probe.expr).exists(),
        ProbeKind::GitSha => std::process::Command::new("git")
            .args(["cat-file", "-e", &probe.expr])
            .current_dir(workdir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false),
        ProbeKind::CmdInPath => which(&probe.expr),
    }
}

fn which(cmd: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let base = dir.join(cmd);
        base.exists() || base.with_extension("exe").exists() || base.with_extension("cmd").exists()
    })
}

/// Извлечь, исполнить и записать пробы узла. Возвращает отчёт; решение о
/// судьбе узла — за вызывающим (advisory в волне 2).
pub fn check_and_record(
    conn: &Connection,
    node_id: &str,
    text: &str,
    workdir: &Path,
) -> Result<ProbeReport> {
    let probes = extract(text);
    let now = Utc::now().timestamp();
    let mut failed = Vec::new();
    for p in &probes {
        let ok = run(p, workdir);
        conn.execute(
            "INSERT INTO probes (node_id, kind, expr, last_ok, checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![node_id, p.kind.as_str(), p.expr, i64::from(ok), now],
        )?;
        if !ok {
            failed.push(p.clone());
        }
    }
    Ok(ProbeReport {
        total: probes.len(),
        failed,
    })
}

/// A recorded probe result that did not pass, as read back for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedProbe {
    pub kind: String,
    pub expr: String,
    /// Unix seconds of the recorded check; `None` if the row has no time.
    pub checked_at: Option<i64>,
}

/// Failing probes for a set of nodes, keyed by node id, newest check first.
///
/// One query for the whole set (ids travel as one JSON array through
/// `json_each`, so there is no bind-parameter limit to hit). Only the latest
/// row per `(node, kind, expr)` counts: a later passing check supersedes an
/// earlier failure. Reads what was recorded; never re-runs a probe.
pub fn failing_for(
    conn: &Connection,
    node_ids: &[&str],
) -> Result<std::collections::HashMap<String, Vec<FailedProbe>>> {
    let mut out: std::collections::HashMap<String, Vec<FailedProbe>> =
        std::collections::HashMap::new();
    if node_ids.is_empty() {
        return Ok(out);
    }
    let ids = serde_json::to_string(node_ids)?;
    let mut stmt = conn.prepare(
        "SELECT p.node_id, p.kind, p.expr, p.checked_at FROM probes p
         WHERE p.node_id IN (SELECT value FROM json_each(?1))
           AND p.last_ok = 0
           AND p.id = (SELECT MAX(q.id) FROM probes q
                       WHERE q.node_id = p.node_id AND q.kind = p.kind AND q.expr = p.expr)
         ORDER BY p.checked_at DESC, p.id DESC",
    )?;
    let rows = stmt.query_map([ids], |r| {
        Ok((
            r.get::<_, String>(0)?,
            FailedProbe {
                kind: r.get(1)?,
                expr: r.get(2)?,
                checked_at: r.get(3)?,
            },
        ))
    })?;
    for row in rows {
        let (node_id, probe) = row?;
        out.entry(node_id).or_default().push(probe);
    }
    Ok(out)
}

/// After a recorded edit of `path`, give every `file_exists` probe that names
/// it a new passing row, so the passing check supersedes the old failure.
/// A probe names the path when its expr equals it, or when the expr is a
/// relative path the absolute path ends with (at a separator). Only probes
/// whose latest row failed are touched; nothing happens for a relative
/// `path` or a file that does not exist now. One query plus the inserts.
pub fn refresh_after_edit(conn: &Connection, path: &str) -> Result<usize> {
    let p = Path::new(path);
    if !p.is_absolute() || !p.exists() {
        return Ok(0);
    }
    let stale: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT p.node_id, p.expr FROM probes p
             WHERE p.kind = 'file_exists' AND p.last_ok = 0
               AND (p.expr = ?1
                    OR (length(p.expr) < length(?1)
                        AND substr(p.expr, 1, 1) NOT IN ('/', '\\')
                        AND substr(?1, length(?1) - length(p.expr) + 1) = p.expr
                        AND substr(?1, length(?1) - length(p.expr), 1) IN ('/', '\\')))
               AND p.id = (SELECT MAX(q.id) FROM probes q
                           WHERE q.node_id = p.node_id AND q.kind = p.kind AND q.expr = p.expr)",
        )?;
        let rows = stmt.query_map([path], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let now = Utc::now().timestamp();
    for (node_id, expr) in &stale {
        conn.execute(
            "INSERT INTO probes (node_id, kind, expr, last_ok, checked_at)
             VALUES (?1, 'file_exists', ?2, 1, ?3)",
            rusqlite::params![node_id, expr, now],
        )?;
    }
    Ok(stale.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_paths_and_shas() {
        let text = "файл A:/workSpace/tg-mcp/src/db.ts и коммит d16a9e67d16a9e67d16a9e67d16a9e67d16a9e67, а слово просто.так — нет";
        let probes = extract(text);
        assert!(probes
            .iter()
            .any(|p| p.kind == ProbeKind::FileExists && p.expr.contains("db.ts")));
        assert!(probes.iter().any(|p| p.kind == ProbeKind::GitSha));
    }

    /// Реальный шум, пойманный на записи о релизе 15.08.2026: одна заметка со
    /// ссылкой и версией родила три несуществующих «пути».
    #[test]
    fn url_version_and_mid_word_slash_are_not_paths() {
        let text = "Релиз опубликован: https://github.com/Blysspeak/aurelius/releases/tag/v1.11.0 \
                    — предикат живёт в crates/aurelius-core/src/graph/search.rs";
        let probes = extract(text);

        assert!(
            probes.is_empty(),
            "ни адрес, ни номер версии, ни обрывок относительного пути пробами не являются: {:?}",
            probes.iter().map(|p| &p.expr).collect::<Vec<_>>()
        );
    }

    /// Импорт по алиасу — не утверждение о файле. `@/config/env.js`
    /// разворачивается сборщиком в `src/config/env.ts`, поэтому проверка
    /// «/config/env.js» на диске проваливалась всегда, на каждой записи,
    /// цитирующей импорт.
    #[test]
    fn an_aliased_import_is_not_a_claim_about_a_file() {
        let probes = extract("конфиг читается из @/config/env.js, а не из process.env напрямую");
        assert!(
            probes.is_empty(),
            "алиас-импорт пробой не является: {:?}",
            probes.iter().map(|p| &p.expr).collect::<Vec<_>>()
        );
        assert!(extract("см. ~/config/env.js").is_empty());
    }

    #[test]
    fn a_real_absolute_path_still_becomes_a_probe() {
        let probes = extract("правка в A:/workSpace/aurelius/Cargo.toml");
        assert_eq!(probes.len(), 1, "настоящий путь обязан остаться пробой");
        assert!(probes[0].expr.ends_with("Cargo.toml"));
    }

    #[test]
    fn file_probe_checks_ground_truth() {
        let exe = std::env::current_exe().expect("current exe");
        let ok = run(
            &Probe {
                kind: ProbeKind::FileExists,
                expr: exe.to_string_lossy().into_owned(),
            },
            Path::new("."),
        );
        assert!(ok);
        let missing = run(
            &Probe {
                kind: ProbeKind::FileExists,
                expr: "A:/точно/нет/такого/файла.rs".into(),
            },
            Path::new("."),
        );
        assert!(!missing);
    }

    fn probe_db() -> (std::path::PathBuf, Connection) {
        let path =
            std::env::temp_dir().join(format!("aurelius-probes-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).expect("open temp db");
        (path, conn)
    }

    fn drop_db(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(p));
        }
    }

    fn put(conn: &Connection, node: &str, expr: &str, ok: bool, at: i64) {
        conn.execute(
            "INSERT INTO probes (node_id, kind, expr, last_ok, checked_at) VALUES (?1, 'file_exists', ?2, ?3, ?4)",
            rusqlite::params![node, expr, i64::from(ok), at],
        )
        .expect("insert probe");
    }

    #[test]
    fn a_file_edit_trace_clears_the_failing_probe() {
        let (path, conn) = probe_db();
        let file = std::env::temp_dir().join(format!("probe-edit-{}.rs", uuid::Uuid::new_v4()));
        std::fs::write(&file, "x").expect("write file");
        let abs = file.to_string_lossy().into_owned();
        let name = file
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        put(&conn, "a", &abs, false, 1);
        put(&conn, "b", &name, false, 1);
        put(&conn, "c", &format!("x{name}"), false, 1);
        assert_eq!(failing_for(&conn, &["a", "b", "c"]).expect("q").len(), 3);

        crate::trace::ingest(
            &conn,
            &crate::trace::TraceInput {
                session_id: "s",
                kind: crate::trace::TraceKind::FileEdit,
                payload: &abs,
                exit_code: None,
                state_hash_pre: None,
                state_hash_post: None,
            },
        )
        .expect("ingest");

        let left = failing_for(&conn, &["a", "b", "c"]).expect("q");
        assert!(!left.contains_key("a"), "exact path refreshed");
        assert!(!left.contains_key("b"), "relative suffix refreshed");
        assert!(
            left.contains_key("c"),
            "a suffix without a separator is another file"
        );
        let _ = std::fs::remove_file(&file);
        drop(conn);
        drop_db(&path);
    }

    #[test]
    fn failing_for_reads_the_whole_set_and_honours_the_latest_check() {
        let (path, conn) = probe_db();
        put(&conn, "a", "/gone.rs", false, 100);
        put(&conn, "b", "/here.rs", true, 100);
        put(&conn, "c", "/fixed.rs", false, 100);
        put(&conn, "c", "/fixed.rs", true, 200);
        put(&conn, "d", "/gone.rs", false, 100);
        let got = failing_for(&conn, &["a", "b", "c", "d", "missing"]).expect("query");
        drop_db(&path);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got["a"][0].expr, "/gone.rs");
        assert_eq!(got["a"][0].checked_at, Some(100));
        assert!(got.contains_key("d"));
        assert!(failing_for(&conn, &[]).expect("empty").is_empty());
    }
}
