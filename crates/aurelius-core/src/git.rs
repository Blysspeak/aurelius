//! Состояние репозитория для первого слоя снапшота: где стоит сессия.
//!
//! Проснувшаяся модель первым делом тратила два-три хода на `git status`,
//! `git branch` и `ls`, потому что снапшот ничего не говорил о том, где она
//! находится. Этот модуль отвечает на это одним блоком не больше [`B_REPO`]
//! байт.
//!
//! Наружу — только `git rev-parse` (найти корень), `git status` и `git log`,
//! каждый под [`GIT_TIMEOUT`]. Никакого `git fetch` и никакой сети: расхождение
//! с вышестоящей веткой считается по тому, что уже лежит локально.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::Connection;

use crate::models::NodeType;

/// Сколько ждать одну команду git. Хук стартует каждую сессию и не имеет
/// права её задерживать: зависший git (сетевой диск, чужой замок) стоит
/// слоя, а не старта.
pub const GIT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Потолки частей блока в байтах — в том же духе, что `B_ANCHOR`/`B_TAIL` в
/// `pickup.rs`. Весь блок — [`B_REPO`]; при переборе сперва уходят старые
/// коммиты, потом пути, ветка не уходит никогда.
const B_REPO: usize = 700;
const B_REPO_HEAD: usize = 200;
const B_REPO_NAME: usize = 60;
const B_REPO_PATH: usize = 120;
const B_REPO_COMMIT: usize = 200;

/// Сколько путей и коммитов показывать самое большее.
const MAX_PATHS: usize = 3;
const MAX_COMMITS: usize = 3;

/// Найденный репозиторий: каноническое имя (у рабочего дерева — имя основного
/// репозитория, как в `hook_project`) и корень, где запускать git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub name: String,
    pub root: PathBuf,
}

/// Где стоит HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    /// Короткий sha отсоединённого HEAD.
    Detached(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub subject: String,
}

/// Всё, что блок показывает. `upstream: None` — у ветки нет вышестоящей,
/// то есть она ни разу не запушена; это отдельный ответ, а не «неизвестно».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoState {
    pub name: String,
    pub head: Head,
    pub upstream: Option<(u32, u32)>,
    pub changed: usize,
    /// Самые свежие по времени изменения первыми, не больше [`MAX_PATHS`].
    pub paths: Vec<String>,
    /// Новые первыми, не больше [`MAX_COMMITS`].
    pub commits: Vec<Commit>,
}

/// Одна команда git в `dir` под [`GIT_TIMEOUT`]. `None` на любом отказе:
/// git не найден, ненулевой код, таймаут. Процесс по таймауту убивается, а не
/// бросается висеть.
fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        // Статус не имеет права брать замок индекса: хук идёт параллельно с
        // git владельца, и чужой `index.lock` сломал бы его команду.
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // Читать вывод параллельно ожиданию: иначе git, заполнивший трубу,
    // встанет навсегда, и таймаут сработает на здоровой команде.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).map(|_| buf)
    });
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().ok()?.ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Корень и каноническое имя репозитория, в котором лежит `dir`.
fn toplevel(dir: &Path) -> Option<Repo> {
    let out = git(
        dir,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-common-dir",
        ],
    )?;
    let out = String::from_utf8(out).ok()?;
    let mut lines = out.lines();
    let root = PathBuf::from(lines.next()?);
    let common = PathBuf::from(lines.next()?);
    // Рабочее дерево носит имя основного репозитория — то же правило, по
    // которому хук выводит проект, иначе слой и проект разошлись бы.
    let named = if common.file_name().is_some_and(|n| n == ".git") {
        common.parent()?.to_path_buf()
    } else {
        root.clone()
    };
    let name = named.file_name()?.to_str()?.to_owned();
    Some(Repo { name, root })
}

