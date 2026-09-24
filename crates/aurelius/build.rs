//! Вшивает происхождение сборки: короткий sha коммита и признак грязного
//! дерева в `AURELIUS_GIT`. Без git (архив исходников, чистый контейнер)
//! значение `unknown` — сборка не падает.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim().to_owned())
}

fn main() {
    let sha = git(&["rev-parse", "--short=12", "HEAD"]).filter(|s| !s.is_empty());
    let value = match sha {
        Some(sha) => {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            if dirty {
                format!("{sha}-dirty")
            } else {
                sha
            }
        }
        None => "unknown".to_owned(),
    };
    println!("cargo:rustc-env=AURELIUS_GIT={value}");

    // Пересобирать при смене коммита и индекса; пути известны только внутри
    // репозитория, вне его хватит правила по умолчанию.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        for f in ["HEAD", "index"] {
            let p = Path::new(&dir).join(f);
            if p.exists() {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            let p = Path::new(&dir).join(r);
            if p.exists() {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
