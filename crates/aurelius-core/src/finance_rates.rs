//! Курсы к USD для финансов и сводки в USD (задача 85f5fa07, 15.10.2026).
//!
//! Решения владельца, зашитые здесь:
//!
//! 1. Базовая валюта — USD (не USDT). Фиат -> USD напрямую по нацбанку
//!    (BYN — НБРБ, RUB — ЦБ РФ, KZT — НБ РК, EUR и прочий фиат — ЕЦБ через
//!    EUR), USDT и крипта -> USD по бирже (Kraken, дневная свеча): USDT не
//!    считается равным 1.
//! 2. Пересчёт — при чтении, по курсу на дату операции (ближайший не позже
//!    даты), а не снимком при записи: исправленный курс пересчитывает историю.
//! 3. Приоритет курса записи: названный владельцем для этой операции >
//!    фактический из его обмена > нацбанк на дату > биржа.
//! 4. Названный курс лежит в `fin_rates` с `entry_id`; `fin_entries` не
//!    меняется. Таблица создаётся лениво, вне `SCHEMA_VERSION` (как и `fin_*`,
//!    см. `db::ensure_finance_schema`).
//! 5. Запись без курса не выпадает: сводка кладёт её в явную корзину
//!    «не пересчитано».
//!
//! Курс хранится точной дробью `rate_num / rate_den` = сколько USD стоит одна
//! единица валюты. Никакого f64 в денежной арифметике: десятичные строки
//! источников разбираются в целые числитель и знаменатель.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration as StdDuration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::db::ensure_finance_schema;
use crate::finance::{self, CurrencyTotal, Entry, GroupBy, Kind, Stats};

/// Сколько дней назад искать курс, если на саму дату его нет (выходные,
/// праздники ЦБ РФ в январе — до 10 дней подряд).
const LOOKBACK_DAYS: i64 = 14;
/// Общий бюджет сети на одну сводку: дальше — только кэш.
const NETWORK_BUDGET: StdDuration = StdDuration::from_secs(20);

// ---------------------------------------------------------------------------
// Схема
// ---------------------------------------------------------------------------

/// `fin_rates` рядом с `fin_*`. Отдельно от `ensure_finance_schema`: та
/// выходит сразу, если `fin_entries` уже есть, а на живой базе она есть.
pub fn ensure_rates_schema(conn: &Connection) -> Result<()> {
    ensure_finance_schema(conn)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS fin_rates (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            date       TEXT NOT NULL
                       CHECK (date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
            currency   TEXT NOT NULL REFERENCES fin_currencies(code),
            rate_num   INTEGER NOT NULL CHECK (typeof(rate_num) = 'integer' AND rate_num > 0),
            rate_den   INTEGER NOT NULL CHECK (typeof(rate_den) = 'integer' AND rate_den > 0),
            source     TEXT NOT NULL,
            fetched_at INTEGER NOT NULL,
            entry_id   INTEGER REFERENCES fin_entries(id),
            CHECK ((source = 'dictated') = (entry_id IS NOT NULL)),
            CHECK (currency <> 'USD')
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_fin_rates_day
            ON fin_rates(currency, source, date) WHERE entry_id IS NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS idx_fin_rates_entry
            ON fin_rates(entry_id, currency) WHERE entry_id IS NOT NULL;
        ",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Точная дробь
// ---------------------------------------------------------------------------

/// Сколько USD стоит одна единица валюты: `num / den`, оба > 0, несократимо.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    pub num: i128,
    pub den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.abs()
}

impl Ratio {
    #[must_use]
    pub fn new(num: i128, den: i128) -> Option<Self> {
        if num <= 0 || den <= 0 {
            return None;
        }
        let g = gcd(num, den);
        Some(Self {
            num: num / g,
            den: den / g,
        })
    }

    #[must_use]
    pub const fn one() -> Self {
        Self { num: 1, den: 1 }
    }

    #[must_use]
    pub fn inv(self) -> Self {
        Self {
            num: self.den,
            den: self.num,
        }
    }

    #[must_use]
    pub fn checked_mul(self, o: Self) -> Option<Self> {
        let g1 = gcd(self.num, o.den).max(1);
        let g2 = gcd(o.num, self.den).max(1);
        Self::new(
            (self.num / g1).checked_mul(o.num / g2)?,
            (self.den / g2).checked_mul(o.den / g1)?,
        )
    }

    #[must_use]
    pub fn checked_div(self, o: Self) -> Option<Self> {
        self.checked_mul(o.inv())
    }

    /// Десятичная строка с `digits` знаками, половина вверх, без хвостовых
    /// нулей. Только для показа.
    #[must_use]
    pub fn show(self, digits: u32) -> String {
        let unit = 10_i128.pow(digits);
        let scaled = self
            .num
            .checked_mul(unit)
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(self.den))
            .and_then(|v| v.checked_div(self.den.checked_mul(2)?));
        let Some(scaled) = scaled else {
            return format!("{}/{}", self.num, self.den);
        };
        let (int_part, frac) = (scaled / unit, scaled % unit);
        if frac == 0 {
            return int_part.to_string();
        }
        let frac = format!("{frac:0width$}", width = digits as usize);
        format!("{int_part}.{}", frac.trim_end_matches('0'))
    }
}

/// «Единиц за 1 USD» для показа: 4 знака, а у дорогих валют (BTC) — 10,
/// чтобы не вышел «0».
fn per_usd_text(r: Ratio) -> String {
    let inv = r.inv();
    inv.show(if inv.num.saturating_mul(100) < inv.den {
        10
    } else {
        4
    })
}

/// Положительная десятичная строка («3.0046», «80,1234», «1») -> точная дробь.
pub fn parse_decimal(raw: &str) -> Result<Ratio> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '\u{a0}' | '\u{202f}'))
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    let (int_part, frac_part) = cleaned.split_once('.').unwrap_or((cleaned.as_str(), ""));
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if (int_part.is_empty() && frac_part.is_empty())
        || !digits(int_part)
        || !digits(frac_part)
        || int_part.len() > 18
        || frac_part.len() > 18
    {
        bail!("курс '{raw}' не число");
    }
    let both = format!("{int_part}{frac_part}");
    let num: i128 = both.parse().map_err(|_| anyhow!("курс '{raw}' не число"))?;
    let exp = u32::try_from(frac_part.len()).unwrap_or(18);
    Ratio::new(num, 10_i128.pow(exp)).ok_or_else(|| anyhow!("курс '{raw}' должен быть больше нуля"))
}

/// `amount_minor` (в единицах с точностью `scale`) по курсу -> центы USD,
/// половина вверх. Суммы в журнале всегда положительны.
pub fn to_usd_minor(amount_minor: i64, scale: u32, rate: Ratio) -> Result<i64> {
    let overflow = || anyhow!("переполнение при пересчёте в USD");
    let num = i128::from(amount_minor)
        .checked_mul(rate.num)
        .and_then(|v| v.checked_mul(100))
        .ok_or_else(overflow)?;
    let den = 10_i128
        .checked_pow(scale.min(finance::MAX_SCALE))
        .and_then(|v| v.checked_mul(rate.den))
        .ok_or_else(overflow)?;
    let cents = num
        .checked_mul(2)
        .and_then(|v| v.checked_add(den))
        .and_then(|v| v.checked_div(den.checked_mul(2)?))
        .ok_or_else(overflow)?;
    i64::try_from(cents).map_err(|_| overflow())
}

/// Сумма в единицах валюты точной дробью: `minor / 10^scale`.
fn units(minor: i64, scale: u32) -> Option<Ratio> {
    Ratio::new(
        i128::from(minor),
        10_i128.pow(scale.min(finance::MAX_SCALE)),
    )
}

// ---------------------------------------------------------------------------
// Источники
// ---------------------------------------------------------------------------

/// Сеть за трейтом: в тестах — подставной ответ, живого HTTP в `cargo test`
/// нет.
pub trait Http {
    fn get(&self, url: &str) -> Result<String>;
}

/// Живая сеть с короткими таймаутами: недоступный источник не держит сводку.
pub struct LiveHttp(reqwest::blocking::Client);

