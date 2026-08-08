//! Documents into Markdown, and the cache that keeps them findable.

pub mod cache;
pub mod convert;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Where markdown too large to inline gets written. Sits beside the database
/// so that `AURELIUS_HOME` and `au home use` move it too — a profile switch
/// must not leave documents pointing into the previous home.
pub fn docs_dir() -> PathBuf {
    aurelius_core::db::db_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
        .join("docs")
}

/// Write markdown out and return where it landed. The hash prefix keeps two
/// documents with the same name apart while staying recognisable to a human
/// reading the directory.
pub fn spill(sha256: &str, file_name: &str, markdown: &str) -> Result<PathBuf> {
    let dir = docs_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let target = dir.join(format!(
        "{}-{}.md",
        &sha256[..8.min(sha256.len())],
        slug(stem)
    ));

    std::fs::write(&target, markdown)
        .with_context(|| format!("could not write {}", target.display()))?;
    Ok(target)
}

/// Filesystem-safe, length-bounded version of a file stem.
///
/// Letters stay letters whatever the alphabet: the hash prefix already
/// guarantees uniqueness, so the rest of the name exists to be recognised by
/// a human, and stripping every non-ASCII character would name every Russian
/// document `document`.
fn slug(stem: &str) -> String {
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "document".to_owned()
    } else {
        trimmed.chars().take(60).collect()
    }
}

/// Files a directory conversion should visit, in a stable order.
///
/// Walks with the same `ignore` crate the project indexer uses, so a
/// `.gitignore` keeps `node_modules` and `target` out for free. Non-recursive
/// unless asked: a tool pointed at a repository root should convert the
/// documents lying there, not descend into the whole tree.
pub fn collect_files(dir: &Path, recursive: bool, max_files: usize) -> Vec<PathBuf> {
    let mut walker = ignore::WalkBuilder::new(dir);
    walker.max_depth(if recursive { None } else { Some(1) });
    walker.hidden(true);

    let mut files: Vec<PathBuf> = walker
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .collect();

    files.sort();
    files.truncate(max_files);
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_keeps_readable_names_and_neutralises_the_rest() {
        assert_eq!(slug("Q3 report (final)"), "Q3-report-final");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("---"), "document");
        assert_eq!(slug(&"a".repeat(100)).chars().count(), 60);
    }

    /// A hash prefix plus `document` is not a name anyone can recognise, and
    /// that is what every Cyrillic (or Greek, or CJK) title would become if
    /// the slug only kept ASCII.
    #[test]
    fn slug_keeps_letters_from_any_alphabet() {
        assert_eq!(slug("Верста — примеры постов"), "Верста-примеры-постов");
        assert_eq!(slug("отчёт"), "отчёт");
    }

    /// Path separators must not survive: a spilled file belongs in the docs
    /// directory, not wherever a crafted name points.
    #[test]
    fn slug_cannot_escape_its_directory() {
        for hostile in ["../secrets", "..\\secrets", "/etc/passwd", "C:/Windows"] {
            let slugged = slug(hostile);
            assert!(!slugged.contains('/'), "{slugged}");
            assert!(!slugged.contains('\\'), "{slugged}");
            assert!(!slugged.contains(".."), "{slugged}");
        }
    }

    #[test]
    fn collect_is_shallow_by_default_and_capped() {
        let root = std::env::temp_dir().join(format!("aurelius-walk-{}", uuid::Uuid::new_v4()));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).expect("create dirs");
        std::fs::write(root.join("a.txt"), "a").expect("write");
        std::fs::write(root.join("b.txt"), "b").expect("write");
        std::fs::write(nested.join("c.txt"), "c").expect("write");

        let shallow = collect_files(&root, false, 100);
        assert_eq!(shallow.len(), 2, "{shallow:?}");

        let deep = collect_files(&root, true, 100);
        assert_eq!(deep.len(), 3, "{deep:?}");

        assert_eq!(collect_files(&root, true, 1).len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}
