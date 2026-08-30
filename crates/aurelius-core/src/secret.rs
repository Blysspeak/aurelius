//! Координата секрета (спека 007, US4): имя, назначение, место хранения — не
//! значение. FR-025 запрещает хранить значение секрета в любом виде, включая
//! зашифрованный: расшифровывать пришлось бы перед подстановкой в контекст, а
//! контекст уходит в транскрипт сессии на диске, в резервные копии и в API —
//! вычистить его оттуда задним числом нельзя.
//!
//! Здесь — то, чем распознаётся попытка записать значение вместо координаты
//! (T041, FR-026), и то, как из свободной строки места хранения выводится её
//! вид (T039, data-model.md).

/// Вид места хранения (`data.location_kind`, data-model.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    Env,
    File,
    PasswordManager,
}

impl LocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LocationKind::Env => "env",
            LocationKind::File => "file",
            LocationKind::PasswordManager => "password_manager",
        }
    }
}

/// Вывести вид места хранения из свободной строки `--where`: признаки, а не
/// парсер конкретного формата. URI-схема (`1password://…`) — менеджер
/// паролей; строка с разделителем пути — файл; голое имя — переменная
/// окружения.
pub fn infer_location_kind(location: &str) -> LocationKind {
    if location.contains("://") {
        LocationKind::PasswordManager
    } else if location.contains('/') || location.contains('\\') {
        LocationKind::File
    } else {
        LocationKind::Env
    }
}

/// Известные префиксы токенов реальных сервисов (T041): строка, начинающаяся
/// с одного из них, — это сам ключ, а не координата, где он лежит.
const KNOWN_KEY_PREFIXES: &[&str] = &["sk-", "ghp_", "AKIA", "xoxb-"];

/// Минимальная длина «длинной строки без пробелов» (T041). Короче — обычный
/// идентификатор или имя переменной, не значение.
const RANDOM_TOKEN_MIN_LEN: usize = 20;

/// Какой признак «похоже на само значение секрета» сработал (FR-026) —
/// печатается человеку буквально через [`SecretLookalike::explain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretLookalike {
    KnownPrefix(&'static str),
    RandomToken,
    PemHeader,
}

impl SecretLookalike {
    pub fn explain(self) -> String {
        match self {
            SecretLookalike::KnownPrefix(prefix) => format!(
                "похоже на само значение ключа: начинается с известного префикса «{prefix}»"
            ),
            SecretLookalike::RandomToken => "похоже на само значение ключа: длинная строка \
                без пробелов с высокой долей случайных на вид символов"
                .to_owned(),
            SecretLookalike::PemHeader => {
                "похоже на само значение ключа: содержит заголовок PEM".to_owned()
            }
        }
    }
}

/// Длинная строка без пробелов, не похожая на путь или URI, с как минимум
/// двумя классами символов (нижний+верхний регистр/цифры) — на глаз выглядит
/// случайной, как настоящий токен, а не как имя переменной или файла.
fn looks_like_random_token(s: &str) -> bool {
    if s.chars().count() < RANDOM_TOKEN_MIN_LEN {
        return false;
    }
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    // URI и путь — структурные строки: длинные и без пробелов, но не сам
    // секрет, а координата на него.
    if s.contains("://") || s.contains('/') || s.contains('\\') {
        return false;
    }
    let has_lower = s.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    [has_lower, has_upper, has_digit]
        .into_iter()
        .filter(|&b| b)
        .count()
        >= 2
}

/// Признак «похоже на само значение секрета» (T041, FR-026). `None` значит
/// «можно писать»; `Some` называет сработавший признак для сообщения об
/// отказе.
pub fn detect_lookalike(location: &str) -> Option<SecretLookalike> {
    if location.contains("-----BEGIN") {
        return Some(SecretLookalike::PemHeader);
    }
    for prefix in KNOWN_KEY_PREFIXES {
        if location.starts_with(prefix) {
            return Some(SecretLookalike::KnownPrefix(prefix));
        }
    }
    if looks_like_random_token(location) {
        return Some(SecretLookalike::RandomToken);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_password_manager_from_uri_scheme() {
        assert_eq!(
            infer_location_kind("1password://Private/Stripe/api-key"),
            LocationKind::PasswordManager
        );
    }

    #[test]
    fn infers_file_from_path_separator() {
        assert_eq!(
            infer_location_kind("/etc/secrets/stripe.key"),
            LocationKind::File
        );
        assert_eq!(
            infer_location_kind("C:\\secrets\\stripe.key"),
            LocationKind::File
        );
    }

    #[test]
    fn infers_env_from_bare_name() {
        assert_eq!(infer_location_kind("STRIPE_SECRET_KEY"), LocationKind::Env);
    }

    #[test]
    fn known_prefix_is_rejected() {
        let hit = detect_lookalike("sk-proj-abc123def456ghi789jkl012mno345");
        assert_eq!(hit, Some(SecretLookalike::KnownPrefix("sk-")));
    }

    #[test]
    fn github_token_prefix_is_rejected() {
        assert_eq!(
            detect_lookalike("ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
            Some(SecretLookalike::KnownPrefix("ghp_"))
        );
    }

    #[test]
    fn pem_header_is_rejected() {
        assert_eq!(
            detect_lookalike("-----BEGIN RSA PRIVATE KEY-----"),
            Some(SecretLookalike::PemHeader)
        );
    }

    #[test]
    fn long_random_looking_token_is_rejected() {
        assert_eq!(
            detect_lookalike("aZ9bQ7mK2xR5vN8pL1wT4"),
            Some(SecretLookalike::RandomToken)
        );
    }

    #[test]
    fn real_location_coordinates_are_accepted() {
        assert_eq!(detect_lookalike("1password://Private/Stripe/api-key"), None);
        assert_eq!(detect_lookalike("/etc/secrets/stripe.key"), None);
        assert_eq!(detect_lookalike("STRIPE_SECRET_KEY"), None);
    }
}
