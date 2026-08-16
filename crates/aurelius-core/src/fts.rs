//! Приведение пользовательской строки к безопасному запросу FTS5.
//!
//! FTS5 разбирает `MATCH`-строку как выражение, а не как текст: дефис в нём —
//! оператор `NOT`, двоеточие — указание колонки. Пользовательская строка,
//! попадавшая туда как есть, роняла поиск на любом дефисном имени:
//! `memory_search("skills-store")` отвечал «no such column: store», а
//! `"rust-clean-code"` — «no such column: clean». Так названы почти все скиллы
//! и половина проектов, поэтому симптом читался как поломка базы, а не как
//! синтаксис запроса.

/// Превратить строку от человека в выражение, которое FTS5 разберёт.
///
/// Каждое слово становится фразой в кавычках — внутри фразы операторы теряют
/// силу, а FTS5-токенизатор всё равно разобьёт `skills-store` на два токена и
/// найдёт их рядом. Сознательно сохранены две вещи: явные операторы
/// `AND`/`OR`/`NOT`/`NEAR` заглавными и хвостовая звёздочка префиксного поиска
/// (`redis*` → `"redis"*`) — на них опирается уже написанное.
///
/// Пустая строка на выходе означает «искать нечего»: вызывающий решает сам,
/// вернуть свежие записи или пустоту.
#[must_use]
pub fn sanitize(raw: &str) -> String {
    raw.split_whitespace()
        .map(|token| {
            if matches!(token, "AND" | "OR" | "NOT" | "NEAR") {
                return token.to_owned();
            }
            let (body, star) = match token.strip_suffix('*') {
                Some(body) => (body, "*"),
                None => (token, ""),
            };
            // Кавычки пользователя снимаются снаружи и удваиваются внутри:
            // уже закавыченная фраза остаётся собой, а лишняя кавычка
            // перестаёт обрывать выражение.
            let body = body.trim_matches('"');
            if body.is_empty() {
                return String::new();
            }
            format!("\"{}\"{star}", body.replace('"', "\"\""))
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn hyphenated_word_becomes_a_phrase() {
        assert_eq!(sanitize("skills-store"), "\"skills-store\"");
        assert_eq!(sanitize("rust-clean-code"), "\"rust-clean-code\"");
    }

    #[test]
    fn operators_and_prefix_search_survive() {
        assert_eq!(sanitize("redis OR postgres"), "\"redis\" OR \"postgres\"");
        assert_eq!(sanitize("redis*"), "\"redis\"*");
    }

    #[test]
    fn quotes_are_neutralized_not_propagated() {
        assert_eq!(sanitize("\"skills-store\""), "\"skills-store\"");
        assert_eq!(sanitize("\""), "");
        assert_eq!(sanitize("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn nothing_to_search_is_an_empty_string() {
        assert_eq!(sanitize("   "), "");
        assert_eq!(sanitize(""), "");
    }
}
