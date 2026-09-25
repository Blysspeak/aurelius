//! Может ли FTS в принципе найти узел по этому запросу — вход в
//! [`super::rank::rrf_score`] (`fts_blind`).
//!
//! FTS ищет по словам. Русский запрос не делит ни одного слова с
//! английской заметкой, и отсутствие такой заметки в FTS-выдаче ничего о
//! ней не говорит — движок её просто не видит (задача f12702e1).

use crate::models::Node;

/// Доля кириллицы среди букв; `None`, когда букв нет вовсе.
fn cyrillic_share(text: &str) -> Option<f64> {
    let (mut cyr, mut all) = (0usize, 0usize);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        all += 1;
        if ('\u{0400}'..='\u{04FF}').contains(&c) {
            cyr += 1;
        }
    }
    (all > 0).then(|| cyr as f64 / all as f64)
}

/// Запрос и узел написаны разными письмами: по большинству букв один
/// кириллический, другой нет. Смешанный текст решает большинство — русская
/// заметка с `ssh` и `systemd` внутри остаётся русской. Без букв с любой
/// стороны — `false`: судить не по чему, и замена не выдаётся.
#[must_use]
pub fn fts_blind_to(query: &str, node: &Node) -> bool {
    let body = format!("{} {}", node.label, node.note.as_deref().unwrap_or(""));
    match (cyrillic_share(query), cyrillic_share(&body)) {
        (Some(q), Some(n)) => (q > 0.5) != (n > 0.5),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::cyrillic_share;

    #[test]
    fn script_is_decided_by_majority_of_letters() {
        assert!(cyrillic_share("перенос ssh на другой порт").is_some_and(|s| s > 0.5));
        assert!(cyrillic_share("sshd socket activation on ubuntu").is_some_and(|s| s < 0.5));
        assert_eq!(cyrillic_share("12:30 -- 42"), None);
    }
}