impl LiveHttp {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::blocking::Client::builder()
                .connect_timeout(StdDuration::from_secs(4))
                .timeout(StdDuration::from_secs(8))
                .user_agent(concat!("aurelius-finance/", env!("CARGO_PKG_VERSION")))
                .build()?,
        ))
    }
}

impl Http for LiveHttp {
    fn get(&self, url: &str) -> Result<String> {
        let resp = self.0.get(url).send()?;
        let status = resp.status();
        if !status.is_success() {
            bail!("HTTP {status}");
        }
        // CBR отдаёт windows-1251; нужные теги и числа — ASCII, остальное
        // можно испортить без вреда.
        Ok(String::from_utf8_lossy(&resp.bytes()?).into_owned())
    }
}

/// Сеть выключена: только кэш. Для тестов и `--offline`.
pub struct NoNetwork;

impl Http for NoNetwork {
    fn get(&self, _url: &str) -> Result<String> {
        bail!("сеть выключена")
    }
}

/// Источник курса валюты к USD, если для неё не названо иного.
#[must_use]
pub fn official_source(code: &str, kind: &str) -> &'static str {
    match code {
        "USD" => "usd",
        "BYN" => "nbrb",
        "RUB" => "cbr",
        "KZT" => "nbk",
        _ if kind == "crypto" => "kraken",
        _ => "ecb",
    }
}

#[must_use]
pub fn source_label(source: &str) -> &str {
    match source {
        "dictated" => "названный",
        "exchange" => "фактический обмен",
        "nbrb" => "НБРБ",
        "cbr" => "ЦБ РФ",
        "nbk" => "НБ РК",
        "ecb" => "ЕЦБ",
        "kraken" => "биржа Kraken",
        "usd" => "USD",
        other => other,
    }
}

/// Что вернул источник: курс, на какую дату он установлен (может быть
/// раньше запрошенной — выходные) и окончательный ли он (дневная свеча
/// текущего дня ещё меняется).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fetched {
    pub date: NaiveDate,
    pub rate: Ratio,
    pub settled: bool,
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let i = s.find(open)? + open.len();
    let j = s[i..].find(close)? + i;
    Some(&s[i..j])
}

fn day_start_unix(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .map_or(0, |n| n.and_utc().timestamp())
}

pub fn parse_nbrb(body: &str) -> Result<Fetched> {
    let date = between(body, "\"Date\":\"", "T")
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .ok_or_else(|| anyhow!("НБРБ: нет Date"))?;
    let scale = between(body, "\"Cur_Scale\":", ",")
        .and_then(|s| s.trim().parse::<i128>().ok())
        .ok_or_else(|| anyhow!("НБРБ: нет Cur_Scale"))?;
    let raw = body
        .split_once("\"Cur_OfficialRate\":")
        .map(|(_, rest)| {
            rest.trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>()
        })
        .ok_or_else(|| anyhow!("НБРБ: нет Cur_OfficialRate"))?;
    // Cur_OfficialRate BYN за Cur_Scale USD -> 1 BYN = scale / rate USD.
    let byn_per_scale = parse_decimal(&raw)?;
    let rate = Ratio::new(scale, 1)
        .and_then(|s| s.checked_div(byn_per_scale))
        .ok_or_else(|| anyhow!("НБРБ: курс вне диапазона"))?;
    Ok(Fetched {
        date,
        rate,
        settled: true,
    })
}

pub fn parse_cbr(body: &str) -> Result<Fetched> {
    let date = between(body, "ValCurs Date=\"", "\"")
        .and_then(|d| NaiveDate::parse_from_str(d, "%d.%m.%Y").ok())
        .ok_or_else(|| anyhow!("ЦБ РФ: нет ValCurs Date"))?;
    let usd = body
        .split_once("<CharCode>USD</CharCode>")
        .and_then(|(_, rest)| rest.split("</Valute>").next())
        .ok_or_else(|| anyhow!("ЦБ РФ: нет USD"))?;
    let nominal =
        between(usd, "<Nominal>", "</Nominal>").ok_or_else(|| anyhow!("ЦБ РФ: нет Nominal"))?;
    let value = between(usd, "<Value>", "</Value>").ok_or_else(|| anyhow!("ЦБ РФ: нет Value"))?;
    let rate = parse_decimal(nominal)?
        .checked_div(parse_decimal(value)?)
        .ok_or_else(|| anyhow!("ЦБ РФ: курс вне диапазона"))?;
    Ok(Fetched {
        date,
        rate,
        settled: true,
    })
}

pub fn parse_nbk(body: &str) -> Result<Fetched> {
    let date = between(body, "<date>", "</date>")
        .and_then(|d| NaiveDate::parse_from_str(d.trim(), "%d.%m.%Y").ok())
        .ok_or_else(|| anyhow!("НБ РК: нет date"))?;
    let usd = body
        .split_once("<title>USD</title>")
        .and_then(|(_, rest)| rest.split("</item>").next())
        .ok_or_else(|| anyhow!("НБ РК: нет USD"))?;
    let value = between(usd, "<description>", "</description>")
        .ok_or_else(|| anyhow!("НБ РК: нет description"))?;
    let quant = between(usd, "<quant>", "</quant>").ok_or_else(|| anyhow!("НБ РК: нет quant"))?;
    let rate = parse_decimal(quant)?
        .checked_div(parse_decimal(value)?)
        .ok_or_else(|| anyhow!("НБ РК: курс вне диапазона"))?;
    Ok(Fetched {
        date,
        rate,
        settled: true,
    })
}

/// CSV ЕЦБ (`detail=dataonly`): курсы к EUR. Для EUR — USD за 1 EUR, для
/// прочего фиата — кросс через EUR, на последнюю дату не позже `date`, где
/// есть обе серии.
pub fn parse_ecb(body: &str, code: &str, date: NaiveDate) -> Result<Option<Fetched>> {
    let mut lines = body.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h.trim() == name)
            .ok_or_else(|| anyhow!("ЕЦБ: нет колонки {name}"))
    };
    let (c_cur, c_date, c_val) = (col("CURRENCY")?, col("TIME_PERIOD")?, col("OBS_VALUE")?);
    let mut by_date: BTreeMap<NaiveDate, HashMap<String, Ratio>> = BTreeMap::new();
    for line in lines {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(cur), Some(d), Some(v)) = (f.get(c_cur), f.get(c_date), f.get(c_val)) else {
            continue;
        };
        let (Ok(d), Ok(v)) = (NaiveDate::parse_from_str(d, "%Y-%m-%d"), parse_decimal(v)) else {
            continue;
        };
        by_date.entry(d).or_default().insert((*cur).to_owned(), v);
    }
    for (d, row) in by_date.range(..=date).rev() {
        let Some(usd_per_eur) = row.get("USD").copied() else {
            continue;
        };
        let rate = if code == "EUR" {
            Some(usd_per_eur)
        } else {
            row.get(code)
                .and_then(|c_per_eur| usd_per_eur.checked_div(*c_per_eur))
        };
        if let Some(rate) = rate {
            return Ok(Some(Fetched {
                date: *d,
                rate,
                settled: true,
            }));
        }
    }
    Ok(None)
}

/// OHLC Kraken: дневная свеча с началом не позже `date`, цена закрытия.
/// Свеча текущего дня UTC ещё идёт — она не окончательная.
pub fn parse_kraken(body: &str, date: NaiveDate, today_utc: NaiveDate) -> Result<Option<Fetched>> {
    let v: serde_json::Value = serde_json::from_str(body).context("Kraken: не JSON")?;
    if let Some(err) = v
        .get("error")
        .and_then(|e| e.as_array())
        .filter(|e| !e.is_empty())
    {
        bail!("Kraken: {}", serde_json::Value::Array(err.clone()));
    }
    let result = v
        .get("result")
        .and_then(|r| r.as_object())
        .ok_or_else(|| anyhow!("Kraken: нет result"))?;
    let candles = result
        .iter()
        .find(|(k, _)| k.as_str() != "last")
        .and_then(|(_, c)| c.as_array())
        .ok_or_else(|| anyhow!("Kraken: нет свечей"))?;
    let limit = day_start_unix(date);
    let best = candles
        .iter()
        .filter_map(|c| {
            let t = c.get(0)?.as_i64()?;
            let close = c.get(4)?.as_str()?;
            (t <= limit).then_some((t, close))
        })
        .max_by_key(|(t, _)| *t);
    let Some((t, close)) = best else {
        return Ok(None);
    };
    let day = DateTime::<Utc>::from_timestamp(t, 0)
        .map(|d| d.date_naive())
        .ok_or_else(|| anyhow!("Kraken: время свечи {t}"))?;
    Ok(Some(Fetched {
        date: day,
        rate: parse_decimal(close)?,
        settled: day < today_utc,
    }))
}