/// Корни проекта, записанные индексатором в `data.path` узла проекта.
///
/// Это и есть ограниченный поиск: граф уже знает, где лежит каждый
/// проиндексированный проект. Обход домашнего каталога на заданную глубину
/// отвергнут — сотни `read_dir` на каждом старте сессии ради ответа, который
/// уже записан.
fn recorded_roots(conn: &Connection, project: &str) -> Vec<PathBuf> {
    let Ok(project_type) = serde_json::to_string(&NodeType::Project) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT json_extract(data, '$.path') FROM nodes
          WHERE node_type = ?1 AND label = ?2 AND deleted_at IS NULL
            AND json_extract(data, '$.path') IS NOT NULL
          ORDER BY updated_at DESC",
    ) else {
        return Vec::new();
    };
    stmt.query_map(rusqlite::params![project_type, project], |r| {
        r.get::<_, String>(0)
    })
    .map(|rows| rows.filter_map(Result::ok).map(PathBuf::from).collect())
    .unwrap_or_default()
}

/// Репозиторий для слоя, по порядку:
/// 1. `git rev-parse --show-toplevel` из `cwd` — если имя совпало с проектом
///    снапшота (или проект не назван);
/// 2. иначе корень, записанный в графе для проекта, — только если его имя
///    совпадает с проектом и в нём есть `.git`: проверка файловой системой,
///    а не лишним вызовом git наугад;
/// 3. иначе `None`, и слой не печатается вовсе.
pub fn locate(conn: &Connection, cwd: Option<&Path>, project: Option<&str>) -> Option<Repo> {
    if let Some(repo) = cwd.and_then(toplevel) {
        if project.is_none_or(|p| p == repo.name) {
            return Some(repo);
        }
    }
    let project = project?;
    recorded_roots(conn, project)
        .into_iter()
        .find(|root| root.file_name().is_some_and(|n| n == project) && root.join(".git").exists())
        .map(|root| Repo {
            name: project.to_owned(),
            root,
        })
}

/// Разобрать `git status --porcelain=v2 --branch -z`.
fn parse_status(raw: &[u8]) -> (Option<Head>, Option<(u32, u32)>, Vec<String>) {
    let text = String::from_utf8_lossy(raw);
    let mut oid = None;
    let mut branch = None;
    let mut upstream = None;
    let mut paths = Vec::new();
    let mut entries = text.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        if let Some(v) = entry.strip_prefix("# branch.oid ") {
            oid = Some(v.to_owned());
        } else if let Some(v) = entry.strip_prefix("# branch.head ") {
            branch = Some(v.to_owned());
        } else if let Some(v) = entry.strip_prefix("# branch.ab ") {
            let mut it = v.split_whitespace();
            let ahead = it
                .next()
                .and_then(|a| a.trim_start_matches('+').parse().ok());
            let behind = it
                .next()
                .and_then(|b| b.trim_start_matches('-').parse().ok());
            if let (Some(a), Some(b)) = (ahead, behind) {
                upstream = Some((a, b));
            }
        } else if entry.starts_with('#') {
            continue;
        } else if let Some(p) = entry.strip_prefix("? ") {
            paths.push(p.to_owned());
        } else if entry.starts_with("1 ") {
            if let Some(p) = entry.splitn(9, ' ').nth(8) {
                paths.push(p.to_owned());
            }
        } else if entry.starts_with("2 ") {
            if let Some(p) = entry.splitn(10, ' ').nth(9) {
                paths.push(p.to_owned());
            }
            // За переименованием в `-z` идёт исходный путь отдельной записью.
            entries.next();
        } else if entry.starts_with("u ") {
            if let Some(p) = entry.splitn(11, ' ').nth(10) {
                paths.push(p.to_owned());
            }
        }
    }
    let head = match (branch.as_deref(), oid.as_deref()) {
        (Some("(detached)"), Some(oid)) => Some(Head::Detached(oid.chars().take(7).collect())),
        (Some(b), _) => Some(Head::Branch(b.to_owned())),
        _ => None,
    };
    (head, upstream, paths)
}

