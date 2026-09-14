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

    /// То же самое объяснение плюс байтовый оффset находки.
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

/// Префиксы, обязанные стоять на границе слова; `sk-` — единственный элемент
/// `KNOWN_KEY_PREFIXES`, начало которого живёт внутри обычных английских слов
/// (`task-hygiene`, `mask-image`, `risk-free`): приклеенный к слову он
/// совпадает подстрокой даже с длинным похожим на токен хвостом —
/// `task-hygiene-wf_77b967a8-bb2`, найдено 13.09.2026, subject
/// `aurelius:write:secret-guard:false-positives` — и побеждается только
/// границей слова; остальные префиксы (`ghp_`, `AKIA`, `xoxb-`) внутри
/// обычных слов не встречаются, и требование границы для них лишь ослабило
/// бы рубеж: настоящий токен, приклеенный без разделителя, обязан отказать
/// точно так же, как отдельный, — см. тесты
/// `known_prefix_with_token_shaped_body_is_still_rejected` и
/// `token_glued_between_prefix_and_suffix_without_word_boundary_is_still_refused`.
fn prefix_requires_word_boundary(prefix: &str) -> bool {
    prefix == "sk-"
}

/// Граница слова перед байтовой позицией `idx`: начало текста, либо
/// предыдущий символ — не буква, не цифра и не подчёркивание (пробел,
/// кавычка, знак равенства, двоеточие, скобка, иная пунктуация); алфавит
/// юникодный: кириллица перед токеном — тоже внутри слова, не граница.
fn is_at_word_boundary(text: &str, idx: usize) -> bool {
    match text[..idx].chars().next_back() {
        None => true,
        Some(prev) => !(prev.is_alphanumeric() || prev == '_'),
    }
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
/// `sk-` совпадал внутри `mask-image`, `task-list`, `risk-free`). Для `sk-`
/// совпадение обязано вдобавок стоять на границе слова (найдено 13.09.2026,
/// тот же subject: `sk-` внутри `task-hygiene-wf_77b967a8-bb2` с хвостом
/// длиннее порога) — см. [`prefix_requires_word_boundary`].
pub fn scan_text_for_lookalike(text: &str) -> Option<(SecretLookalike, usize)> {
    if let Some(idx) = text.find("-----BEGIN") {
        return Some((SecretLookalike::PemHeader, idx));
    }
    for prefix in KNOWN_KEY_PREFIXES {
        for (idx, _) in text.match_indices(prefix) {
            if prefix_requires_word_boundary(prefix) && !is_at_word_boundary(text, idx) {
                continue;
            }
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

/// Стандартные длины hex-хэша: SHA-1 (git) и SHA-256 (`sha256sum`, git v2).
/// Форма неотличима от опакового hex-ключа той же длины — поблажка только
/// здесь, для свободного текста ([`check_candidate`]); `detect_lookalike`
/// (координата секрета) её не получает.
const HEX_HASH_LENS: &[usize] = &[40, 64];

fn is_lower_hex(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, 'a'..='f')
}

fn looks_like_hex_hash(s: &str) -> bool {
    HEX_HASH_LENS.contains(&s.chars().count()) && s.chars().all(is_lower_hex)
}

/// Канонический UUID: 5 групп hex через дефис, длины 8-4-4-4-12.
fn looks_like_uuid(s: &str) -> bool {
    const GROUP_LENS: [usize; 5] = [8, 4, 4, 4, 12];
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip(GROUP_LENS)
            .all(|(g, len)| g.chars().count() == len && g.chars().all(is_lower_hex))
}

/// Диапазон git-коммитов: два hex-sha по 7–40 символов (минимальная
/// аббревиатура git … полная длина SHA-1), соединённые двумя точками
/// (`08cf458e..3ffb349e`). Найдено 14.09.2026, subject
/// `aurelius:write:secret-guard:false-positives`: короткая форма из двух
/// заглушек проходила лишь потому, что целиком короче
/// `RANDOM_TOKEN_MIN_LEN`, а две полные hex-стороны ловились как случайный
/// токен — точка не входит в структурные разделители
/// `looks_like_random_token`. Рубеж это не ослабляет: 40-hex-строка и так
/// принимается отдельно через `HEX_HASH_LENS`, диапазон — та же форма
/// дважды, соединённая `..`, и ничего, кроме hex и двух точек, в себе не
/// несёт.
fn looks_like_git_range(s: &str) -> bool {
    let Some((left, right)) = s.split_once("..") else {
        return false;
    };
    let sha = |part: &str| {
        (7..=40).contains(&part.chars().count()) && part.chars().all(is_lower_hex)
    };
    sha(left) && sha(right)
}

/// Сегмент квалифицированного идентификатора (`Тип` или `метод`): обычный
/// идентификатор кода и строго короче `RANDOM_TOKEN_MIN_LEN` — тот же порог,
/// что у случайного токена, чтобы длинный base64-сегмент (например, часть
/// JWT `header.payload.signature`), сам по себе похожий на секрет, не прошёл
/// под видом «имени».
fn is_identifier_segment(seg: &str) -> bool {
    if seg.is_empty() || seg.chars().count() >= RANDOM_TOKEN_MIN_LEN {
        return false;
    }
    let mut chars = seg.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `Тип.метод` или `Тип.метод()` — читаемый код-идентификатор, не значение.
fn looks_like_qualified_identifier(s: &str) -> bool {
    let core = s.strip_suffix("()").unwrap_or(s);
    let segments: Vec<&str> = core.split('.').collect();
    segments.len() >= 2 && segments.iter().all(|seg| is_identifier_segment(seg))
}

/// `callee(arg, arg, ...)` — простой вызов, не произвольное выражение:
/// ровно одна пара скобок, callee и каждый аргумент — идентификатор той же
/// формы и длины, что сегмент [`looks_like_qualified_identifier`]. Найдено
/// 11.09.2026 (репро: `createMapper(claude)`, 20 символов, отказал в
/// провенанс-поле).
fn looks_like_simple_call(s: &str) -> bool {
    let Some(open) = s.find('(') else {
        return false;
    };
    if !s.ends_with(')') {
        return false;
    }
    let callee = &s[..open];
    let args = &s[open + 1..s.len() - 1];
    is_identifier_segment(callee)
        && (args.is_empty() || args.split(',').all(|arg| is_identifier_segment(arg.trim())))
}

/// Слово кода в camelCase (`translateTurnContent`, `actualReceiveAmountUsdt`,
/// `resolveCardCommission`) — читаемое имя функции, поля или метода, а не
/// значение. Найдено 14.09.2026 (subject
/// `aurelius:crates/aurelius-core/src/secret.rs:camelcase-identifier`):
/// `translateTurnContent` в поле `evidence` двух заметок про dsh-russian-lang
/// отказал как `RandomToken` на смещении 55 и 384 байт, и обе заметки легли
/// только через `--allow-secret`. До этой правки свободный текст знал четыре
/// безопасные формы (хэш, UUID, `Тип.метод`, простой вызов), но голого имени
/// без точки и скобок среди них не было — а `looks_like_random_token` его
/// ловил: два класса символов и длина больше порога.
///
/// Решает форму не длина, а рисунок регистра — тот же приём, что у
/// [`looks_like_slug`] с его сегментами. У camelCase-имени каждый отрезок
/// строчных букв между заглавными (и начальный тоже) длиннее одного символа:
/// слова, а не буквы. У случайной base64-строки той же длины
/// (`aZ9bQ7mK2xR5vN8pL1wT4`) строчные идут по одной между заглавными и
/// цифрами, и она остаётся отклонённой; цифра, разделитель или ведущая
/// заглавная снимают поблажку так же.
fn looks_like_code_identifier(s: &str) -> bool {
    if s.chars().count() < RANDOM_TOKEN_MIN_LEN {
        return false;
    }
    if !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let mut has_upper = false;
    let mut lower_run = 0usize;
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            // Отрезок строчных перед заглавной: ровно одна буква — это уже
            // не начало слова, а разброс регистра случайной строки. Ноль —
            // ведущая заглавная (`AntigravityModels`), тоже не camelCase.
            if lower_run <= 1 {
                return false;
            }
            has_upper = true;
            lower_run = 0;
        } else {
            lower_run += 1;
        }
    }
    has_upper && lower_run > 1
}

/// Пунктуация конца предложения, которую свободный текст цепляет к токену
/// без пробела (`hash.`, `identifier,`) — снимается только для проверки
/// формы; `looks_like_aws_secret_key` и `looks_like_random_token` в
/// [`check_candidate`] по-прежнему видят слово целиком, с этой пунктуацией.
const SENTENCE_PUNCTUATION: &[char] = &[',', '.', ';', '!', '?'];

/// Снять обёрточную пунктуацию (`(x)`, `[x]`, `{x}`, кавычки, `` `x` ``) и
/// пунктуацию конца предложения — в любом порядке, вложенно (`` "(`x`)." ``),
/// пока строка меняется. Форму значения под обёрткой это не меняет.
fn strip_wrapping_punctuation(word: &str) -> &str {
    const PAIRS: &[(char, char)] = &[
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('"', '"'),
        ('\'', '\''),
        ('`', '`'),
    ];
    let mut core = word;
    loop {
        if let Some(trimmed) = core.strip_suffix(SENTENCE_PUNCTUATION) {
            core = trimmed;
            continue;
        }
        let mut stripped = false;
        for &(open, close) in PAIRS {
            if let Some(inner) = core.strip_prefix(open).and_then(|s| s.strip_suffix(close)) {
                core = inner;
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    core
}

/// Узкий список безопасных технических форм для свободного текста (найдено
/// 11.09.2026: git-хэш, UUID, квалифицированный идентификатор и простой
/// вызов ложно ловились `looks_like_random_token` — 2 класса символов на
/// строке длиннее `RANDOM_TOKEN_MIN_LEN` не отличают их от самого секрета той
/// же формы; 13–14.09.2026 сюда же легли диапазон git-коммитов и слаг под
/// обёрточной пунктуацией (subject `aurelius:write:secret-guard:false-positives`),
/// 14.09.2026 — голое camelCase-имя [`looks_like_code_identifier`].
/// Не бланкетный обход: обычный случайный токен под той же
/// обёрткой ни в одну из этих форм не попадёт и продолжит отказывать через
/// `looks_like_random_token` в [`check_candidate`] ниже.
///
/// Слаг здесь проверяется по ядру после [`strip_wrapping_punctuation`], а не
/// по сырой строке, как внутри `looks_like_random_token`: запятая конца
/// предложения, приклеенная к слагу длиннее порога без пробела
/// (`claude-opus-4-6-thinking,`, найдено 14.09.2026, репро 5в того же
/// subject), ломала распознавание слага в сыром виде — а голый слаг той же
/// формы проходил. Смешанно-регистровый случайный токен слагом не является и
/// обёрткой не спасается (асимметрия
/// `wrapped_generic_random_token_is_still_rejected`).
fn looks_like_safe_technical_shape(word: &str) -> bool {
    let core = strip_wrapping_punctuation(word);
    looks_like_hex_hash(core)
        || looks_like_uuid(core)
        || looks_like_git_range(core)
        || looks_like_qualified_identifier(core)
        || looks_like_simple_call(core)
        || looks_like_code_identifier(core)
        || looks_like_slug(core)
}

/// Кандидат — фрагмент текста между пробелами, проверяемый теми же формами,
/// что и координата секрета целиком: длина и состав решают, префикс — нет
/// (он уже отловлен раньше, подстрокой по всему тексту). Известные безопасные
/// формы ([`looks_like_safe_technical_shape`]) проверяются до
/// `looks_like_random_token` — только здесь, не в `detect_lookalike`.
fn check_candidate(word: &str) -> Option<SecretLookalike> {
    if looks_like_aws_secret_key(word) {
        return Some(SecretLookalike::AwsSecretKeyShape);
    }
    if looks_like_safe_technical_shape(word) {
        return None;
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

/// Marker that replaces a masked secret value in trace payloads.
pub const SECRET_MASK: &str = "***";

/// Whether a key (env var, flag, JSON field) names a secret, judged by its
/// words rather than substrings: `max_tokens`, `tokens_used`, `tokenizer`,
/// `secretary_name` and `passport` are not secrets, `access_token`,
/// `GITHUB_TOKEN`, `x-api-key`, `clientSecret` and `PGPASSWORD` are.
pub fn is_secret_key(key: &str) -> bool {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in key.chars() {
        if matches!(c, '_' | '-' | '.') {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase();
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    let single = |p: &str| {
        matches!(
            p,
            "password" | "passwd" | "pwd" | "pass" | "token" | "secret" | "apikey"
        ) || p.ends_with("password")
    };
    let pair = |a: &str, b: &str| {
        matches!(
            (a, b),
            ("api", "key")
                | ("access", "key")
                | ("private", "key")
                | ("client", "secret")
                | ("auth", "token")
        )
    };
    parts.iter().any(|p| single(p)) || parts.windows(2).any(|w| pair(&w[0], &w[1]))
}

/// One masking rule. A keyed rule captures the key in the named group `key`
/// and replaces the match only when [`is_secret_key`] accepts that key.
struct MaskRule {
    re: regex::Regex,
    rep: &'static str,
    keyed: bool,
}

// Static literal patterns: `.expect` can only fire on a typo in this source
// (any test calling `mask_secrets` catches it), never on runtime input.
#[allow(clippy::expect_used)]
fn mask_rules() -> &'static [MaskRule] {
    static RULES: std::sync::OnceLock<Vec<MaskRule>> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        [
            // Password part of URL userinfo: scheme://user:PASSWORD@host.
            (
                r"([A-Za-z][A-Za-z0-9+.\-]*://[^\s:/@]+:)[^\s@/]+@",
                "${1}***@",
                false,
            ),
            // Authorization header value and bare Bearer tokens.
            (
                r#"(?i)(authorization:\s*(?:bearer\s+|basic\s+|token\s+)?|\bbearer\s+)(?:"[^"]*"|'[^']*'|[^\s'"]+)"#,
                "${1}***",
                false,
            ),
            // Known token prefixes (GitHub, OpenAI/Anthropic, Slack, AWS, GitLab).
            (
                r"\b(?:gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{20,}|sk-(?:ant-)?[A-Za-z0-9_\-]{16,}|xox[bpa]-[A-Za-z0-9\-]{10,}|AKIA[0-9A-Z]{16}|glpat-[A-Za-z0-9_\-]{20,})",
                "***",
                false,
            ),
            // Quoted keys in JSON / dict style: "password": "x", 'token': 'y'.
            (
                r#"(["'](?P<key>[A-Za-z0-9_.\-]+)["']\s*:\s*)"[^"]*""#,
                "${1}\"***\"",
                true,
            ),
            (
                r#"(["'](?P<key>[A-Za-z0-9_.\-]+)["']\s*:\s*)'[^']*'"#,
                "${1}'***'",
                true,
            ),
            (
                r#"(["'](?P<key>[A-Za-z0-9_.\-]+)["']\s*:\s*)[^\s"',}\]]+"#,
                "${1}***",
                true,
            ),
            // Flag followed by a space: --password VALUE, --api-key VALUE.
            (
                r#"(--(?P<key>[A-Za-z0-9][A-Za-z0-9_\-]*)\s+)(?:"[^"]*"|'[^']*'|[^\s'"\-][^\s'"]*)"#,
                "${1}***",
                true,
            ),
            // key=value / key: value where the key names a secret.
            (
                r#"\b(?P<key>[A-Za-z0-9_.\-]+)(\s*[=:]\s*)(?:"[^"]*"|'[^']*'|[^\s'"&;]+)"#,
                "${key}${2}***",
                true,
            ),
        ]
        .into_iter()
        .map(|(re, rep, keyed)| MaskRule {
            re: regex::Regex::new(re).expect("static regex"),
            rep,
            keyed,
        })
        .collect()
    })
}

/// Apply a keyed rule: mask matches whose `key` group names a secret. After a
/// non-secret key the scan resumes right past the key, so a secret inside its
/// value (`url: password=x`) is still found.
fn apply_keyed(re: &regex::Regex, rep: &str, text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(caps) = re.captures_at(text, pos) {
        let (Some(whole), Some(key)) = (caps.get(0), caps.name("key")) else {
            break;
        };
        out.push_str(&text[pos..whole.start()]);
        if is_secret_key(key.as_str()) {
            caps.expand(rep, &mut out);
            pos = whole.end();
        } else {
            out.push_str(&text[whole.start()..key.end()]);
            pos = key.end();
        }
    }
    out.push_str(&text[pos..]);
    out
}

/// Replace high-precision secret shapes in free text (trace payloads) with
/// [`SECRET_MASK`], leaving the rest intact. Deliberately does not use the
/// entropy lookalike detector: it flags git SHAs, UUIDs and hashes, which
/// traces must keep. Idempotent: masking an already masked text is a no-op.
pub fn mask_secrets(text: &str) -> String {
    let mut out = text.to_owned();
    for rule in mask_rules() {
        if rule.re.is_match(&out) {
            out = if rule.keyed {
                apply_keyed(&rule.re, rule.rep, &out)
            } else {
                rule.re.replace_all(&out, rule.rep).into_owned()
            };
        }
    }
    out
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

    /// Дефект 3 (найдено 11.09.2026, subject
    /// `aurelius:crates/aurelius-core/src/secret.rs:free-text-safe-shapes`,
    /// измерено против установленного бинаря с изолированным AURELIUS_HOME):
    /// git-хэш (SHA-1, 40 hex) ложно отказывал в свободном тексте.
    #[test]
    fn git_sha1_hash_in_free_text_is_accepted() {
        for text in [
            "commit 7b86a7d98517479bbcd10998e74b292d763159dd fixed it",
            "see ff202359c32a6819358a9e9636b2284b98387c5f for the diff",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "git-хэш отклонён как секрет: {text}"
            );
        }
    }

    /// Та же находка: голый UUID уже проходил как slug (строчный hex +
    /// дефисы), но обёрточные скобки ломают именно эту форму раньше, чем до
    /// неё доходит проверка.
    #[test]
    fn parenthesized_uuid_in_free_text_is_accepted() {
        assert_eq!(
            scan_text_for_lookalike("session id (449adf4b-26f9-4273-9e18-e16e638185f3) attached"),
            None,
            "UUID в скобках отклонён как секрет"
        );
    }

    /// Та же находка: `Тип.метод`/`Тип.метод()` — верхний регистр имени типа
    /// и нижний имени метода дают ровно два класса символов, тот же признак,
    /// что у случайного токена.
    #[test]
    fn qualified_code_identifier_in_free_text_is_accepted() {
        for text in [
            "SocketBusClient.request failed with exit13",
            "SocketBusClient.request() failed with exit13",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "квалифицированный идентификатор отклонён как секрет: {text}"
            );
        }
    }

    /// Асимметрия: обёрточная пунктуация сама по себе поблажки не даёт —
    /// обычный случайный токен под скобками отказывает так же, как без них.
    #[test]
    fn wrapped_generic_random_token_is_still_rejected() {
        assert!(matches!(
            scan_text_for_lookalike("leaked: (aZ9bQ7mK2xR5vN8pL1wT4) rotate it"),
            Some((SecretLookalike::RandomToken, _))
        ));
    }

    /// Асимметрия: обёрнутый известный префикс по-прежнему ловится —
    /// подстрочный поиск префикса по всему тексту обёрткой не задет.
    #[test]
    fn wrapped_known_prefix_token_is_still_rejected() {
        assert!(matches!(
            scan_text_for_lookalike("token (ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789) leaked"),
            Some((SecretLookalike::KnownPrefix("ghp_"), _))
        ));
    }

    /// Асимметрия: `detect_lookalike` (координата секрета, `--where`)
    /// поблажки для хэш-формы не получает — форма хэша неотличима от
    /// опакового ключа такой же формы, а цена ошибки здесь выше, чем в
    /// свободном тексте (см. doc-комментарий `HEX_HASH_LENS`).
    #[test]
    fn detect_lookalike_does_not_exempt_hash_shaped_coordinates() {
        assert_eq!(
            detect_lookalike("7b86a7d98517479bbcd10998e74b292d763159dd"),
            Some(SecretLookalike::RandomToken)
        );
    }

    /// Сегмент, сам по себе достаточно длинный, чтобы выглядеть отдельным
    /// токеном (как base64-часть JWT: `header.payload.signature`), не
    /// проходит под видом идентификатора — граница, которая отделяет
    /// `Тип.метод` от `xxx.yyy.zzz` ([`is_identifier_segment`]). Части
    /// собраны `concat!`, чтобы в исходнике не лежал цельный
    /// credential-похожий литерал.
    #[test]
    fn long_dot_joined_segments_are_not_mistaken_for_a_qualified_identifier() {
        let header = concat!("eyJhbGciOiJIUzI1", "NiIsInR5cCI6IkpXVCJ9");
        let payload = concat!("eyJzdWIiOiIxMjM0", "NTY3ODkwIn0");
        let signature = concat!("dozjgNryP4J3jVmN", "Hl0w5N_XgL0n3I9PlFUP0THsR8U");
        let jwt_shaped = format!("{header}.{payload}.{signature}");
        assert!(matches!(
            scan_text_for_lookalike(&jwt_shaped),
            Some((SecretLookalike::RandomToken, _))
        ));
    }

    /// Приёмка 2 (11.09.2026, родитель): хэш в бэктиках не проходил, пока
    /// `strip_wrapping_punctuation` снимала только одну пару и не знала
    /// бектики.
    #[test]
    fn backtick_wrapped_hash_in_free_text_is_accepted() {
        assert_eq!(
            scan_text_for_lookalike("see `7b86a7d98517479bbcd10998e74b292d763159dd` for the diff"),
            None,
            "хэш в бэктиках отклонён как секрет"
        );
    }

    /// Приёмка 2: обычная пунктуация конца предложения, приклеенная к уже
    /// распознаваемой безопасной форме, не должна была её ломать.
    #[test]
    fn sentence_punctuation_after_safe_shapes_is_accepted() {
        for text in [
            "SocketBusClient.request, and it failed",
            "session id (449adf4b-26f9-4273-9e18-e16e638185f3).",
            "commit 7b86a7d98517479bbcd10998e74b292d763159dd.",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "форма с пунктуацией отклонена как секрет: {text}"
            );
        }
    }

    /// Приёмка 2: `callee(arg)` — простой вызов, не значение (живой репро:
    /// `createMapper(claude)`, 20 символов, отказал в провенанс-поле).
    #[test]
    fn simple_call_with_identifier_argument_is_accepted() {
        assert_eq!(
            scan_text_for_lookalike("failed inside createMapper(claude) during init"),
            None,
            "простой вызов отклонён как секрет"
        );
    }

    /// Асимметрия: callee или аргумент, сам по себе длинный/случайный, — уже
    /// не «простой вызов», а секрет со скобками; обязан отказать так же, как
    /// без скобок.
    #[test]
    fn simple_call_with_long_or_random_parts_is_still_rejected() {
        assert!(matches!(
            scan_text_for_lookalike("failed inside aZ9bQ7mK2xR5vN8pL1wT4(claude) during init"),
            Some((SecretLookalike::RandomToken, _))
        ));
        assert!(matches!(
            scan_text_for_lookalike(
                "failed inside createMapper(aZ9bQ7mK2xR5vN8pL1wT4) during init"
            ),
            Some((SecretLookalike::RandomToken, _))
        ));
    }

    /// Асимметрия: вложенная обёртка (скобки + бэктики) и приклеенная
    /// пунктуация конца предложения на обычном случайном токене поблажки
    /// по-прежнему не дают.
    #[test]
    fn nested_wrapper_and_sentence_punctuation_do_not_exempt_a_random_token() {
        assert!(matches!(
            scan_text_for_lookalike("leaked: (`aZ9bQ7mK2xR5vN8pL1wT4`). rotate it"),
            Some((SecretLookalike::RandomToken, _))
        ));
    }

    /// Найдено 14.09.2026 (subject
    /// `aurelius:crates/aurelius-core/src/secret.rs:camelcase-identifier`):
    /// голое camelCase-имя длиннее порога ловилось как случайный токен.
    /// Первые две строки — дословные подстроки двух заметок про
    /// dsh-russian-lang, отказавших на смещении 55 и 384 байт.
    #[test]
    fn camel_case_code_identifier_in_free_text_is_accepted() {
        for text in [
            "the single call site of translateTurnContent sits inside the onClick",
            "the two fetch calls are the only network calls in the bundle",
            "field actualReceiveAmountUsdt is written before the callback",
            "resolveCardCommission is called before the write",
            "globalShortcutReleased and globalShortcutRepeated fired",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "camelCase-имя отклонено как секрет: {text}"
            );
        }
        // Само имя — тоже кандидат, а не только фраза вокруг него.
        assert_eq!(scan_text_for_lookalike("translateTurnContent"), None);
    }

    /// Асимметрия: поблажка даётся рисунку регистра, а не «длинной строке из
    /// букв». Строка той же длины без camelCase-отрезков (строчные идут по
    /// одной между заглавными) остаётся случайным токеном, цифра снимает
    /// поблажку так же. Дефисный слаг в этот список не входит: он принимается
    /// раньше и по своему признаку ([`looks_like_slug`]).
    #[test]
    fn mixed_case_word_without_camel_runs_is_still_rejected() {
        for text in [
            "leaked: aZbQmKxRvNpLwTsHdGfQr rotate it",
            "leaked: aZ9bQ7mK2xR5vN8pL1wT4 rotate it",
            "leaked: AntigravityModelsClaudeOpus rotate it",
        ] {
            assert!(
                matches!(
                    scan_text_for_lookalike(text),
                    Some((SecretLookalike::RandomToken, _))
                ),
                "случайная строка принята как безопасная форма: {text}"
            );
        }
    }

    /// Асимметрия, как у хэш-формы: `detect_lookalike` (координата секрета,
    /// `--where`) поблажки для camelCase-имени не получает — там цена ошибки
    /// выше, а форма имени неотличима от опакового ключа той же длины.
    #[test]
    fn detect_lookalike_does_not_exempt_code_identifier_shaped_coordinates() {
        assert_eq!(
            detect_lookalike("translateTurnContent"),
            Some(SecretLookalike::RandomToken)
        );
    }

    /// Заказ 14.09.2026: полезная нагрузка из отчёта — заметка о локализации
    /// интерфейса DSH — обязана проходить рубеж целиком, а не только её
    /// отдельные слова. Строки ниже — дословные длинные токены этой заметки и
    /// её провенанс-полей.
    #[test]
    fn localization_note_long_tokens_are_accepted() {
        for text in [
            "форк с русским словарём в апстрим 02Muller25/dsh-api-balance.",
            "смена источника api-balance и рестарт systemd-юнита dsh-web.service.",
            "subject deepseek-harness:web-gui:localization-ru",
            "evidence git -C workSpace/project/dsh-api-balance log --oneline",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "токен заметки отклонён как секрет: {text}"
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

    #[test]
    fn mask_secrets_masks_every_shape() {
        let cases = [
            (
                "git push https://ghp_abcdefghijklmnopqrstuvwxyz0123@github.com",
                "ghp_",
            ),
            ("export T=gho_ABCDEFGHIJKLMNOPQRST1234", "gho_"),
            ("x ghu_ABCDEFGHIJKLMNOPQRST1234", "ghu_"),
            ("x ghs_ABCDEFGHIJKLMNOPQRST1234", "ghs_"),
            ("x ghr_ABCDEFGHIJKLMNOPQRST1234", "ghr_"),
            ("x github_pat_11ABCDEFG0123456789_abcdefXYZ", "github_pat_"),
            ("OPENAI=sk-proj1234567890abcdefghij run", "sk-proj"),
            ("x sk-ant-api03-abcdefghijklmnop1234", "sk-ant-api03"),
            ("slack xoxb-1234567890-abcdefghij", "xoxb-"),
            ("slack xoxp-1234567890-abcdefghij", "xoxp-"),
            ("slack xoxa-1234567890-abcdefghij", "xoxa-"),
            ("aws AKIAIOSFODNN7EXAMPLE", "AKIAIOSFODNN7"),
            ("gl glpat-abcdefghij0123456789", "glpat-"),
            (
                "curl -H 'Authorization: Bearer abc.def.ghi' u",
                "abc.def.ghi",
            ),
            (
                "curl -H \"Authorization: Basic dXNlcjpwYXNz\" u",
                "dXNlcjpwYXNz",
            ),
            ("curl -H 'X: bearer opaqueTok123' u", "opaqueTok123"),
            ("psql --password=hunter2 db", "hunter2"),
            ("PGPASSWORD='s3cr et' psql", "s3cr et"),
            ("DB_PASSWD=abc123 run", "abc123"),
            ("mysql pwd=qwerty", "qwerty"),
            ("gh auth --token=zzz999", "zzz999"),
            ("client_secret: mysecretvalue", "mysecretvalue"),
            ("API_KEY=k123 APIKEY=k456", "k123"),
            ("apikey=k456", "k456"),
            ("aws_access_key=AKZZ12", "AKZZ12"),
            ("private_key: \"-----BEGIN\"", "BEGIN"),
            (
                "psql postgres://admin:Pa55word@db.local:5432/app",
                "Pa55word",
            ),
            (r#"{"password": "jsonpw1"}"#, "jsonpw1"),
            ("{'token': 'dicttok2'}", "dicttok2"),
            (r#"{"api_key":"zkey3"}"#, "zkey3"),
            (r#"{"secret": 12345}"#, "12345"),
            ("mysql --password hunter4 db", "hunter4"),
            ("gh --token tok5abc", "tok5abc"),
            ("vault --secret 'sp ace6'", "sp ace6"),
            ("cli --api-key key7xyz run", "key7xyz"),
        ];
        for (input, secret) in cases {
            let masked = mask_secrets(input);
            assert!(!masked.contains(secret), "{input} -> {masked}");
            assert!(masked.contains(SECRET_MASK), "{input} -> {masked}");
        }
        assert_eq!(
            mask_secrets("psql postgres://admin:Pa55word@db.local:5432/app"),
            "psql postgres://admin:***@db.local:5432/app"
        );
        assert_eq!(
            mask_secrets("psql --password=hunter2 db"),
            "psql --password=*** db"
        );
        assert_eq!(
            mask_secrets(r#"{"password": "x", "user": "u"}"#),
            r#"{"password": "***", "user": "u"}"#
        );
        assert_eq!(mask_secrets("{'token': 'y'}"), "{'token': '***'}");
        assert_eq!(mask_secrets(r#"{"api_key":"z"}"#), r#"{"api_key":"***"}"#);
        assert_eq!(
            mask_secrets("mysql --password hunter4 db"),
            "mysql --password *** db"
        );
        assert_eq!(
            mask_secrets("cli --token --verbose"),
            "cli --token --verbose"
        );
    }

    #[test]
    fn mask_secrets_keeps_technical_shapes_and_is_idempotent() {
        for input in [
            "git show 6784399a1b2c3d4e5f60718293a4b5c6d7e8f901",
            "au task show 20b2bd3b-4946-452b-920c-9b5f2e70190e",
            "cat /home/u/crates/aurelius-core/src/secret.rs",
            "cargo test --workspace -- --nocapture",
            "sha256sum file | grep e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "git clone https://github.com/org/repo.git",
            "task-management-service-long-name",
        ] {
            assert_eq!(mask_secrets(input), input);
        }
        let once = mask_secrets(
            "curl -H 'Authorization: Bearer x' --password=y ghp_abcdefghijklmnopqrstuv \
             --token t {\"secret\": \"s\", 'pwd': 'p', \"api_key\": 5}",
        );
        assert_eq!(mask_secrets(&once), once);
    }

    #[test]
    fn secret_keys_are_judged_by_whole_words() {
        for key in [
            "access_token",
            "GITHUB_TOKEN",
            "x-api-key",
            "clientSecret",
            "PGPASSWORD",
        ] {
            assert!(is_secret_key(key), "{key}");
            for input in [
                format!("{key}=v4lue"),
                format!("{key}: v4lue"),
                format!(r#"{{"{key}": "v4lue"}}"#),
                format!("cmd --{key} v4lue"),
            ] {
                let masked = mask_secrets(&input);
                assert!(!masked.contains("v4lue"), "{input} -> {masked}");
            }
        }
        for key in [
            "max_tokens",
            "tokens_used",
            "tokenizer",
            "secretary_name",
            "passport",
        ] {
            assert!(!is_secret_key(key), "{key}");
            for input in [
                format!("{key}=4096"),
                format!("{key}: 4096"),
                format!(r#"{{"{key}": 4096}}"#),
                format!(r#"{{"{key}": "4096"}}"#),
                format!("cmd --{key} 4096"),
            ] {
                assert_eq!(mask_secrets(&input), input);
            }
        }
        assert_eq!(mask_secrets("url: password=x"), "url: password=***");
    }

    // Дефект 5 (найдено 13-14.09.2026, subject
    // `aurelius:write:secret-guard:false-positives`): установленный бинарь
    // отказал четырём живым заметкам — ниже технические формы текста,
    // обязанные проходить без `--allow-secret`; фикстуры — дословные
    // подстроки отказанных заметок.

    /// Репро 5а: имя workflow — `sk-` внутри слова `xhub-task-hygiene`.
    #[test]
    fn task_hygiene_workflow_name_is_accepted() {
        assert_eq!(
            scan_text_for_lookalike(
                "Task hygiene workflow wf_77b967a8-bb2 (xhub-task-hygiene) stopped"
            ),
            None,
            "имя workflow со sk- внутри слова отклонено как секрет"
        );
    }

    /// Репро 5б: путь к скрипту workflow с UUID и хвостом
    /// `task-hygiene-wf_77b967a8-bb2.js` — хвост после `sk-` длиннее
    /// `RANDOM_TOKEN_MIN_LEN` и с цифрами, так что одного `token_body_follows`
    /// уже недостаточно; спасает только требование границы слова для `sk-`,
    /// см. `prefix_requires_word_boundary`.
    #[test]
    fn workflow_script_path_with_uuid_is_accepted() {
        assert_eq!(
            scan_text_for_lookalike(
                "Resume: Workflow scriptPath ~/.claude/projects/-home-blyss-workSpace-project-xhub/2d4dafc2-3502-4bf9-a07c-eb8d05a7dc8d/workflows/scripts/xhub-task-hygiene-wf_77b967a8-bb2.js"
            ),
            None,
            "путь к скрипту workflow отклонён как секрет"
        );
    }

    /// Репро 5в (14.09.2026): два дословных варианта одной заметки, второй
    /// обрезан на `claude-sonnet-4-6`; список идентификаторов моделей — слаги
    /// из слов и номеров версий, не ключи.
    #[test]
    fn model_slug_lists_are_accepted() {
        for text in [
            "agy 2026-09-14: Antigravity models claude-sonnet-4-6, claude-opus-4-6-thinking, gpt-oss-120b-medium answer headless",
            "au note \"agy 2026-09-14: Antigravity models claude-sonnet-4-6",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "список моделей отклонён как секрет: {text}"
            );
        }
    }

    /// Репро 5г: диапазон git-коммитов в скобках с запятой на хвосте; вторая
    /// строка — та же форма из двух полных 40-hex sha: короткая проходит
    /// лишь потому, что целиком короче `RANDOM_TOKEN_MIN_LEN`, — поблажка
    /// не должна зависеть от длины заглушек.
    #[test]
    fn git_commit_ranges_are_accepted() {
        for text in [
            "029 wave 6 closed 13.09: T081 T082 T083 T087 T089 committed in feat/provider-tariff-core (08cf458e..3ffb349e), tree clean",
            "range 7b86a7d98517479bbcd10998e74b292d763159dd..ff202359c32a6819358a9e9636b2284b98387c5f closed",
        ] {
            assert_eq!(
                scan_text_for_lookalike(text),
                None,
                "диапазон git-коммитов отклонён как секрет: {text}"
            );
        }
    }

    /// Асимметрия дефекта 5 — матрица «обязан отказать» из той же задачи:
    /// настоящий `sk-proj`-токен (отдельным словом и сразу после знака
    /// равенства), GitHub-токен, пара AWS access key id + secret access key и
    /// случайная 40-символьная base62-строка без разделителей; последняя —
    /// ровно форма AWS secret access key (40 символов base64-алфавита с
    /// обоими регистрами и цифрой) и отказывает именно по ней: не слаг и не
    /// hex.
    #[test]
    fn real_secret_shapes_are_still_refused() {
        assert!(matches!(
            scan_text_for_lookalike("token sk-proj-abc123def456ghi789jkl012mno345"),
            Some((SecretLookalike::KnownPrefix("sk-"), _))
        ));
        assert!(matches!(
            scan_text_for_lookalike("OPENAI_API_KEY=sk-proj-abc123def456ghi789jkl012mno345"),
            Some((SecretLookalike::KnownPrefix("sk-"), _))
        ));
        assert!(matches!(
            scan_text_for_lookalike("token: ghp_16C7e42F292c6912E7710c838347Ae178B4a"),
            Some((SecretLookalike::KnownPrefix("ghp_"), _))
        ));
        // AWS access key id без достаточно длинного хвоста после префикса
        // ловится формой (20 символов, верхний регистр + цифра), секрет —
        // собственной 40-символьной формой со слэшем в теле.
        assert!(matches!(
            scan_text_for_lookalike("AKIAIOSFODNN7EXAMPLE"),
            Some((SecretLookalike::RandomToken, _))
        ));
        assert!(matches!(
            scan_text_for_lookalike("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            Some((SecretLookalike::AwsSecretKeyShape, _))
        ));
        // Случайный base62 без разделителей: не слаг, не hex, не путь.
        assert!(matches!(
            scan_text_for_lookalike(concat!(
                "leaked aZ9bQ7mK2xR5vN8pL1wT",
                "4Q9zK3mN6bV8cX5dF2gH"
            )),
            Some((SecretLookalike::AwsSecretKeyShape, _))
        ));
    }
}