fn kraken_pair(code: &str) -> String {
    match code {
        "BTC" => "XBTUSD".to_owned(),
        other => format!("{other}USD"),
    }
}

/// Почему источник не дал курс. Сеть и HTTP-статус выводят из строя весь
/// источник до конца прохода; ошибка в теле ответа (Kraken на HTTP 200 с
/// `EQuery:Unknown asset pair`, нет валюты в ответе) — только эту валюту.
#[derive(Debug)]
pub enum FetchError {
    Network(anyhow::Error),
    Data(anyhow::Error),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) | Self::Data(e) => write!(f, "{e:#}"),
        }
    }
}

fn source_url(source: &str, code: &str, date: NaiveDate) -> Option<String> {
    Some(match source {
        "nbrb" => format!(
            "https://api.nbrb.by/exrates/rates/USD?parammode=2&ondate={}",
            date.format("%Y-%m-%d")
        ),
        "cbr" => format!(
            "https://www.cbr.ru/scripts/XML_daily.asp?date_req={}",
            date.format("%d/%m/%Y")
        ),
        "nbk" => format!(
            "https://nationalbank.kz/rss/get_rates.cfm?fdate={}",
            date.format("%d.%m.%Y")
        ),
        "ecb" => {
            let key = if code == "EUR" {
                "USD".to_owned()
            } else {
                format!("USD+{code}")
            };
            format!(
                "https://data-api.ecb.europa.eu/service/data/EXR/D.{key}.EUR.SP00.A\
                 ?startPeriod={}&endPeriod={}&format=csvdata&detail=dataonly",
                (date - Duration::days(LOOKBACK_DAYS)).format("%Y-%m-%d"),
                date.format("%Y-%m-%d")
            )
        }
        "kraken" => format!(
            "https://api.kraken.com/0/public/OHLC?pair={}&interval=1440&since={}",
            kraken_pair(code),
            day_start_unix(date - Duration::days(7)) - 1
        ),
        _ => return None,
    })
}

/// Один запрос к источнику за курсом `code` на `date`.
pub fn fetch(
    http: &dyn Http,
    source: &str,
    code: &str,
    date: NaiveDate,
    today_utc: NaiveDate,
) -> Result<Option<Fetched>, FetchError> {
    let url = source_url(source, code, date)
        .ok_or_else(|| FetchError::Data(anyhow!("неизвестный источник курса {source}")))?;
    let body = http.get(&url).map_err(FetchError::Network)?;
    match source {
        "nbrb" => parse_nbrb(&body).map(Some),
        "cbr" => parse_cbr(&body).map(Some),
        "nbk" => parse_nbk(&body).map(Some),
        "ecb" => parse_ecb(&body, code, date),
        _ => parse_kraken(&body, date, today_utc),
    }
    .map_err(FetchError::Data)
}

// ---------------------------------------------------------------------------
// Кэш
// ---------------------------------------------------------------------------

/// Строка `fin_rates` для показа.
#[derive(Debug, Clone, Serialize)]
pub struct RateRow {
    pub id: i64,
    pub date: String,
    pub currency: String,
    pub rate_num: i64,
    pub rate_den: i64,
    /// USD за 1 единицу валюты.
    pub usd_per_unit: String,
    /// Единиц валюты за 1 USD.
    pub units_per_usd: String,
    pub source: String,
    pub source_label: String,
    pub fetched_at: DateTime<Utc>,
    pub entry_id: Option<i64>,
}

fn row_to_rate(row: &rusqlite::Row) -> rusqlite::Result<RateRow> {
    let num: i64 = row.get(3)?;
    let den: i64 = row.get(4)?;
    let source: String = row.get(5)?;
    let r = Ratio::new(i128::from(num), i128::from(den)).unwrap_or(Ratio::one());
    Ok(RateRow {
        id: row.get(0)?,
        date: row.get(1)?,
        currency: row.get(2)?,
        rate_num: num,
        rate_den: den,
        usd_per_unit: r.show(8),
        units_per_usd: per_usd_text(r),
        source_label: source_label(&source).to_owned(),
        source,
        fetched_at: DateTime::<Utc>::from_timestamp(row.get(6)?, 0).unwrap_or_default(),
        entry_id: row.get(7)?,
    })
}

const RATE_SELECT: &str =
    "SELECT id, date, currency, rate_num, rate_den, source, fetched_at, entry_id FROM fin_rates";

fn ratio_i64(r: Ratio) -> Result<(i64, i64)> {
    Ok((
        i64::try_from(r.num).map_err(|_| anyhow!("курс слишком точный для хранения"))?,
        i64::try_from(r.den).map_err(|_| anyhow!("курс слишком точный для хранения"))?,
    ))
}

/// Положить курс источника на дату (заменяя прежний на ту же дату).
pub fn store_rate(
    conn: &Connection,
    code: &str,
    source: &str,
    date: NaiveDate,
    rate: Ratio,
    now: DateTime<Utc>,
) -> Result<()> {
    ensure_rates_schema(conn)?;
    let (num, den) = ratio_i64(rate)?;
    let d = date.format("%Y-%m-%d").to_string();
    // Замена одной транзакцией: между DELETE и INSERT читатель не должен
    // увидеть дату без курса.
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM fin_rates WHERE currency = ?1 AND source = ?2 AND date = ?3 AND entry_id IS NULL",
        params![code, source, d],
    )?;
    tx.execute(
        "INSERT INTO fin_rates (date, currency, rate_num, rate_den, source, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![d, code, num, den, source, now.timestamp()],
    )?;
    tx.commit()?;
    Ok(())
}

fn cached(
    conn: &Connection,
    code: &str,
    source: &str,
    date: NaiveDate,
    lookback: i64,
) -> Result<Option<(NaiveDate, Ratio)>> {
    let row: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT date, rate_num, rate_den FROM fin_rates
             WHERE currency = ?1 AND source = ?2 AND entry_id IS NULL
               AND date <= ?3 AND date >= ?4
             ORDER BY date DESC LIMIT 1",
            params![
                code,
                source,
                date.format("%Y-%m-%d").to_string(),
                (date - Duration::days(lookback))
                    .format("%Y-%m-%d")
                    .to_string()
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(row.and_then(|(d, n, den)| {
        Some((
            NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()?,
            Ratio::new(i128::from(n), i128::from(den))?,
        ))
    }))
}

// ---------------------------------------------------------------------------
// Пересчёт
// ---------------------------------------------------------------------------

/// Как запись выражена в USD: сумма, курс одной из её валют и откуда он.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Conversion {
    pub usd_minor: i64,
    pub usd: String,
    /// Валюта, чей курс показан (у обмена — отданная, если она не USD).
    pub rate_currency: String,
    pub usd_per_unit: String,
    pub units_per_usd: String,
    /// Дата курса (у нацбанка может быть раньше даты записи — выходные).
    pub rate_date: String,
    pub source: String,
    pub source_label: String,
    /// Курс взят из кэша и старше даты операции больше чем на день (источник
    /// на эту дату не ответил).
    pub stale: bool,
    /// `≈ 8.32 USD (3.0046 BYN за 1 USD, НБРБ 27.08.2026)` — для строки записи.
    pub text: String,
}

#[derive(Debug, Clone, Copy)]
struct Found {
    rate: Ratio,
    date: NaiveDate,
    source: &'static str,
    /// Курс из кэша, старше даты операции больше чем на день.
    stale: bool,
}

