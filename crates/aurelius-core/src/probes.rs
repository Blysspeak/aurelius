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

fn path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Абсолютные пути Windows (A:\..., C:/...) и Unix (/home/...). Расширение
    // обязательно: голые каталоги слишком часто упоминаются в прошедшем времени.
    RE.get_or_init(|| {
        Regex::new(r"(?:[A-Za-z]:[/\\]|/)[\w./\\ -]+?\.[A-Za-z0-9]{1,8}\b")
            .expect("статический регэксп")
    })
}

fn sha_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[0-9a-f]{40}\b").expect("статический регэксп"))
}

/// Извлечь пробы из текста. Детерминированно, без сети.
pub fn extract(text: &str) -> Vec<Probe> {
    let mut out = Vec::new();
    for m in path_re().find_iter(text).take(8) {
        out.push(Probe {
            kind: ProbeKind::FileExists,
            expr: m.as_str().trim_end_matches(['.', ',', ';']).to_owned(),
        });
    }
    for m in sha_re().find_iter(text).take(4) {
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
}
