//! Repro tests from the review of the finance branch (task 52a7521b, file
//! review_a7k3.rs), ported with the fixed contract asserted on top.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use anyhow::{anyhow, Result};
use aurelius_core::finance::{self, GroupBy, Kind, NewEntry, Source};
use aurelius_core::finance_rates::{self as rates, Http};
use chrono::{DateTime, Utc};
use rusqlite::Connection;

fn conn(tag: &str) -> Connection {
    let dir = std::env::temp_dir().join(format!("fin-review-a7k3-{tag}-{}", uuid::Uuid::new_v4()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    aurelius_core::db::open(&dir.join("t.db")).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn exp(amount: &str, cur: &str, day: &str) -> NewEntry {
    NewEntry {
        kind: Kind::Expense,
        amount: amount.into(),
        currency: cur.into(),
        counter_amount: None,
        counter_currency: None,
        category: Some("еда".into()),
        note: None,
        occurred_at: Some(finance::parse_occurred(day).unwrap()),
        source: Source::Text,
        provenance: None,
    }
}

struct Routes(Vec<(&'static str, Result<String, String>, u64)>);

impl Http for Routes {
    fn get(&self, url: &str) -> Result<String> {
        for (k, v, sleep) in &self.0 {
            if url.contains(k) {
                std::thread::sleep(StdDuration::from_millis(*sleep));
                return v.clone().map_err(|e| anyhow!(e));
            }
        }
        Err(anyhow!("no route {url}"))
    }
}

const TON: &str = r#"{"error":[],"result":{"TONUSD":[[1787788800,"1.408","1.452","1.372","1.416","1.416","1",1]],"last":1787788800}}"#;
const UNKNOWN: &str = r#"{"error":["EQuery:Unknown asset pair"]}"#;

/// A token Kraken does not list must not take every other crypto down with it.
#[test]
fn finance_review_kraken_unknown_pair_poisons_other_crypto() {
    let c = conn("kraken");
    finance::add_currency(&c, "FOO", "crypto", 8).unwrap();
    // TON first (lower id); FOO second -> list is newest/id DESC, FOO is processed first.
    finance::log(
        &c,
        &[
            exp("10", "TON", "2026-08-27"),
            exp("1", "FOO", "2026-08-27"),
        ],
        now(),
    )
    .unwrap();
    let http = Routes(vec![
        ("TONUSD", Ok(TON.into()), 0),
        ("FOOUSD", Ok(UNKNOWN.into()), 0),
    ]);
    let s = rates::stats_usd(&c, None, None, "all", GroupBy::Category, &http, now()).unwrap();
    let u = s.usd.clone().unwrap();
    eprintln!("usd = {}", serde_json::to_string(&u).unwrap());
    assert_eq!(
        u.unconverted.entries, 1,
        "only FOO should be unconverted; TON has a valid Kraken answer"
    );
    assert_eq!(u.expense_minor, 1416, "10 TON * 1.416 = 14.16 USD");
    // The unknown pair is reported for FOO alone, the source is not marked broken.
    assert_eq!(u.rate_errors.len(), 1, "{:?}", u.rate_errors);
    assert!(u.rate_errors[0].contains("FOO"), "{:?}", u.rate_errors);
}

/// Budget exhausted: later currencies are silently served from a stale cache,
/// and rate_errors says nothing.
#[test]
fn finance_review_budget_exhausted_stale_rate_silent() {
    let c = conn("budget");
    // Entries are processed newest id first: RUB is logged first so that the
    // slow BYN fetch spends the whole budget before RUB is reached. In the
    // original order RUB was fetched fresh and the scenario never happened.
    finance::log(
        &c,
        &[
            exp("8000", "RUB", "2026-08-27"),
            exp("100", "BYN", "2026-08-27"),
        ],
        now(),
    )
    .unwrap();
    // RUB cache from 12 days earlier (old rate 1/100 instead of ~1/80).
    rates::store_rate(
        &c,
        "RUB",
        "cbr",
        chrono::NaiveDate::from_ymd_opt(2026, 8, 15).unwrap(),
        rates::parse_decimal("0.01").unwrap(),
        now(),
    )
    .unwrap();
    const NBRB: &str =
        r#"{"Cur_ID":431,"Date":"2026-08-27T00:00:00","Cur_Scale":1,"Cur_OfficialRate":3.0046}"#;
    let http = Routes(vec![
        ("nbrb", Ok(NBRB.into()), 20_500),
        ("cbr", Ok("<ValCurs Date=\"27.08.2026\"><Valute><CharCode>USD</CharCode><Nominal>1</Nominal><Value>80,0000</Value></Valute></ValCurs>".into()), 0),
    ]);
    let s = rates::stats_usd(&c, None, None, "all", GroupBy::Category, &http, now()).unwrap();
    let u = s.usd.clone().unwrap();
    eprintln!("usd = {}", serde_json::to_string(&u).unwrap());
    eprintln!("text =\n{}", finance::stats_text(&s));
    assert!(
        !u.rate_errors.is_empty() || u.unconverted.entries > 0,
        "RUB converted at a 12-day-old cached rate, nothing tells the user"
    );
    // Fixed contract: the skipped currency is named, the stale rate is counted
    // with its date, and both the summary text and the entry line say so.
    assert!(
        u.rate_errors
            .iter()
            .any(|e| e.contains("бюджет") && e.contains("RUB")),
        "{:?}",
        u.rate_errors
    );
    assert_eq!(u.stale_entries, 1);
    assert_eq!(u.oldest_rate_date.as_deref(), Some("2026-08-15"));
    let text = finance::stats_text(&s);
    assert!(text.contains("устаревшему курсу"), "{text}");
    let mut list = finance::list(
        &c,
        &finance::ListFilter {
            limit: 10,
            ..Default::default()
        },
    )
    .unwrap();
    rates::convert_entries(&c, &mut list, &rates::NoNetwork, now()).unwrap();
    let rub = list.iter().find(|e| e.currency == "RUB").unwrap();
    assert!(
        rub.line.contains("на 15.08.2026") && rub.line.contains("курс устарел"),
        "{}",
        rub.line
    );
    assert_eq!(rub.usd.as_ref().map(|u| u.stale), Some(true));
}