/// Подбирает курсы для записей: кэш, ленивая подкачка с таймаутами, после
/// первой сетевой ошибки источник до конца прохода не дёргается.
pub struct Rater<'a> {
    conn: &'a Connection,
    http: &'a dyn Http,
    now: DateTime<Utc>,
    currencies: HashMap<String, (String, u32)>,
    deadline: Instant,
    broken: RefCell<HashSet<&'static str>>,
    memo: RefCell<HashMap<(String, NaiveDate), Option<Found>>>,
    errors: RefCell<Vec<String>>,
}

impl<'a> Rater<'a> {
    pub fn new(conn: &'a Connection, http: &'a dyn Http, now: DateTime<Utc>) -> Result<Self> {
        ensure_rates_schema(conn)?;
        let currencies = finance::currencies(conn, true)?
            .into_iter()
            .map(|c| (c.code, (c.kind, c.scale)))
            .collect();
        Ok(Self {
            conn,
            http,
            now,
            currencies,
            deadline: Instant::now() + NETWORK_BUDGET,
            broken: RefCell::new(HashSet::new()),
            memo: RefCell::new(HashMap::new()),
            errors: RefCell::new(Vec::new()),
        })
    }

    /// Сетевые ошибки прохода, по одной на источник.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        self.errors.borrow().clone()
    }

    fn scale(&self, code: &str) -> u32 {
        self.currencies.get(code).map_or(0, |c| c.1)
    }

    fn kind(&self, code: &str) -> &str {
        self.currencies.get(code).map_or("fiat", |c| c.0.as_str())
    }

    fn dictated(&self, entry_id: i64, code: &str) -> Result<Option<Found>> {
        let row: Option<(String, i64, i64)> = self
            .conn
            .query_row(
                "SELECT date, rate_num, rate_den FROM fin_rates WHERE entry_id = ?1 AND currency = ?2",
                params![entry_id, code],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(d, n, den)| {
            Some(Found {
                rate: Ratio::new(i128::from(n), i128::from(den))?,
                date: NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()?,
                source: "dictated",
                stale: false,
            })
        }))
    }

    /// Курс нацбанка или биржи на дату: точная дата в кэше, иначе запрос к
    /// источнику, иначе ближайший в кэше не позже даты (до 14 дней назад).
    fn market(&self, code: &str, date: NaiveDate) -> Result<Option<Found>> {
        if code == "USD" {
            return Ok(Some(Found {
                rate: Ratio::one(),
                date,
                source: "usd",
                stale: false,
            }));
        }
        let key = (code.to_owned(), date);
        if let Some(hit) = self.memo.borrow().get(&key) {
            return Ok(*hit);
        }
        let source = official_source(code, self.kind(code));
        let found = self.market_uncached(code, source, date)?;
        self.memo.borrow_mut().insert(key, found);
        Ok(found)
    }

    fn market_uncached(
        &self,
        code: &str,
        source: &'static str,
        date: NaiveDate,
    ) -> Result<Option<Found>> {
        // Строка на саму дату — либо курс этого дня, либо закреплённый за
        // выходным действовавший курс: свежий по определению.
        if let Some((d, rate)) = cached(self.conn, code, source, date, 0)? {
            return Ok(Some(Found {
                rate,
                date: d,
                source,
                stale: false,
            }));
        }
        let today_local = self.now.with_timezone(&Local).date_naive();
        let today_utc = self.now.date_naive();
        let mut fresh = None;
        if self.broken.borrow().contains(source) {
            // Ошибка источника уже записана один раз.
        } else if Instant::now() >= self.deadline {
            self.errors.borrow_mut().push(format!(
                "{}: бюджет сети {} с исчерпан, курс {code} на {date} не запрошен",
                source_label(source),
                NETWORK_BUDGET.as_secs()
            ));
        } else {
            match fetch(self.http, source, code, date.min(today_local), today_utc) {
                Ok(Some(f)) if f.date <= date => {
                    if f.settled {
                        store_rate(self.conn, code, source, f.date, f.rate, self.now)?;
                        // Прошлая дата без своего курса (выходной) получает
                        // действовавший курс, чтобы не спрашивать источник
                        // снова. Свежие даты не закрепляются: курс могут
                        // ещё опубликовать.
                        if f.date != date && date <= today_local - Duration::days(2) {
                            store_rate(self.conn, code, source, date, f.rate, self.now)?;
                        }
                    }
                    fresh = Some(f);
                }
                Ok(_) => {}
                Err(FetchError::Data(e)) => {
                    self.errors
                        .borrow_mut()
                        .push(format!("{} {code}: {e:#}", source_label(source)));
                }
                Err(e @ FetchError::Network(_)) => {
                    self.broken.borrow_mut().insert(source);
                    self.errors
                        .borrow_mut()
                        .push(format!("{}: {e}", source_label(source)));
                }
            }
        }
        // Ответ источника сам говорит, какой курс действовал на дату. Курс
        // из кэша без такого ответа старше даты операции больше чем на день
        // — устаревший: так его и помечаем.
        let old = cached(self.conn, code, source, date, LOOKBACK_DAYS)?;
        let best = match (fresh, old) {
            (Some(f), Some((d, _))) if f.date >= d => Some((f.date, f.rate, false)),
            (Some(f), None) => Some((f.date, f.rate, false)),
            (_, Some((d, rate))) => Some((d, rate, date - d > Duration::days(1))),
            (_, None) => None,
        };
        Ok(best.map(|(d, rate, stale)| Found {
            rate,
            date: d,
            source,
            stale,
        }))
    }

    fn conversion(
        &self,
        usd_minor: i64,
        currency: &str,
        f: Found,
        op_date: NaiveDate,
    ) -> Conversion {
        let usd_text = finance::format_amount(usd_minor, 2);
        let rate = f.rate;
        let shown = if currency == "USD" {
            String::new()
        } else if rate.num >= rate.den {
            format!("1 {currency} = {} USD, ", rate.show(4))
        } else {
            format!("{} {currency} за 1 USD, ", per_usd_text(rate))
        };
        // Дата курса — только если она не дата операции; устаревший курс
        // помечен явно.
        let when = if f.date == op_date {
            String::new()
        } else {
            format!(" на {}", f.date.format("%d.%m.%Y"))
        };
        let stale = if f.stale {
            ", курс устарел"
        } else {
            ""
        };
        let text = format!(
            "≈ {usd_text} USD ({shown}{}{when}{stale})",
            source_label(f.source)
        );
        Conversion {
            usd_minor,
            usd: usd_text,
            rate_currency: currency.to_owned(),
            usd_per_unit: rate.show(8),
            units_per_usd: per_usd_text(rate),
            rate_date: f.date.format("%Y-%m-%d").to_string(),
            source: f.source.to_owned(),
            source_label: source_label(f.source).to_owned(),
            stale: f.stale,
            text,
        }
    }

    /// USD-эквивалент записи или `None`, если курса нет ни из одного
    /// источника.
    pub fn convert(&self, e: &Entry) -> Result<Option<Conversion>> {
        let date = e.occurred_at.with_timezone(&Local).date_naive();
        let scale = self.scale(&e.currency);
        let (Some(cm), Some(cc)) = (e.counter_amount_minor, e.counter_currency.as_deref()) else {
            let found = match self.dictated(e.id, &e.currency)? {
                Some(f) => Some(f),
                None => self.market(&e.currency, date)?,
            };
            return found
                .map(|f| {
                    let usd = to_usd_minor(e.amount_minor, scale, f.rate)?;
                    Ok(self.conversion(usd, &e.currency, f, date))
                })
                .transpose();
        };
        // Обмен: стоимость операции в USD одна на обе стороны.
        let sides = [
            (e.currency.as_str(), e.amount_minor, scale),
            (cc, cm, self.scale(cc)),
        ];
        let mut chosen: Option<(usize, Found)> = None;
        for (i, side) in sides.iter().enumerate() {
            if let Some(f) = self.dictated(e.id, side.0)? {
                chosen = Some((i, f));
                break;
            }
        }
        if chosen.is_none() {
            // Одна из сторон — USD: фактический курс другой стороны точен.
            chosen = sides.iter().position(|s| s.0 == "USD").map(|i| {
                (
                    i,
                    Found {
                        rate: Ratio::one(),
                        date,
                        source: "usd",
                        stale: false,
                    },
                )
            });
        }
        if chosen.is_none() {
            for (i, side) in sides.iter().enumerate() {
                if let Some(f) = self.market(side.0, date)? {
                    chosen = Some((i, f));
                    break;
                }
            }
        }
        let Some((i, f)) = chosen else {
            return Ok(None);
        };
        let (code, minor, s) = sides[i];
        let usd = to_usd_minor(minor, s, f.rate)?;
        // Показываем курс отданной валюты, если она не USD; иначе — полученной.
        let shown = usize::from(sides[0].0 == "USD");
        if shown == i {
            return Ok(Some(self.conversion(usd, code, f, date)));
        }
        let (o_code, o_minor, o_scale) = sides[shown];
        let value = units(minor, s)
            .and_then(|u| u.checked_mul(f.rate))
            .ok_or_else(|| anyhow!("переполнение курса обмена"))?;
        let implied = units(o_minor, o_scale)
            .and_then(|u| value.checked_div(u))
            .ok_or_else(|| anyhow!("переполнение курса обмена"))?;
        // Курс другой стороны выведен из этого — устарел он, устарел и этот.
        let found = Found {
            rate: implied,
            date,
            source: "exchange",
            stale: f.stale,
        };
        Ok(Some(self.conversion(usd, o_code, found, date)))
    }
}

