//! Личные финансы владельца: сколько, в какой валюте, на что.
//!
//! Решения владельца (задача d4eb36dd, 15.10.2026), зашитые в схему
//! (`db::ensure_finance_schema`):
//!
//! 1. Деньги — целое число минимальных единиц валюты, точность берётся из
//!    справочника `fin_currencies`. Лишний знак после запятой — ошибка, а не
//!    округление: округлённая сумма — уже не та, что сказал владелец.
//! 2. Обмен (`exchange`) хранит обе стороны: отдал `amount`/`currency`,
//!    получил `counter_amount`/`counter_currency`. Фактический курс выводится
//!    из них и не хранится. Обмен никогда не считается ни расходом, ни доходом.
//! 3. Счетов, карт и мест нет — только сумма, валюта, категория. Упоминание
//!    карты при желании уходит в `note`.
//! 4. Нет жёсткого удаления: исправление — это отмена (`voided_at` +
//!    `void_reason`) и новая запись. Отменённая строка остаётся и в сводки не
//!    входит.
//! 5. Сводки — по каждой валюте отдельно; пересчёт в USD по курсу на дату
//!    операции — в `finance_rates` (поле `usd` у записи и у сводки).

use std::collections::{BTreeMap, HashMap};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::db::ensure_finance_schema;

/// Предел точности валюты: 8 знаков — сатоши, дальше не нужно никому из
/// того, с чем владелец имеет дело.
pub const MAX_SCALE: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Expense,
    Income,
    Exchange,
}

impl Kind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Expense => "expense",
            Kind::Income => "income",
            Kind::Exchange => "exchange",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "expense" => Some(Kind::Expense),
            "income" => Some(Kind::Income),
            "exchange" => Some(Kind::Exchange),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Expense => "расход",
            Kind::Income => "доход",
            Kind::Exchange => "обмен",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Voice,
    Text,
    Manual,
    Import,
}

impl Source {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Voice => "voice",
            Source::Text => "text",
            Source::Manual => "manual",
            Source::Import => "import",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "voice" => Some(Source::Voice),
            "text" => Some(Source::Text),
            "manual" => Some(Source::Manual),
            "import" => Some(Source::Import),
            _ => None,
        }
    }
}

/// Строка справочника `fin_currencies`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Currency {
    pub code: String,
    /// `fiat` | `crypto`
    pub kind: String,
    /// Знаков после запятой: 1 единица = 10^scale минимальных.
    pub scale: u32,
    pub enabled: bool,
}

/// Фактический курс обмена: 1 `base` = `value` `quote`. Сторона выбирается
/// так, чтобы `value` было не меньше единицы — «1 TON = 129.31 RUB», а не
/// «1 RUB = 0.0077 TON».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rate {
    pub base: String,
    pub quote: String,
    pub value: String,
}

/// Запись журнала вместе с тем, что из неё выводится для чтения человеком.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    pub kind: Kind,
    pub amount_minor: i64,
    /// `amount_minor` в единицах валюты, десятичной строкой без хвостовых нулей.
    pub amount: String,
    pub currency: String,
    pub counter_amount_minor: Option<i64>,
    pub counter_amount: Option<String>,
    pub counter_currency: Option<String>,
    pub category: Option<String>,
    pub note: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub source: Source,
    pub provenance: Option<String>,
    pub created_at: DateTime<Utc>,
    pub voided_at: Option<DateTime<Utc>>,
    pub void_reason: Option<String>,
    pub rate: Option<Rate>,
    /// Эквивалент в USD с курсом и источником; `None` — не пересчитывалась
    /// или курса нет (`finance_rates::convert_entries`).
    pub usd: Option<crate::finance_rates::Conversion>,
    /// Одна строка для подтверждения в боте.
    pub line: String,
}

/// Данные одной новой записи — суммы ещё строками: разбор в минимальные
/// единицы требует точности валюты, а она живёт в базе.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub kind: Kind,
    pub amount: String,
    pub currency: String,
    pub counter_amount: Option<String>,
    pub counter_currency: Option<String>,
    pub category: Option<String>,
    pub note: Option<String>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub source: Source,
    pub provenance: Option<String>,
}

// ---------------------------------------------------------------------------
// Суммы
// ---------------------------------------------------------------------------

fn pow10(exp: u32) -> i64 {
    10_i64.pow(exp.min(MAX_SCALE))
}

/// Десятичная строка → минимальные единицы. Принимает `25`, `25.5`, `25,5`,
/// пробелы и `_` как разделители разрядов. Знак, экспонента, ноль и больше
/// знаков после запятой, чем `scale`, — ошибка: округлять сказанное нельзя.
pub fn parse_amount(raw: &str, scale: u32) -> Result<i64> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '\u{a0}' | '\u{202f}'))
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    // «1,000»: тысяча или единица с тремя знаками? Угадывать нельзя.
    if let Some((int, frac)) = raw.trim().split_once(',') {
        let digits = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
        if digits(int) && int != "0" && frac.len() == 3 && digits(frac) {
            bail!(
                "сумма '{raw}' неоднозначна: запятая с тремя цифрами — тысячи или дробь? \
                 уточни у владельца и передай без разделителя тысяч ('1000') или с точкой ('1.000')"
            );
        }
    }
    let (int_part, frac_part) = match cleaned.split_once('.') {
        Some((i, f)) => (i, f),
        None => (cleaned.as_str(), ""),
    };
    let digits_only = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if cleaned.is_empty()
        || (int_part.is_empty() && frac_part.is_empty())
        || !digits_only(int_part)
        || !digits_only(frac_part)
    {
        bail!("сумма '{raw}' не число: нужны цифры и не больше одной точки или запятой");
    }
    let frac_len = u32::try_from(frac_part.len()).unwrap_or(u32::MAX);
    if frac_len > scale {
        bail!(
            "сумма '{raw}': {frac_len} знаков после запятой, а точность валюты — {scale}; \
             округлять не буду, уточни сумму"
        );
    }
    let overflow = || anyhow!("сумма '{raw}' слишком велика");
    let int_value: i64 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().map_err(|_| overflow())?
    };
    let frac_value: i64 = if frac_part.is_empty() {
        0
    } else {
        frac_part.parse().map_err(|_| overflow())?
    };
    let minor = int_value
        .checked_mul(pow10(scale))
        .and_then(|v| v.checked_add(frac_value.checked_mul(pow10(scale - frac_len))?))
        .ok_or_else(overflow)?;
    if minor <= 0 {
        bail!("сумма '{raw}' должна быть больше нуля");
    }
    Ok(minor)
}

