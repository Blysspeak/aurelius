//! Property tests for `aurelius_core::finance` (task c843b315).
//!
//! Every test name starts with `finance_` so one filter runs the lot:
//! `cargo test -p aurelius-core --release -- --include-ignored finance`.
//! Tests named `finance_bug_*` reproduce a confirmed defect; they are
//! `#[ignore]`d so a plain `cargo test` stays green, and fail when included.
//!
//! Each database case runs on its own scratch file under the system temp
//! dir; nothing here resolves `AURELIUS_HOME` or the live database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use aurelius_core::finance::{self, GroupBy, Kind, NewEntry, Source, Stats};
use chrono::{DateTime, Local, Utc};
use proptest::prelude::*;
use proptest::test_runner::Config;
use rusqlite::Connection;

// ---------------------------------------------------------------------------
// Scratch database
// ---------------------------------------------------------------------------

struct Scratch {
    dir: PathBuf,
    conn: Option<Connection>,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("aurelius-fin-prop-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let conn = aurelius_core::db::open(&dir.join("t.db")).expect("open scratch db");
        // A scale-0 currency is not seeded; add one so all scales are covered.
        finance::add_currency(&conn, "JPY", "fiat", 0).expect("add JPY");
        Self {
            dir,
            conn: Some(conn),
        }
    }