/// Проставить каждой записи USD-эквивалент и переписать её строку. Запись
/// без курса получает в строке явное «нет курса в USD». Возвращает сетевые
/// ошибки (не фатальные).
pub fn convert_entries(
    conn: &Connection,
    entries: &mut [Entry],
    http: &dyn Http,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let rater = Rater::new(conn, http, now)?;
    for e in entries.iter_mut() {
        e.usd = rater.convert(e)?;
        e.line = finance::entry_line(e);
        if e.usd.is_none() {
            e.line.push_str(" · нет курса в USD (не пересчитано)");
        }
    }
    Ok(rater.errors())
}

// ---------------------------------------------------------------------------
// Сводка в USD
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
pub struct UsdGroup {
    pub key: String,
    pub expense_minor: i64,
    pub expense: String,
    pub income_minor: i64,
    pub income: String,
    pub entries: i64,
}

/// Записи без курса: сколько, какие, и их суммы в исходных валютах.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Unconverted {
    pub entries: i64,
    pub ids: Vec<i64>,
    pub totals: Vec<CurrencyTotal>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UsdSummary {
    pub currency: String,
    pub expense_minor: i64,
    pub expense: String,
    pub income_minor: i64,
    pub income: String,
    /// Расходы и доходы в USD по `group_by` (по умолчанию — категория).
    pub groups: Vec<UsdGroup>,
    pub converted_entries: i64,
    /// Сколько записей пересчитано по каждому источнику курса.
    pub sources: BTreeMap<String, i64>,
    pub unconverted: Unconverted,
    /// Сколько записей пересчитано по устаревшему курсу (из кэша, старше
    /// даты операции больше чем на день).
    pub stale_entries: i64,
    /// Самая старая дата устаревшего курса; `None`, если таких нет.
    pub oldest_rate_date: Option<String>,
    /// Сетевые ошибки и исчерпанный бюджет сети — сводка всё равно построена.
    pub rate_errors: Vec<String>,
}

fn add(slot: &mut i64, v: i64) -> Result<()> {
    *slot = slot
        .checked_add(v)
        .ok_or_else(|| anyhow!("переполнение суммы в сводке"))?;
    Ok(())
}

/// Сводка за период с итогом и разбивкой в USD; итоги по исходным валютам
/// (`totals`) остаются как были. Обмены в USD-итог не входят — это не
/// расход и не доход.
pub fn stats_usd(
    conn: &Connection,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    period_label: &str,
    group_by: GroupBy,
    http: &dyn Http,
    now: DateTime<Utc>,
) -> Result<Stats> {
    let mut stats = finance::stats(conn, since, until, period_label, group_by)?;
    let entries = finance::list(
        conn,
        &finance::ListFilter {
            since,
            until,
            limit: usize::MAX,
            ..finance::ListFilter::default()
        },
    )?;
    let rater = Rater::new(conn, http, now)?;
    let mut sum = UsdSummary {
        currency: "USD".into(),
        ..UsdSummary::default()
    };
    let mut groups: BTreeMap<String, UsdGroup> = BTreeMap::new();
    let mut lost: BTreeMap<String, CurrencyTotal> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.kind != Kind::Exchange) {
        let expense = e.kind == Kind::Expense;
        let Some(c) = rater.convert(e)? else {
            sum.unconverted.entries += 1;
            sum.unconverted.ids.push(e.id);
            let t = lost
                .entry(e.currency.clone())
                .or_insert_with(|| CurrencyTotal {
                    currency: e.currency.clone(),
                    ..CurrencyTotal::default()
                });
            t.entries += 1;
            add(
                if expense {
                    &mut t.expense_minor
                } else {
                    &mut t.income_minor
                },
                e.amount_minor,
            )?;
            continue;
        };
        sum.converted_entries += 1;
        *sum.sources.entry(c.source.clone()).or_default() += 1;
        if c.stale {
            sum.stale_entries += 1;
            if sum
                .oldest_rate_date
                .as_ref()
                .is_none_or(|d| c.rate_date < *d)
            {
                sum.oldest_rate_date = Some(c.rate_date.clone());
            }
        }
        let key = finance::group_key(e, group_by);
        let g = groups.entry(key.clone()).or_insert_with(|| UsdGroup {
            key,
            ..UsdGroup::default()
        });
        g.entries += 1;
        if expense {
            add(&mut sum.expense_minor, c.usd_minor)?;
            add(&mut g.expense_minor, c.usd_minor)?;
        } else {
            add(&mut sum.income_minor, c.usd_minor)?;
            add(&mut g.income_minor, c.usd_minor)?;
        }
    }
    sum.expense = finance::format_amount(sum.expense_minor, 2);
    sum.income = finance::format_amount(sum.income_minor, 2);
    let mut groups: Vec<UsdGroup> = groups
        .into_values()
        .map(|mut g| {
            g.expense = finance::format_amount(g.expense_minor, 2);
            g.income = finance::format_amount(g.income_minor, 2);
            g
        })
        .collect();
    if group_by != GroupBy::Month {
        groups.sort_by(|a, b| {
            b.expense_minor
                .cmp(&a.expense_minor)
                .then(a.key.cmp(&b.key))
        });
    }
    sum.groups = groups;
    sum.unconverted.totals = lost
        .into_values()
        .map(|mut t| {
            let s = rater.scale(&t.currency);
            t.expense = finance::format_amount(t.expense_minor, s);
            t.income = finance::format_amount(t.income_minor, s);
            t.exchanged_out = "0".into();
            t.exchanged_in = "0".into();
            t
        })
        .collect();
    sum.rate_errors = rater.errors();
    stats.usd = Some(sum);
    Ok(stats)
}

// ---------------------------------------------------------------------------
// Названный курс и управление кэшем
// ---------------------------------------------------------------------------

/// Как владелец назвал курс: сколько единиц валюты за 1 USD («курс 3.25» для
/// BYN) или сколько USD за 1 единицу («TON по 1.4»).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateUnit {
    PerUsd,
    UsdPerUnit,
}

impl RateUnit {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "per_usd" => Some(Self::PerUsd),
            "usd_per_unit" => Some(Self::UsdPerUnit),
            _ => None,
        }
    }
}