/// Сумма или курс из JSON-параметра: строка — как есть, целое JSON-число —
/// точно; дробное JSON-число — ошибка: оно уже прошло через f64 (`1e-8`,
/// `...064007`), а округлять сказанное нельзя.
pub fn amount_from_json(v: &serde_json::Value, key: &str) -> Result<Option<String>> {
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => Ok(Some(s.clone())),
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => Ok(Some(n.to_string())),
        serde_json::Value::Number(n) => bail!(
            "'{key}': JSON number {n} is not exact - pass the amount as a decimal string, \
             e.g. \"0.00000001\" (дробное JSON-число прошло через f64; передай строкой)"
        ),
        other => bail!("'{key}' must be a decimal string, got {other}"),
    }
}

/// Минимальные единицы → десятичная строка без хвостовых нулей: `11.6`, `25`.
#[must_use]
pub fn format_amount(minor: i64, scale: u32) -> String {
    let scale = scale.min(MAX_SCALE);
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    let unit = 10_u64.pow(scale);
    let int_part = abs / unit;
    let frac = abs % unit;
    if scale == 0 || frac == 0 {
        return format!("{sign}{int_part}");
    }
    let frac = format!("{frac:0width$}", width = scale as usize);
    format!("{sign}{int_part}.{}", frac.trim_end_matches('0'))
}

/// Частное `num / den` десятичной строкой с `digits` знаками после запятой,
/// округление половины вверх, хвостовые нули срезаны. Только для показа:
/// курс не хранится.
fn ratio(num: i128, den: i128, digits: u32) -> String {
    if den == 0 {
        return "∞".to_owned();
    }
    let unit = 10_i128.pow(digits);
    let scaled = (num * unit * 2 + den) / (den * 2);
    let int_part = scaled / unit;
    let frac = scaled % unit;
    if frac == 0 {
        return int_part.to_string();
    }
    let frac = format!("{frac:0width$}", width = digits as usize);
    format!("{int_part}.{}", frac.trim_end_matches('0'))
}

/// Фактический курс обмена из двух его сторон.
#[must_use]
pub fn effective_rate(
    amount_minor: i64,
    currency: &str,
    scale: u32,
    counter_minor: i64,
    counter_currency: &str,
    counter_scale: u32,
) -> Rate {
    // given = amount / 10^scale, got = counter / 10^counter_scale.
    // 1 got-unit = given/got = amount * 10^cs / (counter * 10^s) given-units.
    let a = i128::from(amount_minor) * i128::from(pow10(counter_scale));
    let c = i128::from(counter_minor) * i128::from(pow10(scale));
    if a >= c {
        Rate {
            base: counter_currency.to_owned(),
            quote: currency.to_owned(),
            value: ratio(a, c, 4),
        }
    } else {
        Rate {
            base: currency.to_owned(),
            quote: counter_currency.to_owned(),
            value: ratio(c, a, 4),
        }
    }
}

// ---------------------------------------------------------------------------
// Время
// ---------------------------------------------------------------------------

fn from_unix(secs: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(secs, 0).unwrap_or_default()
}

