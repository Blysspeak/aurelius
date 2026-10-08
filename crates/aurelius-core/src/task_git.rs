//! Привязка задачи к ветке и рабочему дереву (`data.git`).
//!
//! До неё задача в работе не была подкреплена ничем: ветка появлялась только
//! в `resolution` при закрытии. Хранится лишь то, ГДЕ ведётся работа — ветка
//! и каталог дерева. Состояние ветки и её PR не хранится, а читается при
//! показе: записанное «запушена» устаревает первым же `git push`.
//!
//! «Влито» узнаётся только по PR на GitHub ([`pull_request`]): squash-merge
//! не делает коммиты ветки предками основной, локальной родословной тут
//! верить нельзя.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::git;

/// `gh` ходит в сеть — потолок выше, чем у локального git, но показ задачи
/// он задерживать не вправе.
const GH_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBinding {
    pub branch: String,
    /// Корень рабочего дерева, в котором стояла эта ветка.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// Названа вызывающим, а не прочитана с HEAD: хук правок её не двигает.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
}

/// Имя репозитория и привязка для каталога `dir`. `None` — не репозиторий
/// или отсоединённый HEAD: ветку назвать нечем.
pub fn detect(dir: &Path) -> Option<(String, GitBinding)> {
    let repo = git::toplevel(dir)?;
    let branch = text(git::git(
        &repo.root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?)?;
    let binding = GitBinding {
        branch,
        worktree: Some(repo.root.to_string_lossy().into_owned()),
        explicit: false,
    };
    Some((repo.name, binding))
}

/// Привязка на взятие задачи в работу: репозиторий текущего каталога, если
/// он носит имя проекта задачи, иначе записанный корень проекта — то же
/// правило, что у `tasks::project_root`.
pub fn at_activation(conn: &rusqlite::Connection, project: &str) -> Option<GitBinding> {
    let cwd = std::env::current_dir().ok();
    let repo = git::locate(conn, cwd.as_deref(), Some(project))?;
    detect(&repo.root).map(|(_, binding)| binding)
}

/// Куда перенести привязку после правки файла `file`; `None` — оставить.
///
/// Привязка идёт за правками, потому что задачу берут в работу раньше, чем
/// заводят ветку. Два случая она не трогает: названную явно и перенос с
/// рабочей ветки на основную — после слияния сессия возвращается на main, и
/// правка там стёрла бы единственный след того, где задача делалась.
pub fn follow_edit(current: Option<&GitBinding>, project: &str, file: &Path) -> Option<GitBinding> {
    if current.is_some_and(|b| b.explicit) {
        return None;
    }
    let (name, found) = detect(file.parent()?)?;
    if name != project || current == Some(&found) {
        return None;
    }
    let root = PathBuf::from(found.worktree.as_deref()?);
    if current.is_some() && found.branch == default_branch(&root) {
        return None;
    }
    Some(found)
}

/// Основная ветка по `origin/HEAD`; без него — `main`.
fn default_branch(root: &Path) -> String {
    git::git(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .and_then(text)
    .and_then(|s| s.strip_prefix("origin/").map(str::to_owned))
    .unwrap_or_else(|| "main".to_owned())
}

/// Каталог, в котором спрашивать git о привязке: её дерево, пока оно есть
/// на диске, иначе корень проекта — ветки у всех деревьев общие.
pub fn root_for(
    conn: &rusqlite::Connection,
    project: Option<&str>,
    binding: &GitBinding,
) -> Option<PathBuf> {
    binding
        .worktree
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| project.and_then(|p| crate::tasks::project_root(conn, p)))
}

/// Состояние ветки относительно вышестоящей одной строкой, без сети — по
/// тому, что уже лежит локально. `None` — git не ответил.
pub fn branch_state(root: &Path, branch: &str) -> Option<String> {
    let raw = git::git(
        root,
        &[
            "for-each-ref",
            "--format=%(upstream:short)%09%(upstream:track,nobracket)",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    let out = String::from_utf8(raw).ok()?;
    let Some(line) = out.lines().next() else {
        return Some("no local branch".to_owned());
    };
    let (upstream, track) = line.split_once('\t').unwrap_or((line, ""));
    Some(match (upstream, track) {
        ("", _) => "not pushed".to_owned(),
        (_, "") => "pushed, in sync".to_owned(),
        (_, "gone") => "upstream gone".to_owned(),
        (_, track) => format!("pushed, {track}"),
    })
}

/// PR ветки на GitHub, как его отдаёт `gh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    /// `OPEN`, `MERGED` или `CLOSED`.
    pub state: String,
    #[serde(rename(deserialize = "isDraft"), default)]
    pub draft: bool,
    pub url: String,
}

/// Самый свежий PR ветки. Ходит в сеть через `gh`; `None` — PR нет, `gh` не
/// установлен, не авторизован или не уложился в [`GH_TIMEOUT`].
pub fn pull_request(root: &Path, branch: &str) -> Option<PullRequest> {
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "list", "--head", branch, "--state", "all"])
        .args(["--limit", "1", "--json", "number,state,isDraft,url"])
        .current_dir(root)
        .env("GH_PROMPT_DISABLED", "1");
    parse_pr(&git::run(cmd, GH_TIMEOUT)?)
}