/// Названный владельцем курс для записи. Валюта по умолчанию — валюта
/// записи (у обмена можно указать любую из сторон). Единица по умолчанию:
/// фиат — «за 1 USD», крипта — «USD за 1».
pub fn set_dictated(
    conn: &Connection,
    entry_id: i64,
    currency: Option<&str>,
    rate: &str,
    unit: Option<RateUnit>,
    now: DateTime<Utc>,
) -> Result<RateRow> {
    ensure_rates_schema(conn)?;
    let entry = finance::get(conn, entry_id)?.ok_or_else(|| anyhow!("записи #{entry_id} нет"))?;
    let code = match currency {
        Some(c) => finance::normalize_code(c)?,
        None => entry.currency.clone(),
    };
    if code != entry.currency && entry.counter_currency.as_deref() != Some(code.as_str()) {
        bail!("у записи #{entry_id} нет валюты {code}");
    }
    if code == "USD" {
        bail!("курс USD к USD всегда 1 — назови курс другой валюты записи");
    }
    let kind: String = conn.query_row(
        "SELECT kind FROM fin_currencies WHERE code = ?1",
        [&code],
        |r| r.get(0),
    )?;
    let unit = unit.unwrap_or(if kind == "crypto" {
        RateUnit::UsdPerUnit
    } else {
        RateUnit::PerUsd
    });
    let given = parse_decimal(rate)?;
    let usd_per_unit = match unit {
        RateUnit::UsdPerUnit => given,
        RateUnit::PerUsd => given.inv(),
    };
    let (num, den) = ratio_i64(usd_per_unit)?;
    let date = entry.occurred_at.with_timezone(&Local).date_naive();
    conn.execute(
        "DELETE FROM fin_rates WHERE entry_id = ?1 AND currency = ?2",
        params![entry_id, code],
    )?;
    conn.execute(
        "INSERT INTO fin_rates (date, currency, rate_num, rate_den, source, fetched_at, entry_id)
         VALUES (?1, ?2, ?3, ?4, 'dictated', ?5, ?6)",
        params![
            date.format("%Y-%m-%d").to_string(),
            code,
            num,
            den,
            now.timestamp(),
            entry_id
        ],
    )?;
    let id = conn.last_insert_rowid();
    conn.query_row(&format!("{RATE_SELECT} WHERE id = ?1"), [id], row_to_rate)
        .map_err(Into::into)
}

/// Снять названный курс записи (всех её валют или одной): дальше действует
/// обычный приоритет — обмен, нацбанк, биржа.
pub fn clear_dictated(conn: &Connection, entry_id: i64, currency: Option<&str>) -> Result<usize> {
    ensure_rates_schema(conn)?;
    let code = currency.map(finance::normalize_code).transpose()?;
    Ok(conn.execute(
        "DELETE FROM fin_rates WHERE entry_id = ?1 AND (?2 IS NULL OR currency = ?2)",
        params![entry_id, code],
    )?)
}

#[derive(Debug, Clone, Default)]
pub struct RateFilter {
    pub currency: Option<String>,
    pub since: Option<NaiveDate>,
    pub until: Option<NaiveDate>,
    pub entry_id: Option<i64>,
    pub limit: usize,
}