fn parse_log(raw: &[u8]) -> Vec<Commit> {
    String::from_utf8_lossy(raw)
        .lines()
        .filter_map(|line| {
            let mut it = line.splitn(3, '\t');
            Some(Commit {
                sha: it.next()?.to_owned(),
                date: it.next()?.to_owned(),
                subject: it.next()?.to_owned(),
            })
        })
        .collect()
}

/// Прочитать состояние репозитория. `None` — git не ответил, слоя не будет.
pub fn read(repo: &Repo) -> Option<RepoState> {
    let status = git(
        &repo.root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ],
    )?;
    let (head, upstream, paths) = parse_status(&status);
    let changed = paths.len();
    // Самые свежие правки первыми; удалённый файл времени не имеет и идёт
    // последним.
    let mut dated: Vec<(Option<SystemTime>, String)> = paths
        .into_iter()
        .map(|p| {
            let mtime = std::fs::symlink_metadata(repo.root.join(&p))
                .and_then(|m| m.modified())
                .ok();
            (mtime, p)
        })
        .collect();
    dated.sort_by_key(|d| std::cmp::Reverse(d.0));
    let paths = dated.into_iter().take(MAX_PATHS).map(|(_, p)| p).collect();
    // Пустой репозиторий без коммитов — законное состояние: `git log` падает,
    // коммитов ноль, ветка всё равно есть.
    let commits = git(
        &repo.root,
        &[
            "log",
            &format!("-{MAX_COMMITS}"),
            "--no-show-signature",
            "--format=%h%x09%cs%x09%s",
        ],
    )
    .map(|raw| parse_log(&raw))
    .unwrap_or_default();
    Some(RepoState {
        name: repo.name.clone(),
        head: head?,
        upstream,
        changed,
        paths,
        commits,
    })
}

/// Обрезать строку до `max` байт по границе символа, с многоточием.
fn clip_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max.saturating_sub('…'.len_utf8());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Путь длиннее потолка режется слева: имя файла важнее начала пути.
fn clip_path(p: &str) -> String {
    if p.len() <= B_REPO_PATH {
        return p.to_owned();
    }
    let mut start = p.len() - B_REPO_PATH.saturating_sub('…'.len_utf8());
    while start < p.len() && !p.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &p[start..])
}

fn head_line(s: &RepoState) -> String {
    // Потолок режет имя, а не ветку: ветку блок не теряет никогда.
    let name = clip_bytes(&s.name, B_REPO_NAME);
    let head = match &s.head {
        Head::Branch(b) => format!("{name} on {b}"),
        Head::Detached(sha) => format!("{name} detached at {sha}"),
    };
    let upstream = match s.upstream {
        Some((ahead, behind)) => format!("ahead {ahead}, behind {behind}"),
        None => "no upstream".to_owned(),
    };
    format!(
        "- {}\n",
        clip_bytes(&format!("{head}, {upstream}"), B_REPO_HEAD)
    )
}

fn dirty_line(s: &RepoState, paths: usize) -> String {
    if s.changed == 0 {
        return "- clean\n".to_owned();
    }
    let shown: Vec<String> = s.paths.iter().take(paths).map(|p| clip_path(p)).collect();
    if shown.is_empty() {
        format!("- {} changed\n", s.changed)
    } else {
        format!("- {} changed: {}\n", s.changed, shown.join(", "))
    }
}

fn commit_line(c: &Commit) -> String {
    let subject = c.subject.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "- {}\n",
        clip_bytes(&format!("{} {} {subject}", c.sha, c.date), B_REPO_COMMIT)
    )
}

