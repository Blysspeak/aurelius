//! Бинарь обязан сам сообщать своё происхождение: версию, короткий sha
//! коммита и признак грязного дерева. Без этого установленный `au` нечем
//! сверить с исходником — время файла только догадка.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

fn version_line(bin: &str) -> String {
    let out = Command::new(bin).arg("--version").output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// `имя X.Y.Z (sha[-dirty])` либо `(unknown)` при сборке без git.
fn assert_provenance(line: &str, name: &str) {
    let rest = line
        .strip_prefix(&format!("{name} {} (", env!("CARGO_PKG_VERSION")))
        .unwrap_or_else(|| panic!("нет версии и скобки: {line}"));
    let inner = rest
        .strip_suffix(')')
        .unwrap_or_else(|| panic!("нет закрывающей скобки: {line}"));
    if inner == "unknown" {
        return;
    }
    let sha = inner.strip_suffix("-dirty").unwrap_or(inner);
    assert!(
        sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "не sha: {line}"
    );
}

#[test]
fn au_version_carries_commit() {
    assert_provenance(&version_line(env!("CARGO_BIN_EXE_au")), "au");
}

#[test]
fn build_version_const_matches_cli() {
    let line = version_line(env!("CARGO_BIN_EXE_au"));
    assert_eq!(line, format!("au {}", aurelius::BUILD_VERSION));
}