/// Курсы в кэше, свежие сверху. `until` включительно.
pub fn list_rates(conn: &Connection, f: &RateFilter) -> Result<Vec<RateRow>> {
    ensure_rates_schema(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{RATE_SELECT}
         WHERE (?1 IS NULL OR currency = ?1)
           AND (?2 IS NULL OR date >= ?2)
           AND (?3 IS NULL OR date <= ?3)
           AND (?4 IS NULL OR entry_id = ?4)
         ORDER BY date DESC, currency, source LIMIT ?5"
    ))?;
    let code = f
        .currency
        .as_deref()
        .map(finance::normalize_code)
        .transpose()?;
    let fmt = |d: Option<NaiveDate>| d.map(|d| d.format("%Y-%m-%d").to_string());
    let rows = stmt.query_map(
        params![
            code,
            fmt(f.since),
            fmt(f.until),
            f.entry_id,
            i64::try_from(f.limit).unwrap_or(i64::MAX)
        ],
        row_to_rate,
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Итог принудительного обновления по одной валюте.
#[derive(Debug, Clone, Serialize)]
pub struct Refreshed {
    pub currency: String,
    pub source: String,
    pub rate_date: Option<String>,
    pub usd_per_unit: Option<String>,
    pub units_per_usd: Option<String>,
    pub error: Option<String>,
}

/// Забыть кэш нацбанков и биржи на дату (названные курсы не трогаются) и
/// запросить заново: одну валюту или все включённые, кроме USD.
pub fn refresh(
    conn: &Connection,
    date: NaiveDate,
    currency: Option<&str>,
    http: &dyn Http,
    now: DateTime<Utc>,
) -> Result<Vec<Refreshed>> {
    ensure_rates_schema(conn)?;
    let codes: Vec<String> = match currency {
        Some(c) => vec![finance::normalize_code(c)?],
        None => finance::currencies(conn, false)?
            .into_iter()
            .map(|c| c.code)
            .filter(|c| c != "USD")
            .collect(),
    };
    let rater = Rater::new(conn, http, now)?;
    let mut out = Vec::new();
    for code in codes {
        if code == "USD" {
            continue;
        }
        conn.execute(
            "DELETE FROM fin_rates WHERE currency = ?1 AND date = ?2 AND entry_id IS NULL",
            params![code, date.format("%Y-%m-%d").to_string()],
        )?;
        let source = official_source(&code, rater.kind(&code));
        let before = rater.errors.borrow().len();
        let found = rater.market(&code, date)?;
        let error = rater.errors.borrow().get(before).cloned();
        out.push(Refreshed {
            currency: code,
            source: source.to_owned(),
            rate_date: found.map(|f| f.date.format("%Y-%m-%d").to_string()),
            usd_per_unit: found.map(|f| f.rate.show(8)),
            units_per_usd: found.map(|f| per_usd_text(f.rate)),
            error,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finance::{log, parse_occurred, NewEntry, Source};
    use std::cell::Cell;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-fin-rates-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        crate::db::open(&dir.join("test.db")).expect("open test db")
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("date")
    }

    /// «Сейчас» для тестов — середина октября, записи — в августе.
    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z")
            .expect("now")
            .with_timezone(&Utc)
    }

    fn entry(kind: Kind, amount: &str, cur: &str, day: &str) -> NewEntry {
        NewEntry {
            kind,
            amount: amount.into(),
            currency: cur.into(),
            counter_amount: None,
            counter_currency: None,
            category: (kind != Kind::Exchange).then(|| "еда".to_owned()),
            note: None,
            occurred_at: Some(parse_occurred(day).expect("day")),
            source: Source::Text,
            provenance: None,
        }
    }

    fn exchange(amount: &str, cur: &str, got: &str, got_cur: &str, day: &str) -> NewEntry {
        NewEntry {
            counter_amount: Some(got.into()),
            counter_currency: Some(got_cur.into()),
            ..entry(Kind::Exchange, amount, cur, day)
        }
    }

    fn one(conn: &Connection, e: NewEntry) -> Entry {
        log(conn, &[e], now()).expect("log").remove(0)
    }

    fn put(conn: &Connection, cur: &str, src: &str, day: &str, rate: &str) {
        store_rate(
            conn,
            cur,
            src,
            d(day),
            parse_decimal(rate).expect("rate"),
            now(),
        )
        .expect("store");
    }

    fn convert(conn: &Connection, e: &Entry) -> Option<Conversion> {
        Rater::new(conn, &NoNetwork, now())
            .expect("rater")
            .convert(e)
            .expect("convert")
    }

    /// Подставная сеть: ответ по подстроке URL, счётчик запросов.
    struct Mock {
        routes: Vec<(&'static str, String)>,
        calls: Cell<usize>,
    }

    impl Mock {
        fn new(routes: Vec<(&'static str, String)>) -> Self {
            Self {
                routes,
                calls: Cell::new(0),
            }
        }
    }

    impl Http for Mock {
        fn get(&self, url: &str) -> Result<String> {
            self.calls.set(self.calls.get() + 1);
            self.routes
                .iter()
                .find(|(k, _)| url.contains(k))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| anyhow!("connection refused: {url}"))
        }
    }

    const NBRB: &str = r#"{"Cur_ID":431,"Cur_Code":"840","Date":"2026-08-27T00:00:00","Cur_Abbreviation":"USD","Cur_Scale":1,"Cur_Name":"Доллар США","Cur_OfficialRate":3.0046}"#;
    const CBR: &str = r#"<?xml version="1.0" encoding="windows-1251"?><ValCurs Date="27.08.2026" name="Foreign Currency Market"><Valute ID="R01235"><NumCode>840</NumCode><CharCode>USD</CharCode><Nominal>1</Nominal><Name>x</Name><Value>80,1234</Value><VunitRate>80,1234</VunitRate></Valute><Valute ID="R01335"><NumCode>398</NumCode><CharCode>KZT</CharCode><Nominal>100</Nominal><Name>y</Name><Value>17,5</Value></Valute></ValCurs>"#;
    const NBK: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<rates>\n<description>Official</description>\n<date>27.08.2026</date>\n<item>\n<title>AUD</title>\n<description>300.1</description>\n<quant>1</quant>\n</item>\n<item>\n<fullname>ДОЛЛАР США</fullname>\n<title>USD</title>\n<description>458.87</description>\n<quant>1</quant>\n</item>\n</rates>";
    const ECB: &str =
        "KEY,FREQ,CURRENCY,CURRENCY_DENOM,EXR_TYPE,EXR_SUFFIX,TIME_PERIOD,OBS_VALUE\n\
        EXR.D.GBP.EUR.SP00.A,D,GBP,EUR,SP00,A,2026-08-27,0.8574\n\
        EXR.D.GBP.EUR.SP00.A,D,GBP,EUR,SP00,A,2026-08-28,0.8572\n\
        EXR.D.USD.EUR.SP00.A,D,USD,EUR,SP00,A,2026-08-27,1.1645\n\
        EXR.D.USD.EUR.SP00.A,D,USD,EUR,SP00,A,2026-08-28,1.1643\n";
    const KRAKEN: &str = r#"{"error":[],"result":{"TONUSD":[[1787702400,"1.441","1.453","1.366","1.408","1.405","290012.99636",1836],[1787788800,"1.408","1.452","1.372","1.416","1.416","258232.11478",1966],[1787875200,"1.417","1.427","1.348","1.370","1.39","1",1]],"last":1787875200}}"#;

    #[test]
    fn finance_rates_parsers_read_real_payloads_exactly() {
        let f = parse_nbrb(NBRB).expect("nbrb");
        assert_eq!(
            (f.date, f.rate),
            (d("2026-08-27"), Ratio::new(10_000, 30_046).expect("r"))
        );
        let f = parse_cbr(CBR).expect("cbr");
        assert_eq!(f.rate, Ratio::new(10_000, 801_234).expect("r"));
        let f = parse_nbk(NBK).expect("nbk");
        assert_eq!(f.rate, Ratio::new(100, 45_887).expect("r"));
        let eur = parse_ecb(ECB, "EUR", d("2026-08-30"))
            .expect("ecb")
            .expect("eur");
        assert_eq!(
            (eur.date, eur.rate),
            (d("2026-08-28"), Ratio::new(11_643, 10_000).expect("r"))
        );
        let gbp = parse_ecb(ECB, "GBP", d("2026-08-27"))
            .expect("ecb")
            .expect("gbp");
        assert_eq!(gbp.rate, Ratio::new(11_645, 8_574).expect("r"));
        let ton = parse_kraken(KRAKEN, d("2026-08-27"), d("2026-10-15"))
            .expect("kraken")
            .expect("candle");
        assert_eq!(
            (ton.date, ton.rate, ton.settled),
            (d("2026-08-27"), Ratio::new(1416, 1000).expect("r"), true)
        );
        let today = parse_kraken(KRAKEN, d("2026-08-28"), d("2026-08-28"))
            .expect("kraken")
            .expect("candle");
        assert!(!today.settled, "today's candle is still moving");
        assert!(parse_kraken(
            r#"{"error":["EQuery:Unknown asset pair"]}"#,
            d("2026-08-27"),
            d("2026-10-15")
        )
        .is_err());
    }

    #[test]
    fn finance_rates_priority_dictated_over_exchange_over_bank() {
        let conn = test_conn();
        put(&conn, "BYN", "nbrb", "2026-08-27", "0.33");
        let spent = one(&conn, entry(Kind::Expense, "30", "BYN", "2026-08-27"));
        let bank = convert(&conn, &spent).expect("bank rate");
        assert_eq!((bank.source.as_str(), bank.usd_minor), ("nbrb", 990));

        // Exchange 96 BYN -> 30 USD: the owner's actual rate beats the bank.
        let ex = one(&conn, exchange("96", "BYN", "30", "USD", "2026-08-27"));
        let actual = convert(&conn, &ex).expect("exchange rate");
        assert_eq!(actual.source, "exchange");
        assert_eq!(actual.rate_currency, "BYN");
        assert_eq!(actual.usd_per_unit, "0.3125");
        assert_eq!(actual.units_per_usd, "3.2");
        assert_eq!(actual.usd_minor, 3_000);

        // A dictated rate beats both, on the plain entry and on the exchange.
        set_dictated(&conn, spent.id, None, "3", None, now()).expect("dictate");
        let dictated = convert(&conn, &spent).expect("dictated");
        assert_eq!(
            (dictated.source.as_str(), dictated.usd_minor),
            ("dictated", 1_000)
        );
        set_dictated(
            &conn,
            ex.id,
            Some("byn"),
            "3",
            Some(RateUnit::PerUsd),
            now(),
        )
        .expect("dictate");
        assert_eq!(
            convert(&conn, &ex).expect("dictated exchange").usd_minor,
            3_200
        );

        // Clearing returns to the normal order.
        assert_eq!(clear_dictated(&conn, spent.id, None).expect("clear"), 1);
        assert_eq!(convert(&conn, &spent).expect("bank again").source, "nbrb");
        assert!(set_dictated(&conn, spent.id, Some("RUB"), "90", None, now()).is_err());
        assert!(set_dictated(&conn, 999, None, "3", None, now()).is_err());
    }

    #[test]
    fn finance_rates_exchange_beats_market_for_crypto() {
        let conn = test_conn();
        put(&conn, "USDT", "kraken", "2026-08-27", "0.9998");
        let spent = one(&conn, entry(Kind::Expense, "100", "USDT", "2026-08-27"));
        let market = convert(&conn, &spent).expect("market");
        assert_eq!(
            (market.source.as_str(), market.usd_minor),
            ("kraken", 9_998)
        );
        // USDT is not assumed 1.0 and an exchange of USDT to USD sets its own rate.
        let ex = one(&conn, exchange("100", "USDT", "99.5", "USD", "2026-08-27"));
        let c = convert(&conn, &ex).expect("exchange");
        assert_eq!(
            (c.source.as_str(), c.usd_minor, c.usd_per_unit.as_str()),
            ("exchange", 9_950, "0.995")
        );
        // Neither side USD: the given side converts by its rate, the other is implied.
        put(&conn, "RUB", "cbr", "2026-08-27", "0.0125");
        let rub_ton = one(&conn, exchange("1500", "RUB", "11.6", "TON", "2026-08-27"));
        let c = convert(&conn, &rub_ton).expect("rub side");
        assert_eq!(
            (c.source.as_str(), c.rate_currency.as_str(), c.usd_minor),
            ("cbr", "RUB", 1_875)
        );
    }

    #[test]
    fn finance_rates_nearest_not_later_date() {
        let conn = test_conn();
        put(&conn, "BYN", "nbrb", "2026-08-25", "0.3");
        put(&conn, "BYN", "nbrb", "2026-08-29", "0.4");
        let mid = one(&conn, entry(Kind::Expense, "10", "BYN", "2026-08-27"));
        let c = convert(&conn, &mid).expect("earlier rate");
        assert_eq!((c.rate_date.as_str(), c.usd_minor), ("2026-08-25", 300));
        assert!(
            c.stale,
            "two days older than the operation and no source answer"
        );
        assert!(c.text.contains("на 25.08.2026"), "{}", c.text);
        let exact = one(&conn, entry(Kind::Expense, "10", "BYN", "2026-08-29"));
        let c = convert(&conn, &exact).expect("exact");
        assert_eq!((c.usd_minor, c.stale), (400, false));
        assert!(
            !c.text.contains(" на "),
            "same date is not repeated: {}",
            c.text
        );
        let next_day = one(&conn, entry(Kind::Expense, "10", "BYN", "2026-08-30"));
        assert!(
            !convert(&conn, &next_day).expect("one day").stale,
            "one day back is allowed"
        );
        let before = one(&conn, entry(Kind::Expense, "10", "BYN", "2026-08-24"));
        assert!(
            convert(&conn, &before).is_none(),
            "a later rate is never used"
        );
        let stale = one(&conn, entry(Kind::Expense, "10", "BYN", "2026-09-20"));
        assert!(
            convert(&conn, &stale).is_none(),
            "beyond the lookback window"
        );
    }

    #[test]
    fn finance_rates_missing_rate_goes_to_unconverted_bucket() {
        let conn = test_conn();
        log(
            &conn,
            &[
                entry(Kind::Expense, "25", "BYN", "2026-08-27"),
                entry(Kind::Expense, "7.5", "USD", "2026-08-27"),
                entry(Kind::Income, "1000", "RUB", "2026-08-27"),
            ],
            now(),
        )
        .expect("log");
        let s = stats_usd(
            &conn,
            None,
            None,
            "all",
            GroupBy::Category,
            &NoNetwork,
            now(),
        )
        .expect("stats");
        let u = s.usd.as_ref().expect("usd");
        assert_eq!(
            (u.expense_minor, u.income_minor, u.converted_entries),
            (750, 0, 1)
        );
        assert_eq!(u.unconverted.entries, 2);
        assert_eq!(u.unconverted.ids.len(), 2);
        let byn = u
            .unconverted
            .totals
            .iter()
            .find(|t| t.currency == "BYN")
            .expect("byn");
        assert_eq!(byn.expense, "25");
        let rub = u
            .unconverted
            .totals
            .iter()
            .find(|t| t.currency == "RUB")
            .expect("rub");
        assert_eq!(rub.income, "1000");
        assert_eq!(s.totals.len(), 3, "per-currency totals stay");
        let text = finance::stats_text(&s);
        assert!(text.contains("не пересчитано"), "{text}");
    }

    #[test]
    fn finance_rates_usd_totals_match_independent_i128() {
        let conn = test_conn();
        put(&conn, "BYN", "nbrb", "2026-08-20", "0.3328");
        put(&conn, "RUB", "cbr", "2026-08-20", "0.01248");
        put(&conn, "BTC", "kraken", "2026-08-20", "80265.9");
        put(&conn, "USDT", "kraken", "2026-08-20", "0.99994");
        let rates: [(&str, u32, i128, i128); 4] = [
            ("BYN", 2, 3328, 10_000),
            ("RUB", 2, 1248, 100_000),
            ("BTC", 8, 802_659, 10),
            ("USDT", 6, 99_994, 100_000),
        ];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut batch = Vec::new();
        let mut expect_exp: i128 = 0;
        let mut expect_inc: i128 = 0;
        for i in 0..60 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let (code, scale, num, den) = rates[(seed >> 33) as usize % 4];
            let minor = (seed >> 20) % 5_000_000 + 1;
            let unit = 10_u64.pow(scale);
            let amount = format!("{}.{:0w$}", minor / unit, minor % unit, w = scale as usize);
            let kind = if i % 5 == 0 {
                Kind::Income
            } else {
                Kind::Expense
            };
            // Independent: cents = round_half_up(minor * num * 100 / (10^scale * den)).
            let n = i128::from(minor) * num * 100;
            let dd = i128::from(unit) * den;
            let cents = (2 * n + dd) / (2 * dd);
            if kind == Kind::Income {
                expect_inc += cents;
            } else {
                expect_exp += cents;
            }
            batch.push(entry(kind, &amount, code, "2026-08-22"));
        }
        log(&conn, &batch, now()).expect("log");
        let s = stats_usd(
            &conn,
            None,
            None,
            "all",
            GroupBy::Category,
            &NoNetwork,
            now(),
        )
        .expect("stats");
        let u = s.usd.expect("usd");
        assert_eq!(u.unconverted.entries, 0);
        assert_eq!(i128::from(u.expense_minor), expect_exp);
        assert_eq!(i128::from(u.income_minor), expect_inc);
        let groups: i64 = u.groups.iter().map(|g| g.expense_minor).sum();
        assert_eq!(groups, u.expense_minor);
    }

    #[test]
    fn finance_rates_fetch_caches_and_failure_degrades() {
        let conn = test_conn();
        let byn = one(&conn, entry(Kind::Expense, "30.05", "BYN", "2026-08-27"));
        let ton = one(&conn, entry(Kind::Expense, "10", "TON", "2026-08-27"));
        let net = Mock::new(vec![
            ("api.nbrb.by", NBRB.to_owned()),
            ("api.kraken.com", KRAKEN.to_owned()),
        ]);
        let mut entries = vec![byn.clone(), ton.clone()];
        let errors = convert_entries(&conn, &mut entries, &net, now()).expect("convert");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(net.calls.get(), 2);
        assert_eq!(entries[0].usd.as_ref().map(|c| c.usd_minor), Some(1_000));
        assert_eq!(entries[1].usd.as_ref().map(|c| c.usd_minor), Some(1_416));
        assert!(entries[0].line.contains("НБРБ"), "{}", entries[0].line);
        // Second pass: from the cache, no network.
        let mut again = vec![byn, ton];
        convert_entries(&conn, &mut again, &net, now()).expect("convert");
        assert_eq!(net.calls.get(), 2);

        // A dead source: one attempt per pass, then unconverted, never a panic.
        let rub = one(&conn, entry(Kind::Expense, "100", "RUB", "2026-08-26"));
        let rub2 = one(&conn, entry(Kind::Expense, "100", "RUB", "2026-08-25"));
        let dead = Mock::new(vec![]);
        let mut both = vec![rub, rub2];
        let errors = convert_entries(&conn, &mut both, &dead, now()).expect("degrades");
        assert_eq!(
            dead.calls.get(),
            1,
            "circuit breaker after the first failure"
        );
        assert_eq!(errors.len(), 1);
        assert!(both.iter().all(|e| e.usd.is_none()));
        assert!(both[0].line.contains("нет курса"), "{}", both[0].line);
    }

    #[test]
    fn finance_rates_refresh_replaces_cached_but_keeps_dictated() {
        let conn = test_conn();
        put(&conn, "BYN", "nbrb", "2026-08-27", "0.5");
        let e = one(&conn, entry(Kind::Expense, "1", "BYN", "2026-08-27"));
        set_dictated(&conn, e.id, None, "3.1", None, now()).expect("dictate");
        let net = Mock::new(vec![("api.nbrb.by", NBRB.to_owned())]);
        let out = refresh(&conn, d("2026-08-27"), Some("BYN"), &net, now()).expect("refresh");
        assert_eq!(out[0].units_per_usd.as_deref(), Some("3.0046"));
        let rows = list_rates(
            &conn,
            &RateFilter {
                currency: Some("BYN".into()),
                limit: 10,
                ..RateFilter::default()
            },
        )
        .expect("list");
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|r| r.source == "dictated" && r.units_per_usd == "3.1"));
        assert!(rows
            .iter()
            .any(|r| r.source == "nbrb" && r.units_per_usd == "3.0046"));
    }

    #[test]
    fn finance_rates_schema_rejects_real_numbers_and_orphan_dictated() {
        let conn = test_conn();
        ensure_rates_schema(&conn).expect("schema");
        let real = conn.execute(
            "INSERT INTO fin_rates (date, currency, rate_num, rate_den, source, fetched_at)
             VALUES ('2026-08-27', 'BYN', 0.33, 1, 'nbrb', 0)",
            [],
        );
        assert!(real.is_err());
        let dictated_without_entry = conn.execute(
            "INSERT INTO fin_rates (date, currency, rate_num, rate_den, source, fetched_at)
             VALUES ('2026-08-27', 'BYN', 1, 3, 'dictated', 0)",
            [],
        );
        assert!(dictated_without_entry.is_err());
    }
}
