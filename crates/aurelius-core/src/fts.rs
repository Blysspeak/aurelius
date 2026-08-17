//! Приведение пользовательской строки к безопасному запросу FTS5.
//!
//! FTS5 разбирает `MATCH`-строку как выражение, а не как текст: дефис в нём —
//! оператор `NOT`, двоеточие — указание колонки. Пользовательская строка,
//! попадавшая туда как есть, роняла поиск на любом дефисном имени:
//! `memory_search("skills-store")` отвечал «no such column: store», а
//! `"rust-clean-code"` — «no such column: clean». Так названы почти все скиллы
//! и половина проектов, поэтому симптом читался как поломка базы, а не как
//! синтаксис запроса.
//!
//! Второе свойство FTS5, столь же неочевидное: пробел между словами означает
//! `AND`. Запрос из трёх слов требовал все три сразу, и одна неудачная форма
//! слова — «алертов» вместо «алерт» — обнуляла выдачу целиком. Слова теперь
//! соединяются через `OR`, а порядок наводит ранжирование (см.
//! [`crate::graph::search_ranked`]): плохая форма портит порядок, а не результат.

/// Разобранный запрос: выражение для FTS5 и слова, из которых оно собрано.
///
/// Слова хранятся отдельно ради ответа на вопрос «почему не нашлось»: имея их,
/// вызывающий может спросить про каждое отдельно и отличить «знания нет» от
/// «слово написано в форме, которой нет в индексе».
pub struct Query {
    /// Выражение для `MATCH`.
    pub expr: String,
    /// Слова пользователя без операторов, кавычек и звёздочки.
    pub terms: Vec<String>,
}

/// Явные операторы FTS5, которые пользователь пишет сам.
const OPERATORS: [&str; 4] = ["AND", "OR", "NOT", "NEAR"];

/// Разобрать строку от человека.
///
/// Каждое слово становится фразой в кавычках — внутри фразы операторы теряют
/// силу, а FTS5-токенизатор всё равно разобьёт `skills-store` на два токена и
/// найдёт их рядом. Слова соединяются через `OR`; хвостовая звёздочка
/// префиксного поиска (`redis*` → `"redis"*`) сохраняется.
///
/// Если пользователь написал оператор заглавными сам, выражение собирается как
/// написано: явное намерение важнее умолчания, и `a AND b` обязан остаться
/// требованием обоих слов.
///
/// Пустой `expr` означает «искать нечего»: вызывающий решает сам, вернуть
/// свежие записи или пустоту.
#[must_use]
pub fn parse(raw: &str) -> Query {
    let mut parts = Vec::new();
    let mut terms = Vec::new();
    let mut explicit = false;

    for token in raw.split_whitespace() {
        if OPERATORS.contains(&token) {
            explicit = true;
            parts.push(token.to_owned());
            continue;
        }
        let (body, star) = match token.strip_suffix('*') {
            Some(body) => (body, "*"),
            None => (token, ""),
        };
        // Кавычки пользователя снимаются снаружи и удваиваются внутри: уже
        // закавыченная фраза остаётся собой, а лишняя кавычка перестаёт
        // обрывать выражение.
        let body = body.trim_matches('"');
        if body.is_empty() {
            continue;
        }
        terms.push(body.to_owned());
        parts.push(format!("\"{}\"{star}", body.replace('"', "\"\"")));
    }

    // Оператор без слов по обе стороны — не выражение, а мусор: «OR» сам по
    // себе роняет разбор FTS5.
    if terms.is_empty() {
        return Query {
            expr: String::new(),
            terms,
        };
    }

    let expr = if explicit {
        parts.join(" ")
    } else {
        parts.join(" OR ")
    };
    Query { expr, terms }
}

/// Только выражение — для вызывающих, которым диагностика слов не нужна.
#[must_use]
pub fn sanitize(raw: &str) -> String {
    parse(raw).expr
}

/// Обернуть одно слово в выражение для проверки «а это слово вообще
/// встречается». Слово уже прошло [`parse`], поэтому кавычки в нём удвоены.
#[must_use]
pub fn term_expr(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::{parse, sanitize};

    #[test]
    fn hyphenated_word_becomes_a_phrase() {
        assert_eq!(sanitize("skills-store"), "\"skills-store\"");
        assert_eq!(sanitize("rust-clean-code"), "\"rust-clean-code\"");
    }

    /// Ради чего всё: три слова больше не требуют совпадения всех трёх. Одна
    /// неудачная форма слова портит порядок выдачи, а не обнуляет её.
    #[test]
    fn words_are_joined_by_or_not_by_silent_and() {
        assert_eq!(
            sanitize("алерт телеграм отправка"),
            "\"алерт\" OR \"телеграм\" OR \"отправка\""
        );
    }

    #[test]
    fn an_explicit_operator_is_obeyed_as_written() {
        assert_eq!(sanitize("redis AND postgres"), "\"redis\" AND \"postgres\"");
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
        // Оператор без слов — тоже «искать нечего», а не сломанное выражение.
        assert_eq!(sanitize("OR"), "");
    }

    #[test]
    fn the_words_themselves_are_kept_for_diagnostics() {
        let q = parse("алерт telegram*");
        assert_eq!(q.terms, vec!["алерт".to_owned(), "telegram".to_owned()]);
    }
}