/// Блок слоя. Не длиннее [`B_REPO`] байт: при переборе сперва уходят самые
/// старые коммиты, затем пути; строка ветки остаётся всегда.
#[must_use]
pub fn render(s: &RepoState) -> String {
    let head = head_line(s);
    let mut commits: Vec<String> = s.commits.iter().map(commit_line).collect();
    let mut paths = s.paths.len();
    loop {
        let text = format!("{head}{}{}", dirty_line(s, paths), commits.concat());
        if text.len() <= B_REPO || (commits.is_empty() && paths == 0) {
            return text;
        }
        if commits.pop().is_none() {
            paths -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> RepoState {
        RepoState {
            name: "demo".into(),
            head: Head::Branch("feat/x".into()),
            upstream: Some((2, 0)),
            changed: 8,
            paths: vec!["a.rs".into(), "b.rs".into(), "c.rs".into()],
            commits: vec![
                Commit {
                    sha: "aaaaaaa".into(),
                    date: "2026-09-19".into(),
                    subject: "newest".into(),
                },
                Commit {
                    sha: "bbbbbbb".into(),
                    date: "2026-09-18".into(),
                    subject: "middle".into(),
                },
                Commit {
                    sha: "ccccccc".into(),
                    date: "2026-09-17".into(),
                    subject: "oldest".into(),
                },
            ],
        }
    }

    #[test]
    fn porcelain_v2_parses_branch_divergence_and_every_entry_kind() {
        let raw =
            "# branch.oid 0123456789abcdef\0# branch.head main\0# branch.upstream origin/main\0\
                   # branch.ab +2 -1\0\
                   1 .M N... 100644 100644 100644 abc abc src/lib.rs\0\
                   2 R. N... 100644 100644 100644 abc abc R100 new name.rs\0old.rs\0\
                   ? scratch.patch\0";
        let (head, upstream, paths) = parse_status(raw.as_bytes());
        assert_eq!(head, Some(Head::Branch("main".into())));
        assert_eq!(upstream, Some((2, 1)));
        assert_eq!(paths, vec!["src/lib.rs", "new name.rs", "scratch.patch"]);
    }

    #[test]
    fn detached_head_reports_short_sha_and_no_upstream() {
        let raw = "# branch.oid 0123456789abcdef\0# branch.head (detached)\0";
        let (head, upstream, paths) = parse_status(raw.as_bytes());
        assert_eq!(head, Some(Head::Detached("0123456".into())));
        assert_eq!(upstream, None);
        assert!(paths.is_empty());
    }

    #[test]
    fn render_names_branch_divergence_dirt_and_commits() {
        let text = render(&state());
        assert_eq!(
            text,
            "- demo on feat/x, ahead 2, behind 0\n\
             - 8 changed: a.rs, b.rs, c.rs\n\
             - aaaaaaa 2026-09-19 newest\n\
             - bbbbbbb 2026-09-18 middle\n\
             - ccccccc 2026-09-17 oldest\n"
        );
    }

    /// Перебор потолка: сперва уходят старые коммиты, затем пути; ветка —
    /// никогда.
    #[test]
    fn render_over_ceiling_drops_oldest_commits_then_paths_never_branch() {
        let mut s = state();
        for c in &mut s.commits {
            c.subject = "слово ".repeat(40);
        }
        s.paths = (0..3)
            .map(|i| format!("{}{i}.rs", "p/".repeat(55)))
            .collect();
        let text = render(&s);
        assert!(text.len() <= B_REPO, "{} байт:\n{text}", text.len());
        assert!(text.starts_with("- demo on feat/x, ahead 2, behind 0\n"));
        assert!(
            !text.contains("ccccccc"),
            "старейший коммит обязан уйти первым"
        );

        s.commits.clear();
        s.paths = (0..3)
            .map(|i| format!("{}{i}.rs", "дир/".repeat(40)))
            .collect();
        s.name = "д".repeat(90);
        let text = render(&s);
        assert!(text.len() <= B_REPO, "{} байт:\n{text}", text.len());
        assert!(
            text.contains(" on feat/x"),
            "ветка не уходит никогда:\n{text}"
        );
    }

    #[test]
    fn clean_tree_and_missing_upstream_are_said_out_loud() {
        let mut s = state();
        s.changed = 0;
        s.paths.clear();
        s.upstream = None;
        s.commits.truncate(1);
        assert_eq!(
            render(&s),
            "- demo on feat/x, no upstream\n- clean\n- aaaaaaa 2026-09-19 newest\n"
        );
    }
}
