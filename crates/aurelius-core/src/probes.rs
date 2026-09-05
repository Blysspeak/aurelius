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
}