fn parse_pr(raw: &[u8]) -> Option<PullRequest> {
    serde_json::from_slice::<Vec<PullRequest>>(raw)
        .ok()?
        .into_iter()
        .next()
}

fn text(raw: Vec<u8>) -> Option<String> {
    let s = String::from_utf8(raw).ok()?.trim().to_owned();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Репозиторий `<tmp>/<uuid>/demo` с одним коммитом на ветке `main`.
    fn repo() -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("aurelius-task-git-{}", uuid::Uuid::new_v4()))
            .join("demo");
        std::fs::create_dir_all(&root).expect("mkdir");
        run(&root, &["init", "-q", "-b", "main"]);
        run(&root, &["config", "user.email", "test@example.com"]);
        run(&root, &["config", "user.name", "test"]);
        std::fs::write(root.join("a.txt"), "a").expect("write");
        run(&root, &["add", "."]);
        run(&root, &["commit", "-q", "-m", "init"]);
        root
    }

    fn run(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("запустить git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    #[test]
    fn detect_names_repo_branch_and_worktree() {
        let root = repo();
        run(&root, &["checkout", "-q", "-b", "feat/x"]);
        let (name, binding) = detect(&root).expect("привязка");
        assert_eq!(name, "demo");
        assert_eq!(binding.branch, "feat/x");
        assert!(binding.worktree.expect("дерево").ends_with("demo"));
        assert!(!binding.explicit);
    }

    #[test]
    fn detect_is_none_on_detached_head() {
        let root = repo();
        run(&root, &["checkout", "-q", "--detach"]);
        assert_eq!(detect(&root), None);
    }

    /// Рабочее дерево носит имя основного репозитория, поэтому правка в нём
    /// относится к тому же проекту и переносит привязку на его ветку.
    #[test]
    fn follow_edit_moves_binding_into_worktree_branch() {
        let root = repo();
        let wt = root.parent().expect("parent").join("wt");
        run(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/wt",
                &wt.to_string_lossy(),
            ],
        );
        let on_main = detect(&root).expect("main").1;
        let moved = follow_edit(Some(&on_main), "demo", &wt.join("a.txt")).expect("перенос");
        assert_eq!(moved.branch, "feat/wt");
        assert!(moved.worktree.expect("дерево").ends_with("wt"));
    }

    #[test]
    fn follow_edit_keeps_feature_branch_explicit_binding_and_foreign_repo() {
        let root = repo();
        let file = root.join("a.txt");
        let feature = GitBinding {
            branch: "feat/done".into(),
            worktree: None,
            explicit: false,
        };
        // С рабочей ветки на основную привязка не переезжает.
        assert_eq!(follow_edit(Some(&feature), "demo", &file), None);
        // Явную не двигает даже правка на другой рабочей ветке.
        run(&root, &["checkout", "-q", "-b", "feat/other"]);
        let explicit = GitBinding {
            explicit: true,
            ..feature
        };
        assert_eq!(follow_edit(Some(&explicit), "demo", &file), None);
        // Чужой репозиторий — не этот проект.
        assert_eq!(follow_edit(None, "another", &file), None);
        // Без привязки первая же правка её ставит.
        let first = follow_edit(None, "demo", &file).expect("первая привязка");
        assert_eq!(first.branch, "feat/other");
    }

    #[test]
    fn branch_state_tells_unpushed_synced_ahead_and_missing() {
        let root = repo();
        let origin = root.parent().expect("parent").join("origin.git");
        run(&root, &["init", "-q", "--bare", &origin.to_string_lossy()]);
        run(
            &root,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        run(&root, &["checkout", "-q", "-b", "feat/x"]);
        assert_eq!(branch_state(&root, "feat/x").as_deref(), Some("not pushed"));

        run(&root, &["push", "-q", "-u", "origin", "feat/x"]);
        assert_eq!(
            branch_state(&root, "feat/x").as_deref(),
            Some("pushed, in sync")
        );

        std::fs::write(root.join("a.txt"), "b").expect("write");
        run(&root, &["commit", "-q", "-am", "next"]);
        assert_eq!(
            branch_state(&root, "feat/x").as_deref(),
            Some("pushed, ahead 1")
        );

        // Ветку на сервере удалили (PR влит) — локальная осталась висеть.
        run(&root, &["push", "-q", "origin", "--delete", "feat/x"]);
        assert_eq!(
            branch_state(&root, "feat/x").as_deref(),
            Some("upstream gone")
        );
        assert_eq!(
            branch_state(&root, "feat/nope").as_deref(),
            Some("no local branch")
        );
    }

    #[test]
    fn parse_pr_takes_first_and_tolerates_empty_list() {
        let raw = br#"[{"isDraft":true,"number":57,"state":"MERGED","url":"https://github.com/o/r/pull/57"}]"#;
        assert_eq!(
            parse_pr(raw),
            Some(PullRequest {
                number: 57,
                state: "MERGED".into(),
                draft: true,
                url: "https://github.com/o/r/pull/57".into(),
            })
        );
        assert_eq!(parse_pr(b"[]"), None);
        assert_eq!(parse_pr(b"not json"), None);
    }
}
