//! Координата секрета (спека 007, US4): имя, назначение, место хранения — не
//! значение. FR-025 запрещает хранить значение секрета в любом виде, включая
//! зашифрованный: расшифровывать пришлось бы перед подстановкой в контекст, а
//! контекст уходит в транскрипт сессии на диске, в резервные копии и в API —
//! вычистить его оттуда задним числом нельзя.
//!
//! Здесь — то, чем распознаётся попытка записать значение вместо координаты
//! (T041, FR-026), и то, как из свободной строки места хранения выводится её
//! вид (T039, data-model.md), и признак узла-координаты (`is_secret_ref`,
//! FR-027) — единственное место, где он определён, чтобы поиск и выдача не
//! обрастали второй копией того же условия.

use crate::models::{Node, NodeType};

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

/// Узел — координата секрета (`Config` с `data.kind == "secret_ref"`, см.
/// `graph::add_secret_ref`). FR-027: координата отдаётся только по явному
/// запросу (`au secret list` / `secret_list`) и не должна попадать ни в
/// общий поиск, ни в снимок памяти, ни в любую другую автоматическую выдачу.
/// Раньше этот же предикат был продублирован внутри `graph::snapshot`
/// (единственное место, где он вообще проверялся) — общий полнотекстовый
/// поиск его не знал вовсе, и координата уходила в первый же посторонний
/// запрос, задевший её метку, назначение или место хранения.
pub fn is_secret_ref(node: &Node) -> bool {
    matches!(node.node_type, NodeType::Config)
        && node.data.get("kind").and_then(|v| v.as_str()) == Some("secret_ref")
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
    AwsSecretKeyShape,
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
            SecretLookalike::AwsSecretKeyShape => "похоже на само значение ключа: 40 символов \
                base64-алфавита с обоими регистрами и цифрами — формат AWS secret access key"
                .to_owned(),
        }
    }
}

/// Ровно такую длину имеет AWS secret access key (не путать с access key id,
/// у которого есть узнаваемый префикс `AKIA` — секрет к нему префикса не
/// имеет вовсе).
const AWS_SECRET_KEY_LEN: usize = 40;

/// AWS secret access key: 40 символов строго из base64-алфавита
/// (`[A-Za-z0-9+/=]`) с обоими регистрами и цифрой. Признак нарочно узкий и
/// привязан к длине, а не просто «есть `/`» — иначе легитимные координаты
/// вроде `1password://Private/Stripe/api-key` или `Documents/Projects/keys`
/// отклонялись бы как «структурные», хотя они и есть путь, а не значение.
fn looks_like_aws_secret_key(s: &str) -> bool {
    if s.chars().count() != AWS_SECRET_KEY_LEN {
        return false;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        return false;
    }
    let has_lower = s.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    has_lower && has_upper && has_digit
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
    // Проверяется до общей эвристики «длинная строка без пробелов»: та
    // намеренно сдаётся при виде '/' (см. её комментарий), а у настоящего
    // AWS secret access key '/' в base64-теле почти всегда есть.
    if looks_like_aws_secret_key(location) {
        return Some(SecretLookalike::AwsSecretKeyShape);
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

    /// Находка 12: старая эвристика `looks_like_random_token` сдавалась при
    /// виде '/' и пропускала настоящий AWS secret access key (40 символов
    /// base64-алфавита) как «структурную» строку — то есть как координату.
    #[test]
    fn aws_secret_access_key_is_rejected() {
        assert_eq!(
            detect_lookalike("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            Some(SecretLookalike::AwsSecretKeyShape)
        );
    }

    /// Асимметрия предыдущего теста: признак узкий и завязан на длину ровно
    /// 40, а не на «содержит /» — легитимные координаты с разделителем пути
    /// обязаны по-прежнему приниматься.
    #[test]
    fn path_like_coordinates_are_not_mistaken_for_an_aws_key() {
        for location in [
            "1password://Private/Stripe/api-key",
            "A:/workSpace/aurelius/.env",
            "STRIPE_SECRET_KEY",
        ] {
            assert_eq!(
                detect_lookalike(location),
                None,
                "легитимная координата отклонена: {location}"
            );
        }
    }

    /// Единственное определение признака «это координата секрета» (FR-027) —
    /// используется поиском и снимком памяти, чтобы не заводить третью копию
    /// одного и того же условия.
    #[test]
    fn is_secret_ref_matches_only_config_nodes_flagged_as_secret() {
        let secret = Node {
            id: uuid::Uuid::new_v4(),
            node_type: NodeType::Config,
            label: "STRIPE_SECRET_KEY".to_owned(),
            note: None,
            source: "test".to_owned(),
            data: serde_json::json!({"kind": "secret_ref"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            memory_kind: crate::models::MemoryKind::Semantic,
            last_accessed_at: chrono::Utc::now(),
            access_count: 0,
            content_hash: None,
            created_by: None,
            updated_by: None,
            deleted_at: None,
            sync_seq: None,
        };
        assert!(is_secret_ref(&secret));

        let mut plain_config = secret.clone();
        plain_config.data = serde_json::json!({});
        assert!(!is_secret_ref(&plain_config), "обычный Config — не секрет");

        let mut wrong_type = secret;
        wrong_type.node_type = NodeType::Concept;
        assert!(
            !is_secret_ref(&wrong_type),
            "признак 'secret_ref' на чужом типе узла не должен считаться"
        );
    }
}
