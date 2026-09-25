//! Snowball stemming for the full-text channel.
//!
//! `nodes_fts` uses the plain unicode61 tokenizer, so a Russian word in a
//! different case or number never matched the stored form. The stems live in
//! a separate table, `nodes_stem_fts` (see `db.rs`, V19), filled through the
//! SQL function `aurelius_stem`, which is [`stem_text`].

use std::sync::LazyLock;

use rust_stemmers::{Algorithm, Stemmer};

static RUSSIAN: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::Russian));
static ENGLISH: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::English));

/// Stem one lowercase token: Russian for anything with Cyrillic letters,
/// English otherwise, digit-only tokens as they are.
fn stem_token(token: &str) -> String {
    if token.chars().all(|c| c.is_ascii_digit()) {
        return token.to_owned();
    }
    let cyrillic = token.chars().any(|c| matches!(c, '\u{0400}'..='\u{04FF}'));
    let stemmer = if cyrillic { &*RUSSIAN } else { &*ENGLISH };
    stemmer.stem(token).into_owned()
}

/// Tokenize like FTS5 unicode61 (split on non-alphanumeric characters,
/// lowercase), stem every token and join them with single spaces.
#[must_use]
pub fn stem_text(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| stem_token(&t.to_lowercase()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Stem one query word. A word FTS5 would split (`skills-store`) comes back
/// as several stems separated by spaces, to be matched as a phrase.
#[must_use]
pub fn stem_term(term: &str) -> String {
    stem_text(term)
}

#[cfg(test)]
mod tests {
    use super::{stem_term, stem_text};

    #[test]
    fn russian_forms_share_a_stem() {
        assert_eq!(stem_term("алертов"), stem_term("алерт"));
        assert_eq!(stem_term("базах"), stem_term("база"));
    }

    #[test]
    fn english_forms_share_a_stem() {
        assert_eq!(stem_term("running"), stem_term("run"));
    }

    #[test]
    fn text_is_tokenized_like_unicode61() {
        assert_eq!(stem_text("Skills-Store 2026"), "skill store 2026");
    }
}