    fn conn(&self) -> &Connection {
        self.conn.as_ref().expect("conn")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(self.conn.take());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn row_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM fin_entries", [], |r| r.get(0))
        .expect("count")
}

fn cfg(cases: u32) -> Config {
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

/// (code, scale) — every scale the owner uses: 0, 2, 6, 8.
const CURRENCIES: &[(&str, u32)] = &[("JPY", 0), ("BYN", 2), ("USDT", 6), ("BTC", 8)];
const SCALES: &[u32] = &[0, 2, 6, 8];

fn pow10(s: u32) -> i128 {
    10_i128.pow(s)
}

/// Independent minor -> decimal formatter (i128, no shared code with the module).
fn decimal_of(v: i128, scale: u32) -> String {
    let unit = pow10(scale);
    let (int, frac) = (v / unit, v % unit);
    if scale == 0 {
        return int.to_string();
    }
    format!("{int}.{frac:0w$}", w = scale as usize)
}

// ---------------------------------------------------------------------------
// 1. Amount parsing
// ---------------------------------------------------------------------------

/// Canonical decimal string for `scale`: no leading zeros, no trailing zeros
/// in the fraction, value > 0. Returned with its exact minor value.
fn canonical_amount() -> impl Strategy<Value = (u32, String, i128)> {
    (
        prop::sample::select(SCALES),
        0_u64..=99_999_999_999_u64,
        0_u64..=99_999_999,
    )
        .prop_filter_map("non-zero", |(scale, int, frac_raw)| {
            let unit = 10_u64.pow(scale);
            let frac = if scale == 0 { 0 } else { frac_raw % unit };
            let minor = i128::from(int) * i128::from(unit) + i128::from(frac);
            // Above i64::MAX is covered by `finance_prop_near_i64_max`.
            if minor == 0 || minor > i128::from(i64::MAX) {
                return None;
            }
            let s = if frac == 0 {
                int.to_string()
            } else {
                let f = format!("{frac:0w$}", w = scale as usize);
                format!("{int}.{}", f.trim_end_matches('0'))
            };
            Some((scale, s, minor))
        })
}

proptest! {
    #![proptest_config(cfg(40_000))]

    /// minor -> display -> parse is the identity for every scale and every
    /// positive i64, including i64::MAX itself.
    #[test]
    fn finance_prop_minor_format_parse_identity(
        scale in prop::sample::select(SCALES),
        minor in prop_oneof![1_i64..=1_000_000, 1_i64..=i64::MAX, (i64::MAX - 1_000_000)..=i64::MAX],
    ) {
        let shown = finance::format_amount(minor, scale);
        let back = finance::parse_amount(&shown, scale);
        prop_assert!(back.is_ok(), "parse('{}', {}) failed: {:?}", shown, scale, back.err());
        prop_assert_eq!(back.ok(), Some(minor));
    }
}

proptest! {
    #![proptest_config(cfg(30_000))]

    /// Canonical string -> minor -> display equals the input; comma decimal
    /// separator and surrounding spaces give the same minor value.
    #[test]
    fn finance_prop_decimal_string_round_trip(
        (scale, s, minor) in canonical_amount(),
        lead in "[ \t]{0,3}",
        trail in "[ \t]{0,3}",
        comma in any::<bool>(),
    ) {
        let parsed = finance::parse_amount(&s, scale);
        prop_assert_eq!(parsed.as_ref().ok().map(|m| i128::from(*m)), Some(minor), "'{}' scale {}", s, scale);
        let m = parsed.expect("checked");
        prop_assert_eq!(finance::format_amount(m, scale), s.clone());

        let variant = if comma { s.replace('.', ",") } else { s.clone() };
        let padded = format!("{lead}{variant}{trail}");
        prop_assert_eq!(finance::parse_amount(&padded, scale).ok(), Some(m), "'{}'", padded);
    }
}

proptest! {
    #![proptest_config(cfg(15_000))]

    /// More fraction digits than the scale is an error, never rounding —
    /// even when the extra digits are zeros.
    #[test]
    fn finance_prop_excess_decimals_rejected(
        scale in prop::sample::select(SCALES),
        int in 0_u64..1_000_000_000,
        extra in 1_usize..=6,
        digits in "[0-9]{14}",
        comma in any::<bool>(),
    ) {
        let frac = &digits[..scale as usize + extra];
        let sep = if comma { "," } else { "." };
        let s = format!("{int}{sep}{frac}");
        prop_assert!(finance::parse_amount(&s, scale).is_err(), "'{}' accepted at scale {}", s, scale);
    }
}

/// Inputs that must never be accepted: exponent notation, signs, zero.
fn garbage_amount() -> impl Strategy<Value = String> {
    let mant = (1_u64..100_000, 0_u32..4).prop_map(|(m, f)| {
        if f == 0 {
            m.to_string()
        } else {
            format!("{}.{}", m / 10_u64.pow(f.min(4)), m % 10_u64.pow(f.min(4)))
        }
    });
    prop_oneof![
        (mant.clone(), prop::sample::select(&["e", "E", "e+", "e-", "E-"][..]), 0_u32..30)
            .prop_map(|(m, e, x)| format!("{m}{e}{x}")),
        (prop::sample::select(&["-", "+", "−"][..]), mant.clone()).prop_map(|(s, m)| format!("{s}{m}")),
        (0_usize..5, 0_usize..9, any::<bool>()).prop_map(|(i, f, comma)| {
            let int = "0".repeat(i);
            let frac = "0".repeat(f);
            let sep = if comma { "," } else { "." };
            if f == 0 { if int.is_empty() { String::from("0") } else { int } } else { format!("{int}{sep}{frac}") }
        }),
        prop::sample::select(&["", " ", ".", ",", "..", "1.2.3", "1,2,3", "nan", "inf", "0x10", "1/2", "١٢", "12abc"][..])
            .prop_map(str::to_owned),
    ]
}

proptest! {
    #![proptest_config(cfg(15_000))]

    #[test]
    fn finance_prop_garbage_rejected(scale in prop::sample::select(SCALES), s in garbage_amount()) {
        prop_assert!(finance::parse_amount(&s, scale).is_err(), "'{}' accepted at scale {}", s, scale);
    }
}

proptest! {
    #![proptest_config(cfg(10_000))]

    /// Around i64::MAX in minor units: at or below parses exactly, above is
    /// an error (no wrap, no panic). Huge integer parts are errors too.
    #[test]
    fn finance_prop_near_i64_max(
        scale in prop::sample::select(SCALES),
        k in 0_i128..1_000_000_000_000,
        huge in "[1-9][0-9]{19,40}",
    ) {
        let max = i128::from(i64::MAX);
        let below = decimal_of(max - k, scale);
        prop_assert_eq!(
            finance::parse_amount(&below, scale).ok().map(i128::from),
            Some(max - k),
            "'{}' scale {}", below, scale
        );
        let above = decimal_of(max + 1 + k, scale);
        prop_assert!(finance::parse_amount(&above, scale).is_err(), "'{}' accepted", above);
        prop_assert!(finance::parse_amount(&huge, scale).is_err(), "'{}' accepted", huge);
    }
}

// ---------------------------------------------------------------------------
// 2. Stats invariant against an independent i128 sum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Gen {
    kind: Kind,
    cur: usize,
    minor: i64,
    counter_cur: usize,
    counter_minor: i64,
    category: &'static str,
    with_category: bool,
    ts: i64,
    voided: bool,
    comma: bool,
}

/// Raw category spellings; stats must see them trimmed and lowercased.
const CATEGORIES: &[&str] = &["еда", "Еда", " транспорт ", "ПОДПИСКИ", "rent", "rent "];
/// 2025-01-01T00:00:00Z .. 2027-01-01T00:00:00Z.
const TS_LO: i64 = 1_735_689_600;
const TS_HI: i64 = 1_798_761_600;

fn gen_entry() -> impl Strategy<Value = Gen> {
    (
        (0_u8..3, 0_usize..CURRENCIES.len(), 1_i64..=1_000_000_000_000),
        (1_usize..CURRENCIES.len(), 1_i64..=1_000_000_000_000),
        (prop::sample::select(CATEGORIES), any::<bool>()),
        (TS_LO..TS_HI, prop::bool::weighted(0.2), any::<bool>()),
    )
        .prop_map(|((k, cur, minor), (off, cminor), (category, wc), (ts, voided, comma))| Gen {
            kind: [Kind::Expense, Kind::Income, Kind::Exchange][usize::from(k)],
            cur,
            minor,
            counter_cur: (cur + off) % CURRENCIES.len(),
            counter_minor: cminor,
            category,
            with_category: wc,
            ts,
            voided,
            comma,
        })
}

fn to_new(g: &Gen) -> NewEntry {
    let (code, scale) = CURRENCIES[g.cur];
    let mut amount = finance::format_amount(g.minor, scale);
    if g.comma {
        amount = amount.replace('.', ",");
    }
    let exchange = g.kind == Kind::Exchange;
    let (ccode, cscale) = CURRENCIES[g.counter_cur];
    NewEntry {
        kind: g.kind,
        amount,
        currency: code.to_lowercase(),
        counter_amount: exchange.then(|| finance::format_amount(g.counter_minor, cscale)),
        counter_currency: exchange.then(|| ccode.to_owned()),
        category: (!exchange || g.with_category).then(|| g.category.to_owned()),
        note: None,
        occurred_at: DateTime::<Utc>::from_timestamp(g.ts, 0),
        source: Source::Import,
        provenance: None,
    }
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
struct Tot {
    expense: i128,
    income: i128,
    out: i128,
    inn: i128,
    entries: i64,
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
struct Grp {
    expense: i128,
    income: i128,
    entries: i64,
}

type Expected = (BTreeMap<String, Tot>, BTreeMap<(String, String), Grp>, i64);

fn expected(gens: &[Gen], since: Option<i64>, until: Option<i64>, by: GroupBy) -> Expected {
    let mut totals: BTreeMap<String, Tot> = BTreeMap::new();
    let mut groups: BTreeMap<(String, String), Grp> = BTreeMap::new();
    let mut voided = 0;
    for g in gens {
        if since.is_some_and(|s| g.ts < s) || until.is_some_and(|u| g.ts >= u) {
            continue;
        }
        if g.voided {
            voided += 1;
            continue;
        }
        let code = CURRENCIES[g.cur].0.to_owned();
        let minor = i128::from(g.minor);
        if g.kind == Kind::Exchange {
            let t = totals.entry(code).or_default();
            t.entries += 1;
            t.out += minor;
            let ccode = CURRENCIES[g.counter_cur].0.to_owned();
            totals.entry(ccode).or_default().inn += i128::from(g.counter_minor);
            continue;
        }
        let t = totals.entry(code.clone()).or_default();
        t.entries += 1;
        let key = match by {
            GroupBy::Category => g.category.trim().to_lowercase(),
            GroupBy::Month => DateTime::<Utc>::from_timestamp(g.ts, 0)
                .expect("ts")
                .with_timezone(&Local)
                .format("%Y-%m")
                .to_string(),
            GroupBy::Currency => code.clone(),
        };
        let gr = groups.entry((code, key)).or_default();
        gr.entries += 1;
        if g.kind == Kind::Expense {
            t.expense += minor;
            gr.expense += minor;
        } else {
            t.income += minor;
            gr.income += minor;
        }
    }
    (totals, groups, voided)
}

fn actual(s: &Stats) -> Expected {
    let totals = s
        .totals
        .iter()
        .map(|t| {
            (
                t.currency.clone(),
                Tot {
                    expense: i128::from(t.expense_minor),
                    income: i128::from(t.income_minor),
                    out: i128::from(t.exchanged_out_minor),
                    inn: i128::from(t.exchanged_in_minor),
                    entries: t.entries,
                },
            )
        })
        .collect();
    let groups = s
        .groups
        .iter()
        .map(|g| {
            (
                (g.currency.clone(), g.key.clone()),
                Grp {
                    expense: i128::from(g.expense_minor),
                    income: i128::from(g.income_minor),
                    entries: g.entries,
                },
            )
        })
        .collect();
    (totals, groups, s.voided_excluded)
}

/// Writes `gens` in batches of `chunk`, voids the flagged ones.
fn load(conn: &Connection, gens: &[Gen], chunk: usize) {
    let now = Utc::now();
    let mut ids = Vec::with_capacity(gens.len());
    for part in gens.chunks(chunk.max(1)) {
        let batch: Vec<NewEntry> = part.iter().map(to_new).collect();
        let written = finance::log(conn, &batch, now).expect("valid batch must log");
        ids.extend(written.iter().map(|e| e.id));
    }
    for (g, id) in gens.iter().zip(ids) {
        if g.voided {
            finance::void(conn, id, "prop", now).expect("void");
        }
    }
}

proptest! {
    #![proptest_config(cfg(400))]

    /// fin_stats totals per currency / category / month / currency-group
    /// equal an independent i128 sum; exchanges never in expense/income;
    /// voided rows excluded; a random [since, until) window is honoured.
    #[test]
    fn finance_prop_stats_equal_independent_sum(
        gens in prop::collection::vec(gen_entry(), 1..160),
        chunk in 1_usize..64,
        a in TS_LO..TS_HI,
        b in TS_LO..TS_HI,
    ) {
        let db = Scratch::new();
        let conn = db.conn();
        load(conn, &gens, chunk);
        let (lo, hi) = (a.min(b), a.max(b));
        let windows = [
            (None, None),
            (Some(lo), Some(hi)),
        ];
        for (since, until) in windows {
            for by in [GroupBy::Category, GroupBy::Month, GroupBy::Currency] {
                let s = finance::stats(
                    conn,
                    since.and_then(|t| DateTime::<Utc>::from_timestamp(t, 0)),
                    until.and_then(|t| DateTime::<Utc>::from_timestamp(t, 0)),
                    "prop",
                    by,
                ).expect("stats");
                prop_assert_eq!(actual(&s), expected(&gens, since, until, by), "by {:?} window {:?}..{:?}", by, since, until);
                for t in &s.totals {
                    let scale = CURRENCIES.iter().find(|c| c.0 == t.currency).map(|c| c.1).expect("scale");
                    prop_assert_eq!(finance::parse_amount(&t.expense, scale).unwrap_or(0), t.expense_minor);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Batch atomicity
// ---------------------------------------------------------------------------

/// One way to make an entry invalid.
fn poison(mut e: NewEntry, how: u8) -> NewEntry {
    match how {
        0 => e.amount = format!("{}.123456789", e.amount.split(['.', ',']).next().unwrap_or("1")),
        1 => e.currency = "DOGE".into(),
        2 => {
            e.kind = Kind::Exchange;
            e.counter_amount = None;
            e.counter_currency = Some("BYN".into());
        }
        3 => {
            e.kind = Kind::Expense;
            e.counter_amount = None;
            e.counter_currency = None;
            e.category = Some("   ".into());
        }
        4 => {
            e.kind = Kind::Exchange;
            e.counter_amount = Some("1".into());
            e.counter_currency = Some(e.currency.clone());
        }
        5 => e.amount = "0".into(),
        6 => {
            e.kind = Kind::Income;
            e.category = Some("x".into());
            e.counter_amount = Some("1".into());
            e.counter_currency = Some("BYN".into());
        }
        _ => e.amount = "1e3".into(),
    }
    e
}

proptest! {
    #![proptest_config(cfg(500))]

    #[test]
    fn finance_prop_batch_with_one_invalid_writes_nothing(
        before in prop::collection::vec(gen_entry(), 0..20),
        batch in prop::collection::vec(gen_entry(), 1..60),
        pos in any::<prop::sample::Index>(),
        how in 0_u8..8,
    ) {
        let db = Scratch::new();
        let conn = db.conn();
        let before: Vec<Gen> = before.into_iter().map(|mut g| { g.voided = false; g }).collect();
        load(conn, &before, 7);
        let n0 = row_count(conn);
        let mut entries: Vec<NewEntry> = batch.iter().map(to_new).collect();
        let i = pos.index(entries.len());
        entries[i] = poison(entries[i].clone(), how);
        let r = finance::log(conn, &entries, Utc::now());
        prop_assert!(r.is_err(), "poison {} at {} accepted", how, i);
        let msg = format!("{:#}", r.expect_err("err"));
        prop_assert!(msg.contains(&format!("#{i} ")), "error does not name entry {}: {}", i, msg);
        prop_assert_eq!(row_count(conn), n0);
        // The connection is still usable: the clean part of the batch logs.
        entries.remove(i);
        if !entries.is_empty() {
            finance::log(conn, &entries, Utc::now()).expect("clean rest logs");
            prop_assert_eq!(row_count(conn), n0 + i64::try_from(entries.len()).expect("len"));
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Sum overflow
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(cfg(200))]

    /// Many large entries in one currency: stats is exact while the true sum
    /// fits in i64 and an explicit error once it does not — never a wrap.
    #[test]
    fn finance_prop_sum_overflow_is_explicit(
        minors in prop::collection::vec((i64::MAX / 16)..=(i64::MAX / 2), 1..12),
        kind in prop::sample::select(&[Kind::Expense, Kind::Income, Kind::Exchange][..]),
    ) {
        let db = Scratch::new();
        let conn = db.conn();
        let gens: Vec<Gen> = minors.iter().map(|&m| Gen {
            kind,
            cur: 3,
            minor: m,
            counter_cur: 1,
            counter_minor: m,
            category: "x",
            with_category: true,
            ts: TS_LO + 1,
            voided: false,
            comma: false,
        }).collect();
        load(conn, &gens, 5);
        let true_sum: i128 = minors.iter().map(|&m| i128::from(m)).sum();
        let r = finance::stats(conn, None, None, "all", GroupBy::Category);
        if true_sum > i128::from(i64::MAX) {
            prop_assert!(r.is_err(), "sum {} > i64::MAX but stats returned Ok", true_sum);
        } else {
            let s = r.expect("fits");
            let btc = s.totals.iter().find(|t| t.currency == "BTC").expect("BTC");
            let got = match kind {
                Kind::Expense => btc.expense_minor,
                Kind::Income => btc.income_minor,
                Kind::Exchange => btc.exchanged_out_minor,
            };
            prop_assert_eq!(i128::from(got), true_sum);
            if kind == Kind::Exchange {
                let byn = s.totals.iter().find(|t| t.currency == "BYN").expect("BYN");
                prop_assert_eq!(i128::from(byn.exchanged_in_minor), true_sum);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Owner's phrasing through the MCP parse path
// ---------------------------------------------------------------------------

/// What `handlers::finance::amount_param` does with a JSON value before
/// `parse_amount`: strings verbatim, numbers through `Number::to_string`.
fn mcp_amount(json_literal: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json_literal).expect("json") {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        other => panic!("unexpected {other}"),
    }
}

#[test]
fn finance_owner_phrasing_as_strings() {
    let cases: &[(&str, u32, i64)] = &[
        ("25", 2, 2_500),
        ("25.5", 2, 2_550),
        ("25,50", 2, 2_550),
        ("75000", 2, 7_500_000),
        ("75 000", 2, 7_500_000),
        ("0.00000001", 8, 1),
        ("11.6", 8, 1_160_000_000),
        ("30", 6, 30_000_000),
    ];
    for &(raw, scale, minor) in cases {
        assert_eq!(finance::parse_amount(raw, scale).ok(), Some(minor), "'{raw}'");
        let via_mcp = mcp_amount(&serde_json::to_string(raw).expect("json"));
        assert_eq!(finance::parse_amount(&via_mcp, scale).ok(), Some(minor), "mcp '{raw}'");
    }
}

#[test]
fn finance_owner_phrasing_as_json_numbers_ok() {
    // JSON numbers the shortest-float form keeps exact.
    let cases: &[(&str, u32, i64)] = &[
        ("25", 2, 2_500),
        ("25.5", 2, 2_550),
        ("25.50", 2, 2_550),
        ("75000", 2, 7_500_000),
        ("11.6", 8, 1_160_000_000),
    ];
    for &(lit, scale, minor) in cases {
        let s = mcp_amount(lit);
        assert_eq!(finance::parse_amount(&s, scale).ok(), Some(minor), "{lit} -> '{s}'");
    }
}

/// BUG: a JSON number `0.00000001` (1 satoshi) reaches `parse_amount` as
/// `"1e-8"` and is refused as "not a number". The tool schema says string,
/// but the handler explicitly accepts numbers (`amount_param`).
#[test]
#[ignore = "BUG finance-json-number-exponent: JSON number 0.00000001 -> '1e-8' rejected"]
fn finance_bug_json_number_small_amount_rejected() {
    let s = mcp_amount("0.00000001");
    assert_eq!(finance::parse_amount(&s, 8).ok(), Some(1), "0.00000001 -> '{s}'");
}

proptest! {
    #![proptest_config(cfg(5_000))]

    /// BUG: any JSON number with more than ~15-17 significant digits goes
    /// through f64 and is silently rounded before `parse_amount` sees it.
    /// The module promises "never rounding".
    #[test]
    #[ignore = "BUG finance-json-number-float: JSON numbers pass through f64 and are rounded or turned into exponent form"]
    fn finance_bug_json_number_equals_string((scale, s, minor) in canonical_amount()) {
        let via_number = mcp_amount(&s);
        prop_assert_eq!(
            finance::parse_amount(&via_number, scale).ok().map(i128::from),
            Some(minor),
            "JSON number {} became '{}' at scale {}", s, via_number, scale
        );
    }
}

/// Not a defect of the parser as specified, recorded so the behaviour is
/// visible: with scale >= 3 a thousands comma reads as a decimal comma.
#[test]
fn finance_note_thousands_comma_reads_as_decimal_at_scale_6() {
    assert_eq!(finance::parse_amount("1,000", 6).ok(), Some(1_000_000)); // 1 USDT, not 1000
    assert!(finance::parse_amount("1,000", 2).is_err()); // BYN: refused, good
    assert!(finance::parse_amount("1,000.50", 2).is_err());
}
