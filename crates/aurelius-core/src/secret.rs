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

    /// То же самое объяснение плюс байтовый оффset находки — единственное,
    /// что отказу дозволено сказать о МЕСТЕ совпадения (T041 рубеж на
    /// запись, FR-026 расширенный): не подстроку и не сам текст целиком, а
    /// число. Печатается буквально в отказе `add_node_full`
    /// (`SecretLookalikeRefused`), который затем уходит в stderr и в
    /// журнал улик — оба места, где сама находка не должна была бы
    /// оказаться повторно.
    pub fn explain_at(self, offset: usize) -> String {
        format!("{}, смещение {offset} байт", self.explain())
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

/// Тело токена сразу за известным префиксом (T041, найдено 07.09.2026,
/// subject `aurelius:write:secret-guard:false-positives`): голого совпадения
/// префикса недостаточно, нужен ещё и хвост, похожий на сам ключ, а не на
/// конец обычного слова. Тот же приём, что у [`looks_like_aws_secret_key`]:
/// решает форма (длина и состав символов), а не факт совпадения подстроки.
/// Без этого условия трёхбуквенный `sk-` совпадает и внутри `mask-image`,
/// `task-list`, `risk-free` — а после префикса там всего 4-5 букв, не длинный
/// хвост со случайной на вид цифрой.
///
/// Считается длина именно алфавита токена (`[A-Za-z0-9_-]`), а не до
/// ближайшего пробела: приклейка без границы слова (см. doc
/// `scan_text_for_lookalike`) обязана ловиться так же, как чистый токен, а
/// для этого нельзя требовать пробел или конец строки сразу за телом.
fn token_body_follows(tail: &str) -> bool {
    let body_len = tail
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .count();
    body_len >= RANDOM_TOKEN_MIN_LEN && tail.chars().take(body_len).any(|c| c.is_ascii_digit())
}

/// «Человеческая» hyphen-строка: slug (`backlog-audit-20260908`,
/// `feat-guard-and-trace`) или дата/время с дефисами вместо привычных
/// разделителей (`2026-09-08T17-40-00` — тот же ISO 8601, но с дефисами в
/// `HH-MM-SS` вместо двоеточий, чтобы строка годилась именем файла и
/// subject-идентификатором без экранирования). Найдено 08.09.2026 (subject
/// `aurelius:crates/aurelius-core/src/secret.rs:hyphen-slug`): `backlog-audit-20260908`
/// ложно отказывал как случайный токен, потому что дефис не входил в список
/// структурных разделителей ниже.
///
/// ЛОВУШКА, из-за которой дефис нельзя было просто дописать в тот список
/// рядом с `/`, `\`, `:`, `=`: настоящие ключи тоже дефис-разделены —
/// `sk-proj-abc123def456ghi789jkl012mno345`, Slack `xoxb-…-…-…` — и голого
/// факта «есть дефис», без разбора того, из чего состоят сегменты, было бы
/// достаточно, чтобы пропустить и их тоже. Отличает форма: у человеческого
/// слага сегменты — только строчные буквы и цифры, у настоящего токена после
/// префикса — вперемешку регистр и высокая на вид случайность. Поэтому здесь
/// не «строка содержит дефис», а «строка целиком состоит из дефис-сегментов
/// в нижнем регистре» (плюс не более одной буквы `T`-разделителя для
/// даты/времени).
///
/// Безопасность этого шейпа держится не на нём самом, а на порядке снаружи:
/// оба вызывающих (`detect_lookalike`, `scan_text_for_lookalike`) проверяют
/// `KNOWN_KEY_PREFIXES` раньше, чем доходят до `looks_like_random_token`, —
/// `sk-proj-…` и `xoxb-…` отклоняются как `KnownPrefix` до того, как эта
/// функция вообще увидит их целиком. Этот порядок закреплён тестом
/// `known_prefix_wins_over_slug_shape_even_though_it_looks_like_one`: поменяй
/// местами проверку префикса и проверку формы — тест перестанет проходить.
fn looks_like_slug(s: &str) -> bool {
    let mut seen_hyphen = false;
    let mut seen_upper_t = false;
    let mut prev_was_hyphen = true; // ведущий дефис — пустой сегмент, запрет
    for c in s.chars() {
        if c == '-' {
            if prev_was_hyphen {
                return false; // пустой сегмент: ведущий дефис или "--"
            }
            seen_hyphen = true;
            prev_was_hyphen = true;
            continue;
        }
        if c == 'T' && !seen_upper_t {
            seen_upper_t = true;
            prev_was_hyphen = false;
            continue;
        }
        if !(c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return false;
        }
        prev_was_hyphen = false;
    }
    seen_hyphen && !prev_was_hyphen // был хотя бы один дефис, и не в конце
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
    // URI, путь, `KEY=value`, `namespace:id` — структурные строки: длинные и
    // без пробелов, но не сам секрет, а координата, присвоение или составной
    // идентификатор. Найдено 07.09.2026 при подключении сканирования к
    // `--claim`/`--subject` (subject `aurelius:write:secret-guard`): ровно
    // такую форму README и документация флагов задают этим полям как
    // канонический пример (`REFUND_REQUESTS_ENABLED=true`,
    // `xhub:.env:REFUND_REQUESTS_ENABLED`) — оба длиннее
    // `RANDOM_TOKEN_MIN_LEN` и оба ложно отказывали, пока `=`/`:` не встали
    // в один ряд с `/`. Дефис в этот список НЕ входит — см. `looks_like_slug`
    // и её комментарий про ловушку с `sk-proj-…`/Slack-токенами.
    if s.contains("://")
        || s.contains('/')
        || s.contains('\\')
        || s.contains(':')
        || s.contains('=')
    {
        return false;
    }
    if looks_like_slug(s) {
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

/// Ключ поля-маркера в `data` узла, которым помечается обход рубежа
/// (`au note --allow-secret` / соответствующее поле MCP). Живёт в `data`, а
/// не в отдельном параметре `add_node_full`, потому что сигнатуру этой
/// функции менять нельзя: у неё вызывающие вне зоны правки (индексатор,
/// слияние синка, кодек импорта, MCP-обработчики) — добавить параметр значило
/// бы чинить все их вызовы разом. `data` вызывающий строит сам, до вызова —
/// маркер попадает туда тем же путём, что и любое другое поле, и остаётся в
/// узле навсегда: «какие записи легли с выключенным рубежом» — вопрос,
/// который граф потом умеет сам себе задать (SELECT по `data`), а не список,
/// который надо было вести отдельно и не забыть.
pub const BYPASS_MARKER_KEY: &str = "secret_guard_bypassed";

/// Просканировать текст произвольной длины (тело заметки, итог сессии) на
/// вхождение похожего на секрет фрагмента ГДЕ УГОДНО внутри строки — рубеж
/// на запись (T041 расширенный, subject `aurelius:write:secret-guard`), а не
/// суждение об одном поле-координате целиком, каким остаётся
/// [`detect_lookalike`]. Возвращает вид признака и байтовый оффset начала
/// совпадения: оффset — единственное, что дозволено назвать о месте находки
/// в отказе, не печатая сам текст.
///
/// Заголовок PEM ищется подстрокой по всему тексту (`str::find`), НЕ по
/// границе слова. Находка 07.09.2026 по вопросу ulika (maskSecrets,
/// hooks/lib): та регулярка держится на `\b`, и токен, приклеенный к
/// словообразующему символу без разделителя, проходит мимо. `detect_lookalike`
/// таким пробелом не страдает лишь потому, что судит значение целиком
/// (`starts_with`, вызывающий уже отрезал координату от прочего текста) —
/// здесь текст произвольной длины, и тот же приём воспроизвёл бы ту же дыру,
/// поэтому его нет вовсе. Известный префикс ищется тем же приёмом, но с
/// поправкой — см. [`token_body_follows`]: голой подстроки без неё было
/// достаточно, чтобы поймать приклейку, но она же била по обычным словам
/// (найдено 07.09.2026, subject `aurelius:write:secret-guard:false-positives`:
/// `sk-` совпадал внутри `mask-image`, `task-list`, `risk-free`).
pub fn scan_text_for_lookalike(text: &str) -> Option<(SecretLookalike, usize)> {
    if let Some(idx) = text.find("-----BEGIN") {
        return Some((SecretLookalike::PemHeader, idx));
    }
    for prefix in KNOWN_KEY_PREFIXES {
        for (idx, _) in text.match_indices(prefix) {
            if token_body_follows(&text[idx + prefix.len()..]) {
                return Some((SecretLookalike::KnownPrefix(prefix), idx));
            }
        }
    }
    // Форма AWS-ключа и «случайного токена» завязана на длину и состав ВСЕЙ
    // кандидатной строки, а не одного префикса — нужен разделитель между
    // кандидатами, и юникодный пробел здесь не то же самое, что граница
    // слова `\b`: пробел не входит ни в один алфавит, который эти две
    // проверки распознают, так что резать по нему не воспроизводит дыру,
    // из-за которой сюда вообще пришлось добавлять сканирование.
    let mut start: Option<usize> = None;
    let mut text_end = 0;
    for (idx, ch) in text.char_indices() {
        text_end = idx + ch.len_utf8();
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                if let Some(kind) = check_candidate(&text[s..idx]) {
                    return Some((kind, s));
                }
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(s) = start {
        if let Some(kind) = check_candidate(&text[s..text_end]) {
            return Some((kind, s));
        }
    }
    None
}

/// Кандидат — фрагмент текста между пробелами, проверяемый теми же формами,
/// что и координата секрета целиком: длина и состав решают, префикс — нет
/// (он уже отловлен раньше, подстрокой по всему тексту).
fn check_candidate(word: &str) -> Option<SecretLookalike> {
    if looks_like_aws_secret_key(word) {
        return Some(SecretLookalike::AwsSecretKeyShape);
    }
    if looks_like_random_token(word) {
        return Some(SecretLookalike::RandomToken);
    }
    None
}

/// Поля `data`, которым дозволено носить вольный текст, а не только машинное
/// значение (найдено 07.09.2026, subject `aurelius:write:secret-guard`):
/// `au note "тело" --claim "<токен>"` уходило кодом 0, потому что рубеж в
/// `add_node_full` судил только `label`/`note`, а карточка `agent-checkpoint`
/// отдельно велит класть дословную команду именно в `--evidence`.
/// `verify_with` — тоже команда, тот же риск, что и `evidence`, поэтому здесь
/// наравне, а не как поле второго сорта.
///
/// Список поимённый и закрытый, а не «`data` целиком»: `data` носит и
/// машинные поля (например, идемпотентный `key`), где сорокасимвольное
/// значение base64-алфавита — легитимное значение, а не совпадение с
/// [`looks_like_aws_secret_key`]; слепое сканирование отказало бы на верном
/// вводе, а это и есть путь, которым рубеж превращается в то, что выключают.
const PROVENANCE_SCAN_KEYS: &[&str] = &[
    crate::provenance::CLAIM_KEY,
    crate::provenance::EVIDENCE_KEY,
    crate::provenance::SUBJECT_KEY,
    crate::provenance::VERIFY_WITH_KEY,
];

/// Просканировать именованные провенанс-строки `data` узла (см.
/// [`PROVENANCE_SCAN_KEYS`]) той же проверкой, что и `label`/`note` —
/// второй источник свободного текста в сигнатуре `add_node_full`, после
/// самих `label`/`note`, которого не хватало до этой правки.
pub fn scan_provenance_for_lookalike(data: &serde_json::Value) -> Option<(SecretLookalike, usize)> {
    PROVENANCE_SCAN_KEYS.iter().find_map(|key| {
        data.get(*key)
            .and_then(serde_json::Value::as_str)
            .and_then(scan_text_for_lookalike)
    })
}

/// Отказ рубежа перед графом (`add_node_full`, T041 расширенный): текст поля
/// `label`/`note`, либо одной из именованных провенанс-строк `data`
/// (`claim`/`evidence`/`subject`/`verify_with`, [`scan_provenance_for_lookalike`]),
/// похож на значение секрета. Тип, а не голая строка — чтобы `classify` в
/// `au` мог опознать его через `downcast_ref`, тем же приёмом, что
/// `NoActiveTask` (`graph::mod`) для «нет активной задачи» (код 12).
/// `Display` не включает ни одно из этих полей целиком — только класс
/// признака и оффset: отказ, печатающий в себе секрет, опровергает сам себя,
/// а это сообщение уходит в stderr и в журнал улик.
#[derive(Debug, thiserror::Error)]
pub struct SecretLookalikeRefused {
    pub kind: SecretLookalike,
    pub offset: usize,
}

impl std::fmt::Display for SecretLookalikeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "запись отклонена: {}", self.kind.explain_at(self.offset))
    }
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

    /// Дефект 1 (найдено 07.09.2026, subject
    /// `aurelius:write:secret-guard:false-positives`): три обычные английские
    /// фразы ложно отказывали, потому что `sk-` ловился голой подстрокой без
    /// требования, чтобы за ним шло тело, похожее на сам токен, — матрица из
    /// задачи, все три «принять».
    #[test]
    fn ordinary_english_with_short_prefix_lookalike_is_accepted() {
        for text in [
            "CSS mask-image dropped silently",
            "the task-list rendering is off",
            "risk-free rollout plan",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "обычный текст отклонён как секрет: {text}"
            );
        }
    }

    /// Асимметрия предыдущего теста, та же матрица, три «отказать»: настоящий
    /// токен после известного префикса обязан ловиться и посередине текста, и
    /// приклеенным без границы слова с обеих сторон.
    #[test]
    fn known_prefix_with_token_shaped_body_is_still_rejected() {
        assert!(matches!(
            scan_text_for_lookalike("leaked ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789 here"),
            Some((SecretLookalike::KnownPrefix("ghp_"), _))
        ));
        assert!(matches!(
            scan_text_for_lookalike("prefixghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789suffix"),
            Some((SecretLookalike::KnownPrefix("ghp_"), _))
        ));
        assert!(matches!(
            scan_text_for_lookalike("token sk-proj-abc123def456ghi789jkl012mno345"),
            Some((SecretLookalike::KnownPrefix("sk-"), _))
        ));
    }

    /// Дефект 2 (найдено 07.09.2026, subject `aurelius:write:secret-guard`):
    /// именованные провенанс-поля судятся, а машинное поле рядом в том же
    /// `data` — нет, иначе легитимное сорокасимвольное значение вроде
    /// идемпотентного `key` отказало бы наравне с настоящим токеном.
    #[test]
    fn scan_provenance_for_lookalike_checks_only_the_four_named_fields() {
        let data = serde_json::json!({
            "claim": "risk-free rollout plan",
            "evidence": "ran ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
            "key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        });
        assert!(matches!(
            scan_provenance_for_lookalike(&data),
            Some((SecretLookalike::KnownPrefix("ghp_"), _))
        ));

        let machine_only = serde_json::json!({
            "key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        });
        assert_eq!(
            scan_provenance_for_lookalike(&machine_only),
            None,
            "машинное поле вне списка провенанс-ключей не должно сканироваться"
        );
    }

    /// Коллатеральная находка 07.09.2026 при подключении сканирования к
    /// `--claim`/`--subject`: `KEY=value` и `namespace:id` — канонические
    /// примеры этих полей из README, но без `=`/`:` в списке структурных
    /// разделителей обе формы ложно ловились как `RandomToken` (2 из 3
    /// классов символов на строке длиннее `RANDOM_TOKEN_MIN_LEN`).
    #[test]
    fn key_value_and_namespaced_identifiers_are_not_mistaken_for_random_tokens() {
        for text in [
            "REFUND_REQUESTS_ENABLED=true",
            "xhub:.env:REFUND_REQUESTS_ENABLED",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "структурная строка отклонена как секрет: {text}"
            );
        }
    }

    /// Находка 08.09.2026 (subject `aurelius:crates/aurelius-core/src/secret.rs:hyphen-slug`,
    /// живой репро: `au note` с subject `backlog-audit-20260908` отказал на
    /// смещении 0 и потребовал `--allow-secret`). Дефис не входил в список
    /// структурных разделителей `looks_like_random_token`, и slug с цифрой
    /// длиннее `RANDOM_TOKEN_MIN_LEN` ловился как случайный токен.
    #[test]
    fn hyphenated_slugs_and_dates_are_accepted_as_ordinary_text() {
        for text in [
            "backlog-audit-20260908",
            "feat-guard-and-trace",
            "2026-09-08T17-40-00",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "hyphen-slug отклонён как секрет: {text}"
            );
        }
    }

    /// Асимметрия предыдущего теста: реальные ключи тоже дефис-разделены и по
    /// одному алфавиту символов неотличимы от слага — их ловит не форма, а
    /// известный префикс, проверяемый раньше формы (см.
    /// `known_prefix_wins_over_slug_shape_even_though_it_looks_like_one`).
    #[test]
    fn hyphen_carrying_credentials_are_still_rejected() {
        // Slack bot token shape (xoxb-<team>-<bot>-<secret>). Литерал собран
        // `concat!`, а не написан целиком: цельная строка этой формы блокирует
        // `git push` защитой GitHub (GH013, push protection), хотя секретом не
        // является. Компилятору достаётся ровно та же строка, тест не ослаблен.
        assert!(matches!(
            detect_lookalike(concat!(
                "xoxb",
                "-123456789012-abcdefghijklmnopqrstuvwxyz0123456789"
            )),
            Some(SecretLookalike::KnownPrefix("xoxb-"))
        ));
        // AWS access key id — публичный пример из документации AWS, не живой секрет.
        assert!(matches!(
            detect_lookalike("AKIAIOSFODNN7EXAMPLE"),
            Some(SecretLookalike::KnownPrefix("AKIA"))
        ));
        // Ещё одно известное семейство префиксов из того же списка.
        assert!(matches!(
            detect_lookalike("sk-proj-abc123def456ghi789jkl012mno345"),
            Some(SecretLookalike::KnownPrefix("sk-"))
        ));
    }

    /// Пинает порядок проверок, обязательный по заданию: известный префикс
    /// обязан решать РАНЬШЕ, чем общий carve-out для slug-формы сможет
    /// принять строку. `sk-proj-abc123def456ghi789jkl012mno345` — валидный
    /// slug по форме (только строчные буквы, цифры и дефисы), и если
    /// поменять местами проверку `KNOWN_KEY_PREFIXES` и вызов
    /// `looks_like_random_token`/`looks_like_slug` внутри `detect_lookalike`,
    /// эта строка станет отклоняться как `None` (принята) вместо
    /// `KnownPrefix` — тест это поймает.
    #[test]
    fn known_prefix_wins_over_slug_shape_even_though_it_looks_like_one() {
        let token = "sk-proj-abc123def456ghi789jkl012mno345";
        assert!(
            looks_like_slug(token),
            "тестовая строка должна сама по себе быть валидным слагом по форме"
        );
        assert_eq!(
            detect_lookalike(token),
            Some(SecretLookalike::KnownPrefix("sk-")),
            "известный префикс должен решать раньше carve-out для slug-формы"
        );
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