fn local_midnight(date: NaiveDate) -> Option<DateTime<Utc>> {
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Когда случилась трата: RFC 3339, `ГГГГ-ММ-ДД ЧЧ:ММ` или `ГГГГ-ММ-ДД`
/// (тогда полдень по местному времени — чтобы сдвиг пояса не перенёс запись
/// на соседний день).
pub fn parse_occurred(spec: &str) -> Result<DateTime<Utc>> {
    let spec = spec.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(spec) {
        return Ok(dt.with_timezone(&Utc));
    }
    let naive = if let Ok(n) = NaiveDateTime::parse_from_str(spec, "%Y-%m-%d %H:%M") {
        Some(n)
    } else if let Ok(d) = NaiveDate::parse_from_str(spec, "%Y-%m-%d") {
        d.and_hms_opt(12, 0, 0)
    } else {
        None
    };
    naive
        .and_then(|n| Local.from_local_datetime(&n).earliest())
        .map(|dt| dt.with_timezone(&Utc))
        .ok_or_else(|| {
            anyhow!("момент '{spec}' не разобран: RFC 3339, ГГГГ-ММ-ДД ЧЧ:ММ или ГГГГ-ММ-ДД")
        })
}

/// Полуинтервал `[since, until)` по местному календарю.
#[derive(Debug, Clone, Serialize)]
pub struct Period {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub label: String,
}

fn parse_bound(spec: &str) -> Result<DateTime<Utc>> {
    let spec = spec.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(spec) {
        return Ok(dt.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(spec, "%Y-%m-%d")
        .ok()
        .and_then(local_midnight)
        .ok_or_else(|| anyhow!("граница '{spec}' не разобрана: ГГГГ-ММ-ДД или RFC 3339"))
}

/// Период: `today`, `yesterday`, `week` (с понедельника), `month` (с 1-го),
/// `year`, `all`, `<N>d` (последние N дней, включая сегодня), либо явные
/// `since`/`until` (дата — местная полночь, `until` не включается). Явные
/// границы перекрывают соответствующую границу `period`. Ничего не задано —
/// текущий месяц.
pub fn resolve_period(
    period: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Period> {
    let today = now.with_timezone(&Local).date_naive();
    let midnight = |d: NaiveDate| {
        local_midnight(d).ok_or_else(|| anyhow!("местная полночь {d} не существует"))
    };
    let spec = period.map(|p| p.trim().to_lowercase()).unwrap_or_else(|| {
        if since.is_some() || until.is_some() {
            "all".into()
        } else {
            "month".into()
        }
    });
    let mut out = match spec.as_str() {
        "today" => Period {
            since: Some(midnight(today)?),
            until: None,
            label: "today".into(),
        },
        "yesterday" => {
            let y = today.pred_opt().ok_or_else(|| anyhow!("нет вчера"))?;
            Period {
                since: Some(midnight(y)?),
                until: Some(midnight(today)?),
                label: "yesterday".into(),
            }
        }
        "week" => {
            let back = i64::from(today.weekday().num_days_from_monday());
            Period {
                since: Some(midnight(today - Duration::days(back))?),
                until: None,
                label: "week".into(),
            }
        }
        "month" => {
            let first = today.with_day(1).ok_or_else(|| anyhow!("нет 1-го числа"))?;
            Period {
                since: Some(midnight(first)?),
                until: None,
                label: format!("month {}", first.format("%Y-%m")),
            }
        }
        "year" => {
            let first = NaiveDate::from_ymd_opt(today.year(), 1, 1)
                .ok_or_else(|| anyhow!("нет 1 января"))?;
            Period {
                since: Some(midnight(first)?),
                until: None,
                label: format!("year {}", today.year()),
            }
        }
        "all" => Period {
            since: None,
            until: None,
            label: "all".into(),
        },
        other => {
            let days: i64 = other
                .strip_suffix('d')
                .and_then(|n| n.parse().ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| {
                    anyhow!(
                        "период '{other}': today | yesterday | week | month | year | all | <N>d"
                    )
                })?;
            Period {
                since: Some(midnight(today - Duration::days(days - 1))?),
                until: None,
                label: format!("{days}d"),
            }
        }
    };
    if let Some(s) = since {
        out.since = Some(parse_bound(s)?);
    }
    if let Some(u) = until {
        out.until = Some(parse_bound(u)?);
    }
    if since.is_some() || until.is_some() {
        out.label = format!(
            "{} .. {}",
            since.unwrap_or("…").trim(),
            until.unwrap_or("…").trim()
        );
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Валюты
// ---------------------------------------------------------------------------

fn row_to_currency(row: &rusqlite::Row) -> rusqlite::Result<Currency> {
    Ok(Currency {
        code: row.get(0)?,
        kind: row.get(1)?,
        scale: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
    })
}

fn get_currency(conn: &Connection, code: &str) -> Result<Option<Currency>> {
    Ok(conn
        .query_row(
            "SELECT code, kind, scale, enabled FROM fin_currencies WHERE code = ?1",
            [code],
            row_to_currency,
        )
        .optional()?)
}

/// Код валюты в каноническом виде: верхний регистр, 2–10 латинских букв и
/// цифр.
pub fn normalize_code(raw: &str) -> Result<String> {
    let code = raw.trim().to_ascii_uppercase();
    if !(2..=10).contains(&code.len()) || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("код валюты '{raw}': 2–10 латинских букв или цифр");
    }
    Ok(code)
}

pub fn currencies(conn: &Connection, include_disabled: bool) -> Result<Vec<Currency>> {
    ensure_finance_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT code, kind, scale, enabled FROM fin_currencies
         WHERE ?1 OR enabled = 1 ORDER BY kind DESC, code",
    )?;
    let rows = stmt.query_map([include_disabled], row_to_currency)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Завести валюту или включить отключённую. Точность существующей валюты
/// не меняется: она задаёт смысл уже записанных минимальных единиц.
pub fn add_currency(conn: &Connection, code: &str, kind: &str, scale: u32) -> Result<Currency> {
    ensure_finance_schema(conn)?;
    let code = normalize_code(code)?;
    if !matches!(kind, "fiat" | "crypto") {
        bail!("вид валюты '{kind}': fiat | crypto");
    }
    if scale > MAX_SCALE {
        bail!("точность {scale}: не больше {MAX_SCALE}");
    }
    if let Some(existing) = get_currency(conn, &code)? {
        if existing.scale != scale || existing.kind != kind {
            bail!(
                "валюта {code} уже есть ({}, точность {}); точность не меняется — \
                 на ней держатся записанные суммы",
                existing.kind,
                existing.scale
            );
        }
        if existing.enabled {
            bail!("валюта {code} уже есть и включена");
        }
        conn.execute(
            "UPDATE fin_currencies SET enabled = 1 WHERE code = ?1",
            [&code],
        )?;
    } else {
        conn.execute(
            "INSERT INTO fin_currencies (code, kind, scale, enabled) VALUES (?1, ?2, ?3, 1)",
            params![code, kind, scale],
        )?;
    }
    get_currency(conn, &code)?.ok_or_else(|| anyhow!("валюта {code} пропала"))
}

/// Отключить валюту: новые записи в ней не принимаются, старые остаются и
/// видны в сводках.
pub fn disable_currency(conn: &Connection, code: &str) -> Result<Currency> {
    ensure_finance_schema(conn)?;
    let code = normalize_code(code)?;
    let changed = conn.execute(
        "UPDATE fin_currencies SET enabled = 0 WHERE code = ?1 AND enabled = 1",
        [&code],
    )?;
    if changed == 0 {
        bail!("валюты {code} нет или она уже отключена");
    }
    get_currency(conn, &code)?.ok_or_else(|| anyhow!("валюта {code} пропала"))
}

fn enabled_currency(conn: &Connection, raw: &str) -> Result<Currency> {
    let code = normalize_code(raw)?;
    match get_currency(conn, &code)? {
        Some(c) if c.enabled => Ok(c),
        Some(_) => bail!("валюта {code} отключена — уточни у владельца, включать ли её снова"),
        None => bail!(
            "неизвестная валюта {code} — спроси владельца; заводить её без его подтверждения нельзя"
        ),
    }
}

// ---------------------------------------------------------------------------
// Записи
// ---------------------------------------------------------------------------

const ENTRY_SELECT: &str = "SELECT e.id, e.kind, e.amount_minor, e.currency, \
     e.counter_amount_minor, e.counter_currency, e.category, e.note, e.occurred_at, \
     e.source, e.provenance, e.created_at, e.voided_at, e.void_reason, \
     c.scale, cc.scale \
     FROM fin_entries e \
     JOIN fin_currencies c ON c.code = e.currency \
     LEFT JOIN fin_currencies cc ON cc.code = e.counter_currency";

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    let kind_s: String = row.get(1)?;
    let source_s: String = row.get(9)?;
    let amount_minor: i64 = row.get(2)?;
    let currency: String = row.get(3)?;
    let counter_minor: Option<i64> = row.get(4)?;
    let counter_currency: Option<String> = row.get(5)?;
    let scale: u32 = row.get(14)?;
    let counter_scale: Option<u32> = row.get(15)?;
    let rate = match (counter_minor, counter_currency.as_deref(), counter_scale) {
        (Some(cm), Some(cc), Some(cs)) => {
            Some(effective_rate(amount_minor, &currency, scale, cm, cc, cs))
        }
        _ => None,
    };
    let mut entry = Entry {
        id: row.get(0)?,
        kind: Kind::parse(&kind_s).unwrap_or(Kind::Expense),
        amount_minor,
        amount: format_amount(amount_minor, scale),
        currency,
        counter_amount_minor: counter_minor,
        counter_amount: counter_minor
            .zip(counter_scale)
            .map(|(m, s)| format_amount(m, s)),
        counter_currency,
        category: row.get(6)?,
        note: row.get(7)?,
        occurred_at: from_unix(row.get(8)?),
        source: Source::parse(&source_s).unwrap_or(Source::Manual),
        provenance: row.get(10)?,
        created_at: from_unix(row.get(11)?),
        voided_at: row.get::<_, Option<i64>>(12)?.map(from_unix),
        void_reason: row.get(13)?,
        rate,
        usd: None,
        line: String::new(),
    };
    entry.line = entry_line(&entry);
    Ok(entry)
}

/// `#12 расход 25 BYN · еда · 15.10.2026` /
/// `#3 обмен 1500 RUB -> 11.6 TON · курс 1 TON = 129.3103 RUB · 27.08.2026`.
#[must_use]
pub fn entry_line(e: &Entry) -> String {
    let mut line = format!("#{} {} {} {}", e.id, e.kind.label(), e.amount, e.currency);
    if let (Some(ca), Some(cc)) = (&e.counter_amount, &e.counter_currency) {
        line.push_str(&format!(" -> {ca} {cc}"));
    }
    if let Some(r) = &e.rate {
        line.push_str(&format!(" · курс 1 {} = {} {}", r.base, r.value, r.quote));
    }
    if let Some(u) = &e.usd {
        line.push_str(&format!(" {}", u.text));
    }
    if let Some(c) = &e.category {
        line.push_str(&format!(" · {c}"));
    }
    line.push_str(&format!(
        " · {}",
        e.occurred_at.with_timezone(&Local).format("%d.%m.%Y")
    ));
    if let Some(n) = &e.note {
        line.push_str(&format!(" · {n}"));
    }
    if let Some(v) = &e.voided_at {
        line.push_str(&format!(
            " · ОТМЕНЕНА {} ({})",
            v.with_timezone(&Local).format("%d.%m.%Y"),
            e.void_reason.as_deref().unwrap_or("")
        ));
    }
    line
}

/// Запись по id, в том числе отменённая.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Entry>> {
    ensure_finance_schema(conn)?;
    get_entry(conn, id)
}

fn get_entry(conn: &Connection, id: i64) -> Result<Option<Entry>> {
    Ok(conn
        .query_row(
            &format!("{ENTRY_SELECT} WHERE e.id = ?1"),
            [id],
            row_to_entry,
        )
        .optional()?)
}

fn clean_text(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Записать пачку одной транзакцией: либо все записи, либо ни одной. Ошибка
/// называет номер записи в пачке (с нуля).
pub fn log(conn: &Connection, entries: &[NewEntry], now: DateTime<Utc>) -> Result<Vec<Entry>> {
    ensure_finance_schema(conn)?;
    if entries.is_empty() {
        bail!("пустая пачка: нечего записывать");
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let mut ids = Vec::with_capacity(entries.len());
    for (i, new) in entries.iter().enumerate() {
        let id = insert_one(&tx, new, now).with_context(|| format!("запись #{i} пачки"))?;
        ids.push(id);
    }
    tx.commit()?;
    ids.into_iter()
        .map(|id| get_entry(conn, id)?.ok_or_else(|| anyhow!("запись {id} пропала")))
        .collect()
}

fn insert_one(conn: &Connection, new: &NewEntry, now: DateTime<Utc>) -> Result<i64> {
    let cur = enabled_currency(conn, &new.currency)?;
    let amount_minor = parse_amount(&new.amount, cur.scale)?;
    let counter_amount = clean_text(new.counter_amount.as_deref());
    let counter_currency = clean_text(new.counter_currency.as_deref());
    let (counter_minor, counter_code) = match (new.kind, counter_amount, counter_currency) {
        (Kind::Exchange, Some(amount), Some(code)) => {
            let cc = enabled_currency(conn, &code)?;
            if cc.code == cur.code {
                bail!(
                    "обмен {} на {}: валюты сторон должны различаться",
                    cur.code,
                    cc.code
                );
            }
            (Some(parse_amount(&amount, cc.scale)?), Some(cc.code))
        }
        (Kind::Exchange, None, _) => bail!(
            "обмен без полученной суммы (counter_amount): не выводи её из официального курса — \
             спроси владельца"
        ),
        (Kind::Exchange, _, None) => {
            bail!("обмен без полученной валюты (counter_currency) — спроси владельца")
        }
        (_, None, None) => (None, None),
        (kind, _, _) => bail!(
            "counter_amount/counter_currency только у обмена, а это {}",
            kind.as_str()
        ),
    };
    let category = clean_text(new.category.as_deref()).map(|c| c.to_lowercase());
    if new.kind != Kind::Exchange && category.is_none() {
        bail!(
            "у {} нужна категория (сверься с fin_categories)",
            new.kind.as_str()
        );
    }
    conn.execute(
        "INSERT INTO fin_entries (kind, amount_minor, currency, counter_amount_minor,
             counter_currency, category, note, occurred_at, source, provenance, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            new.kind.as_str(),
            amount_minor,
            cur.code,
            counter_minor,
            counter_code,
            category,
            clean_text(new.note.as_deref()),
            new.occurred_at.unwrap_or(now).timestamp(),
            new.source.as_str(),
            clean_text(new.provenance.as_deref()),
            now.timestamp(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Отменить запись: строка остаётся, причина обязательна, повторная отмена —
/// ошибка, а не перезапись причины.
pub fn void(conn: &Connection, id: i64, reason: &str, now: DateTime<Utc>) -> Result<Entry> {
    ensure_finance_schema(conn)?;
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("отмена без причины не принимается");
    }
    let changed = conn.execute(
        "UPDATE fin_entries SET voided_at = ?2, void_reason = ?3
         WHERE id = ?1 AND voided_at IS NULL",
        params![id, now.timestamp(), reason],
    )?;
    if changed == 0 {
        if get_entry(conn, id)?.is_some() {
            bail!("запись #{id} уже отменена");
        }
        bail!("записи #{id} нет");
    }
    get_entry(conn, id)?.ok_or_else(|| anyhow!("запись {id} пропала"))
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub kind: Option<Kind>,
    pub category: Option<String>,
    pub currency: Option<String>,
    pub include_voided: bool,
    pub limit: usize,
}

/// Свежие сверху.
pub fn list(conn: &Connection, f: &ListFilter) -> Result<Vec<Entry>> {
    ensure_finance_schema(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{ENTRY_SELECT}
         WHERE (?1 IS NULL OR e.occurred_at >= ?1)
           AND (?2 IS NULL OR e.occurred_at < ?2)
           AND (?3 IS NULL OR e.kind = ?3)
           AND (?4 IS NULL OR e.category = ?4)
           AND (?5 IS NULL OR e.currency = ?5 OR e.counter_currency = ?5)
           AND (?6 OR e.voided_at IS NULL)
         ORDER BY e.occurred_at DESC, e.id DESC
         LIMIT ?7"
    ))?;
    let category = f.category.as_deref().map(|c| c.trim().to_lowercase());
    let currency = f.currency.as_deref().map(normalize_code).transpose()?;
    let limit = i64::try_from(f.limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(
        params![
            f.since.map(|d| d.timestamp()),
            f.until.map(|d| d.timestamp()),
            f.kind.map(Kind::as_str),
            category,
            currency,
            f.include_voided,
            limit,
        ],
        row_to_entry,
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Категория с числом живых (не отменённых) записей.
#[derive(Debug, Clone, Serialize)]
pub struct CategoryCount {
    pub category: String,
    pub count: i64,
    pub last_used: DateTime<Utc>,
}

pub fn categories(conn: &Connection) -> Result<Vec<CategoryCount>> {
    ensure_finance_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT category, COUNT(*), MAX(occurred_at) FROM fin_entries
         WHERE category IS NOT NULL AND voided_at IS NULL
         GROUP BY category ORDER BY COUNT(*) DESC, category",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CategoryCount {
            category: row.get(0)?,
            count: row.get(1)?,
            last_used: from_unix(row.get(2)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---------------------------------------------------------------------------
// Сводки
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Category,
    Month,
    Currency,
}

impl GroupBy {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "category" => Some(GroupBy::Category),
            "month" => Some(GroupBy::Month),
            "currency" => Some(GroupBy::Currency),
            _ => None,
        }
    }
}

/// Итог одной валюты. Обмены — отдельно: отдано (`exchanged_out`) и
/// получено (`exchanged_in`), ни в расход, ни в доход они не входят.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CurrencyTotal {
    pub currency: String,
    pub expense_minor: i64,
    pub expense: String,
    pub income_minor: i64,
    pub income: String,
    pub exchanged_out_minor: i64,
    pub exchanged_out: String,
    pub exchanged_in_minor: i64,
    pub exchanged_in: String,
    pub entries: i64,
}

/// Строка группировки: ключ (категория, месяц `ГГГГ-ММ` или код валюты) в
/// одной валюте. Только расходы и доходы.
#[derive(Debug, Clone, Serialize)]
pub struct GroupRow {
    pub key: String,
    pub currency: String,
    pub expense_minor: i64,
    pub expense: String,
    pub income_minor: i64,
    pub income: String,
    pub entries: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub period: Period,
    pub group_by: String,
    pub totals: Vec<CurrencyTotal>,
    pub groups: Vec<GroupRow>,
    pub voided_excluded: i64,
    /// Итог и разбивка в USD (`finance_rates::stats_usd`); `None` — сводка
    /// без пересчёта.
    pub usd: Option<crate::finance_rates::UsdSummary>,
}

fn total_slot<'a>(
    totals: &'a mut BTreeMap<String, CurrencyTotal>,
    code: &str,
) -> &'a mut CurrencyTotal {
    totals
        .entry(code.to_owned())
        .or_insert_with(|| CurrencyTotal {
            currency: code.to_owned(),
            ..CurrencyTotal::default()
        })
}

/// Ключ группы записи: категория, месяц `ГГГГ-ММ` или код валюты.
pub(crate) fn group_key(e: &Entry, group_by: GroupBy) -> String {
    match group_by {
        GroupBy::Category => e.category.clone().unwrap_or_default(),
        GroupBy::Month => e
            .occurred_at
            .with_timezone(&Local)
            .format("%Y-%m")
            .to_string(),
        GroupBy::Currency => e.currency.clone(),
    }
}

fn add_to(slot: &mut i64, v: i64) -> Result<()> {
    *slot = slot
        .checked_add(v)
        .ok_or_else(|| anyhow!("переполнение суммы в сводке"))?;
    Ok(())
}

/// Сводка за период: итоги по каждой валюте без пересчёта и группы по
/// категории, месяцу или валюте. Отменённые записи не входят никуда.
pub fn stats(
    conn: &Connection,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    period_label: &str,
    group_by: GroupBy,
) -> Result<Stats> {
    ensure_finance_schema(conn)?;
    let all = list(
        conn,
        &ListFilter {
            since,
            until,
            include_voided: true,
            limit: usize::MAX,
            ..ListFilter::default()
        },
    )?;
    let scales: HashMap<String, u32> = currencies(conn, true)?
        .into_iter()
        .map(|c| (c.code, c.scale))
        .collect();
    let scale_of = |code: &str| scales.get(code).copied().unwrap_or(0);

    let mut totals: BTreeMap<String, CurrencyTotal> = BTreeMap::new();
    let mut groups: BTreeMap<(String, String), GroupRow> = BTreeMap::new();
    let mut voided = 0;
    for e in &all {
        if e.voided_at.is_some() {
            voided += 1;
            continue;
        }
        if e.kind == Kind::Exchange {
            let t = total_slot(&mut totals, &e.currency);
            t.entries += 1;
            add_to(&mut t.exchanged_out_minor, e.amount_minor)?;
            if let (Some(cm), Some(cc)) = (e.counter_amount_minor, &e.counter_currency) {
                add_to(&mut total_slot(&mut totals, cc).exchanged_in_minor, cm)?;
            }
            continue;
        }
        let t = total_slot(&mut totals, &e.currency);
        t.entries += 1;
        if e.kind == Kind::Expense {
            add_to(&mut t.expense_minor, e.amount_minor)?;
        } else {
            add_to(&mut t.income_minor, e.amount_minor)?;
        }
        let key = group_key(e, group_by);
        let g = groups
            .entry((e.currency.clone(), key.clone()))
            .or_insert_with(|| GroupRow {
                key,
                currency: e.currency.clone(),
                expense_minor: 0,
                expense: String::new(),
                income_minor: 0,
                income: String::new(),
                entries: 0,
            });
        g.entries += 1;
        if e.kind == Kind::Expense {
            add_to(&mut g.expense_minor, e.amount_minor)?;
        } else {
            add_to(&mut g.income_minor, e.amount_minor)?;
        }
    }

    let totals = totals
        .into_values()
        .map(|mut t| {
            let s = scale_of(&t.currency);
            t.expense = format_amount(t.expense_minor, s);
            t.income = format_amount(t.income_minor, s);
            t.exchanged_out = format_amount(t.exchanged_out_minor, s);
            t.exchanged_in = format_amount(t.exchanged_in_minor, s);
            t
        })
        .collect();
    let mut groups: Vec<GroupRow> = groups
        .into_values()
        .map(|mut g| {
            let s = scale_of(&g.currency);
            g.expense = format_amount(g.expense_minor, s);
            g.income = format_amount(g.income_minor, s);
            g
        })
        .collect();
    if group_by == GroupBy::Month {
        groups.sort_by(|a, b| a.key.cmp(&b.key).then(a.currency.cmp(&b.currency)));
    } else {
        groups.sort_by(|a, b| {
            a.currency
                .cmp(&b.currency)
                .then(b.expense_minor.cmp(&a.expense_minor))
                .then(a.key.cmp(&b.key))
        });
    }
    Ok(Stats {
        period: Period {
            since,
            until,
            label: period_label.to_owned(),
        },
        group_by: match group_by {
            GroupBy::Category => "category",
            GroupBy::Month => "month",
            GroupBy::Currency => "currency",
        }
        .to_owned(),
        totals,
        groups,
        voided_excluded: voided,
        usd: None,
    })
}

/// Сводка человеку: итоги по валютам и группы.
#[must_use]
pub fn stats_text(s: &Stats) -> String {
    let mut out = vec![format!("Финансы — {}", s.period.label)];
    if s.totals.is_empty() {
        out.push("записей нет".to_owned());
        return out.join("\n");
    }
    if let Some(u) = &s.usd {
        out.push(format!(
            "Итого в USD: расход {}, доход {}",
            u.expense, u.income
        ));
        for g in &u.groups {
            let mut parts = Vec::new();
            if g.expense_minor > 0 {
                parts.push(format!("−{}", g.expense));
            }
            if g.income_minor > 0 {
                parts.push(format!("+{}", g.income));
            }
            if parts.is_empty() {
                parts.push("0".to_owned());
            }
            out.push(format!("  {}: {} USD", g.key, parts.join(" ")));
        }
        if u.unconverted.entries > 0 {
            let lost: Vec<String> = u
                .unconverted
                .totals
                .iter()
                .map(|t| {
                    let mut p = Vec::new();
                    if t.expense_minor > 0 {
                        p.push(format!("расход {} {}", t.expense, t.currency));
                    }
                    if t.income_minor > 0 {
                        p.push(format!("доход {} {}", t.income, t.currency));
                    }
                    p.join(", ")
                })
                .collect();
            out.push(format!(
                "не пересчитано (нет курса), записей {}: {}",
                u.unconverted.entries,
                lost.join("; ")
            ));
        }
        if !u.rate_errors.is_empty() {
            out.push(format!("курсы недоступны: {}", u.rate_errors.join("; ")));
        }
        out.push(String::new());
        out.push("В исходных валютах:".to_owned());
    }
    for t in &s.totals {
        let mut parts = Vec::new();
        if t.expense_minor > 0 {
            parts.push(format!("расход {}", t.expense));
        }
        if t.income_minor > 0 {
            parts.push(format!("доход {}", t.income));
        }
        if t.exchanged_out_minor > 0 {
            parts.push(format!("отдано в обмен {}", t.exchanged_out));
        }
        if t.exchanged_in_minor > 0 {
            parts.push(format!("получено обменом {}", t.exchanged_in));
        }
        out.push(format!("{}: {}", t.currency, parts.join(", ")));
    }
    if s.usd.is_none() && !s.groups.is_empty() {
        out.push(String::new());
        out.push(format!("По {}:", s.group_by));
        for g in &s.groups {
            let mut parts = Vec::new();
            if g.expense_minor > 0 {
                parts.push(format!("−{}", g.expense));
            }
            if g.income_minor > 0 {
                parts.push(format!("+{}", g.income));
            }
            out.push(format!("  {} {}: {}", g.key, g.currency, parts.join(" ")));
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-fin-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        crate::db::open(&dir.join("test.db")).expect("open test db")
    }

    fn new(kind: Kind, amount: &str, currency: &str, category: Option<&str>) -> NewEntry {
        NewEntry {
            kind,
            amount: amount.into(),
            currency: currency.into(),
            counter_amount: None,
            counter_currency: None,
            category: category.map(str::to_owned),
            note: None,
            occurred_at: None,
            source: Source::Text,
            provenance: None,
        }
    }

    fn exchange(amount: &str, cur: &str, counter: Option<&str>, counter_cur: &str) -> NewEntry {
        NewEntry {
            counter_amount: counter.map(str::to_owned),
            counter_currency: Some(counter_cur.into()),
            ..new(Kind::Exchange, amount, cur, None)
        }
    }

    fn all_time(conn: &Connection) -> Stats {
        stats(conn, None, None, "all", GroupBy::Category).expect("stats")
    }

    fn total<'a>(s: &'a Stats, cur: &str) -> &'a CurrencyTotal {
        s.totals
            .iter()
            .find(|t| t.currency == cur)
            .expect("currency total")
    }

    #[test]
    fn finance_expense_in_fiat_and_crypto_has_correct_minor_units() {
        let conn = test_conn();
        let out = log(
            &conn,
            &[
                new(Kind::Expense, "25", "byn", Some("Еда")),
                new(Kind::Expense, "30", "USDT", Some("подписки")),
                new(Kind::Expense, "0,00012345", "BTC", Some("комиссии")),
            ],
            Utc::now(),
        )
        .expect("log");
        assert_eq!(out[0].amount_minor, 2_500);
        assert_eq!(out[0].currency, "BYN");
        assert_eq!(out[0].category.as_deref(), Some("еда"));
        assert_eq!(out[1].amount_minor, 30_000_000);
        assert_eq!(out[1].amount, "30");
        assert_eq!(out[2].amount_minor, 12_345);
        assert_eq!(out[2].amount, "0.00012345");
        assert!(out[0].line.contains("расход 25 BYN"), "{}", out[0].line);
    }

    #[test]
    fn finance_exchange_requires_both_sides() {
        let conn = test_conn();
        let missing_amount = log(&conn, &[exchange("30", "USD", None, "BYN")], Utc::now());
        assert!(missing_amount.is_err());
        let mut missing_currency = exchange("30", "USD", Some("100"), "BYN");
        missing_currency.counter_currency = None;
        assert!(log(&conn, &[missing_currency], Utc::now()).is_err());
        let mut expense_with_counter = new(Kind::Expense, "5", "BYN", Some("еда"));
        expense_with_counter.counter_amount = Some("1".into());
        expense_with_counter.counter_currency = Some("USD".into());
        assert!(log(&conn, &[expense_with_counter], Utc::now()).is_err());
        // A failed batch writes nothing, not even its valid entries.
        let batch = [
            new(Kind::Expense, "5", "BYN", Some("еда")),
            exchange("30", "USD", None, "BYN"),
        ];
        assert!(log(&conn, &batch, Utc::now()).is_err());
        assert!(list(
            &conn,
            &ListFilter {
                limit: 10,
                ..ListFilter::default()
            }
        )
        .expect("list")
        .is_empty());

        let ok = log(
            &conn,
            &[exchange("75000", "RUB", Some("2500"), "BYN")],
            Utc::now(),
        )
        .expect("exchange");
        let rate = ok[0].rate.clone().expect("rate");
        assert_eq!(
            (rate.base.as_str(), rate.quote.as_str(), rate.value.as_str()),
            ("BYN", "RUB", "30")
        );
        assert!(
            ok[0].line.contains("75000 RUB -> 2500 BYN"),
            "{}",
            ok[0].line
        );
    }

    #[test]
    fn finance_more_decimals_than_scale_is_an_error_not_rounding() {
        let conn = test_conn();
        assert!(log(
            &conn,
            &[new(Kind::Expense, "25.123", "BYN", Some("еда"))],
            Utc::now()
        )
        .is_err());
        assert!(log(
            &conn,
            &[new(Kind::Expense, "1.0000001", "USDT", Some("x"))],
            Utc::now()
        )
        .is_err());
        assert!(parse_amount("0.123456789", 8).is_err());
        assert_eq!(parse_amount("0.12345678", 8).expect("8 digits"), 12_345_678);
        assert!(parse_amount("-5", 2).is_err());
        assert!(parse_amount("0", 2).is_err());
        assert!(parse_amount("1e3", 2).is_err());
        assert_eq!(parse_amount("75 000", 2).expect("spaces"), 7_500_000);
    }

    #[test]
    fn finance_void_keeps_row_and_excludes_it_from_stats() {
        let conn = test_conn();
        let out = log(
            &conn,
            &[
                new(Kind::Expense, "10", "BYN", Some("еда")),
                new(Kind::Expense, "4", "BYN", Some("еда")),
            ],
            Utc::now(),
        )
        .expect("log");
        let voided = void(&conn, out[0].id, "ошибка ввода", Utc::now()).expect("void");
        assert!(voided.voided_at.is_some());
        assert!(void(&conn, out[0].id, "ещё раз", Utc::now()).is_err());
        assert!(void(&conn, out[1].id, "  ", Utc::now()).is_err());

        let with_voided = list(
            &conn,
            &ListFilter {
                include_voided: true,
                limit: 10,
                ..ListFilter::default()
            },
        )
        .expect("list");
        assert_eq!(with_voided.len(), 2);
        let s = all_time(&conn);
        assert_eq!(total(&s, "BYN").expense_minor, 400);
        assert_eq!(s.voided_excluded, 1);
        assert_eq!(categories(&conn).expect("cats")[0].count, 1);
    }

    #[test]
    fn finance_stats_per_currency_without_conversion() {
        let conn = test_conn();
        log(
            &conn,
            &[
                new(Kind::Expense, "25", "BYN", Some("еда")),
                new(Kind::Expense, "5.5", "BYN", Some("транспорт")),
                new(Kind::Expense, "30", "USDT", Some("еда")),
                new(Kind::Income, "1000", "USD", Some("зарплата")),
            ],
            Utc::now(),
        )
        .expect("log");
        let s = all_time(&conn);
        assert_eq!(s.totals.len(), 3);
        assert_eq!(total(&s, "BYN").expense, "30.5");
        assert_eq!(total(&s, "USDT").expense_minor, 30_000_000);
        assert_eq!(total(&s, "USD").income, "1000");
        assert_eq!(total(&s, "USD").expense_minor, 0);
        let food: Vec<_> = s.groups.iter().filter(|g| g.key == "еда").collect();
        assert_eq!(
            food.len(),
            2,
            "same category in two currencies stays two rows"
        );
    }

    #[test]
    fn finance_exchange_excluded_from_expense_totals() {
        let conn = test_conn();
        log(
            &conn,
            &[
                exchange("1500", "RUB", Some("11.6"), "TON"),
                new(Kind::Expense, "100", "RUB", Some("еда")),
            ],
            Utc::now(),
        )
        .expect("log");
        let s = all_time(&conn);
        let rub = total(&s, "RUB");
        assert_eq!(rub.expense_minor, 10_000);
        assert_eq!(rub.exchanged_out_minor, 150_000);
        let ton = total(&s, "TON");
        assert_eq!(ton.expense_minor, 0);
        assert_eq!(ton.income_minor, 0);
        assert_eq!(ton.exchanged_in, "11.6");
        assert!(s
            .groups
            .iter()
            .all(|g| g.currency == "RUB" && g.key == "еда"));
    }

    #[test]
    fn finance_unknown_or_disabled_currency_is_refused() {
        let conn = test_conn();
        assert!(log(
            &conn,
            &[new(Kind::Expense, "1", "DOGE", Some("x"))],
            Utc::now()
        )
        .is_err());
        add_currency(&conn, "doge", "crypto", 8).expect("add");
        assert!(
            add_currency(&conn, "DOGE", "crypto", 2).is_err(),
            "scale is frozen"
        );
        log(
            &conn,
            &[new(Kind::Expense, "1", "DOGE", Some("x"))],
            Utc::now(),
        )
        .expect("enabled");
        disable_currency(&conn, "DOGE").expect("disable");
        assert!(log(
            &conn,
            &[new(Kind::Expense, "1", "DOGE", Some("x"))],
            Utc::now()
        )
        .is_err());
        assert!(currencies(&conn, false)
            .expect("list")
            .iter()
            .all(|c| c.code != "DOGE"));
    }

    #[test]
    fn finance_schema_rejects_real_amounts() {
        let conn = test_conn();
        ensure_finance_schema(&conn).expect("schema");
        let r = conn.execute(
            "INSERT INTO fin_entries (kind, amount_minor, currency, category, occurred_at, source, created_at)
             VALUES ('expense', 2.5, 'BYN', 'x', 0, 'text', 0)",
            [],
        );
        assert!(r.is_err());
    }

    #[test]
    fn finance_period_yesterday_is_a_closed_day() {
        let now = Utc::now();
        let p = resolve_period(Some("yesterday"), None, None, now).expect("period");
        let (since, until) = (p.since.expect("since"), p.until.expect("until"));
        assert!(until <= now && since < until);
        assert!(resolve_period(Some("fortnight"), None, None, now).is_err());
        assert!(resolve_period(Some("14d"), None, None, now).is_ok());
    }
}
