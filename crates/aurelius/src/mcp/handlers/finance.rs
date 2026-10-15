//! MCP surface over `aurelius_core::finance` — the same `fin_*` tables `au fin`
//! reads and writes. Every tool here reads or writes once per call and
//! returns; all money arithmetic lives in the core module.

use anyhow::{anyhow, bail, Result};
use aurelius_core::finance::{self, Kind, NewEntry, Source};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Value};

use super::open_db;

/// Keys one `entries[]` item of `fin_log` may carry. The top-level schema
/// check in `params.rs` does not reach inside array items, so an item key
/// typo ("ammount") would otherwise be dropped silently.
const ENTRY_KEYS: &[&str] = &[
    "kind",
    "amount",
    "currency",
    "counter_amount",
    "counter_currency",
    "category",
    "note",
    "occurred_at",
    "source",
    "provenance",
];

fn str_param<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Amount as given: a JSON string is taken verbatim, a JSON number through
/// its shortest decimal form. Strings are preferred — they never pass through
/// a float.
fn amount_param(v: &Value, key: &str) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(other) => bail!("'{key}' must be a decimal string, got {other}"),
    }
}

fn parse_entry(item: &Value, default_source: Source) -> Result<NewEntry> {
    let obj = item
        .as_object()
        .ok_or_else(|| anyhow!("each entry must be an object"))?;
    if let Some(unknown) = obj.keys().find(|k| !ENTRY_KEYS.contains(&k.as_str())) {
        bail!(
            "unknown entry field '{unknown}' (allowed: {})",
            ENTRY_KEYS.join(", ")
        );
    }
    let kind_s = str_param(item, "kind").ok_or_else(|| anyhow!("missing 'kind'"))?;
    let kind = Kind::parse(kind_s)
        .ok_or_else(|| anyhow!("unknown kind '{kind_s}' (expense | income | exchange)"))?;
    let source = match str_param(item, "source") {
        Some(s) => Source::parse(s)
            .ok_or_else(|| anyhow!("unknown source '{s}' (voice | text | manual | import)"))?,
        None => default_source,
    };
    Ok(NewEntry {
        kind,
        amount: amount_param(item, "amount")?.ok_or_else(|| anyhow!("missing 'amount'"))?,
        currency: str_param(item, "currency")
            .ok_or_else(|| anyhow!("missing 'currency'"))?
            .to_owned(),
        counter_amount: amount_param(item, "counter_amount")?,
        counter_currency: str_param(item, "counter_currency").map(str::to_owned),
        category: str_param(item, "category").map(str::to_owned),
        note: str_param(item, "note").map(str::to_owned),
        occurred_at: str_param(item, "occurred_at")
            .map(finance::parse_occurred)
            .transpose()?,
        source,
        provenance: str_param(item, "provenance").map(str::to_owned),
    })
}

pub fn fin_log(params: &Value) -> Result<Value> {
    fin_log_with_conn(&open_db()?, params)
}

fn fin_log_with_conn(conn: &Connection, params: &Value) -> Result<Value> {
    let default_source = match str_param(params, "source") {
        Some(s) => Source::parse(s)
            .ok_or_else(|| anyhow!("unknown source '{s}' (voice | text | manual | import)"))?,
        None => Source::Text,
    };
    let items = params
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("missing 'entries' array"))?;
    let entries = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            parse_entry(item, default_source).map_err(|e| anyhow!("entries[{i}]: {e}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let written = finance::log(conn, &entries, Utc::now())?;
    let lines: Vec<&str> = written.iter().map(|e| e.line.as_str()).collect();
    Ok(json!({ "logged": written.len(), "lines": lines, "entries": written }))
}

fn period_from(params: &Value) -> Result<finance::Period> {
    finance::resolve_period(
        str_param(params, "period"),
        str_param(params, "since"),
        str_param(params, "until"),
        Utc::now(),
    )
}

pub fn fin_list(params: &Value) -> Result<Value> {
    fin_list_with_conn(&open_db()?, params)
}

fn fin_list_with_conn(conn: &Connection, params: &Value) -> Result<Value> {
    let period = period_from(params)?;
    let kind = str_param(params, "kind")
        .map(|k| Kind::parse(k).ok_or_else(|| anyhow!("unknown kind '{k}'")))
        .transpose()?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(50, |l| usize::try_from(l).unwrap_or(usize::MAX));
    let entries = finance::list(
        conn,
        &finance::ListFilter {
            since: period.since,
            until: period.until,
            kind,
            category: str_param(params, "category").map(str::to_owned),
            currency: str_param(params, "currency").map(str::to_owned),
            include_voided: params
                .get("include_voided")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            limit,
        },
    )?;
    let lines: Vec<&str> = entries.iter().map(|e| e.line.as_str()).collect();
    Ok(json!({ "period": period, "count": entries.len(), "lines": lines, "entries": entries }))
}

pub fn fin_stats(params: &Value) -> Result<Value> {
    fin_stats_with_conn(&open_db()?, params)
}

fn fin_stats_with_conn(conn: &Connection, params: &Value) -> Result<Value> {
    let period = period_from(params)?;
    let by = str_param(params, "group_by").unwrap_or("category");
    let group_by = finance::GroupBy::parse(by)
        .ok_or_else(|| anyhow!("unknown group_by '{by}' (category | month | currency)"))?;
    let stats = finance::stats(conn, period.since, period.until, &period.label, group_by)?;
    let mut out = serde_json::to_value(&stats)?;
    out["text"] = json!(finance::stats_text(&stats));
    Ok(out)
}

pub fn fin_void(params: &Value) -> Result<Value> {
    fin_void_with_conn(&open_db()?, params)
}

fn fin_void_with_conn(conn: &Connection, params: &Value) -> Result<Value> {
    let id = params
        .get("id")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str()?.trim_start_matches('#').parse().ok())
        })
        .ok_or_else(|| anyhow!("missing or non-integer 'id'"))?;
    let reason = str_param(params, "reason").ok_or_else(|| anyhow!("missing 'reason'"))?;
    let entry = finance::void(conn, id, reason, Utc::now())?;
    Ok(json!({ "voided": true, "line": entry.line, "entry": entry }))
}

pub fn fin_categories(_params: &Value) -> Result<Value> {
    let conn = open_db()?;
    Ok(json!({ "categories": finance::categories(&conn)? }))
}

pub fn fin_currencies(params: &Value) -> Result<Value> {
    fin_currencies_with_conn(&open_db()?, params)
}

fn fin_currencies_with_conn(conn: &Connection, params: &Value) -> Result<Value> {
    let code = || str_param(params, "code").ok_or_else(|| anyhow!("missing 'code'"));
    match str_param(params, "action").unwrap_or("list") {
        "list" => {
            let all = params
                .get("include_disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(json!({ "currencies": finance::currencies(conn, all)? }))
        }
        "add" => {
            let kind = str_param(params, "kind").ok_or_else(|| anyhow!("missing 'kind'"))?;
            let scale = params
                .get("scale")
                .and_then(Value::as_u64)
                .and_then(|s| u32::try_from(s).ok())
                .ok_or_else(|| anyhow!("missing or invalid 'scale'"))?;
            Ok(json!({ "currency": finance::add_currency(conn, code()?, kind, scale)? }))
        }
        "disable" => Ok(json!({ "currency": finance::disable_currency(conn, code()?)? })),
        other => bail!("unknown action '{other}' (list | add | disable)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-fin-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        aurelius_core::db::open(&dir.join("t.db")).expect("open")
    }

    #[test]
    fn fin_log_batch_returns_one_line_per_entry_with_rate() {
        let c = conn();
        let out = fin_log_with_conn(
            &c,
            &json!({ "source": "voice", "entries": [
                { "kind": "expense", "amount": "25", "currency": "BYN", "category": "еда" },
                { "kind": "exchange", "amount": "75000", "currency": "RUB",
                  "counter_amount": 2500, "counter_currency": "BYN" }
            ]}),
        )
        .expect("log");
        assert_eq!(out["logged"], 2);
        let lines = out["lines"].as_array().expect("lines");
        assert!(lines[1]
            .as_str()
            .expect("line")
            .contains("курс 1 BYN = 30 RUB"));
        assert_eq!(out["entries"][0]["source"], "voice");
    }

    #[test]
    fn fin_log_rejects_unknown_entry_field_and_missing_side() {
        let c = conn();
        let typo = json!({ "entries": [{ "kind": "expense", "ammount": "1", "currency": "BYN" }] });
        assert!(fin_log_with_conn(&c, &typo).is_err());
        let half = json!({ "entries": [{ "kind": "exchange", "amount": "30", "currency": "USD",
                                         "counter_currency": "BYN" }] });
        let err = fin_log_with_conn(&c, &half).expect_err("missing side");
        assert!(format!("{err:#}").contains("спроси владельца"), "{err:#}");
    }

    #[test]
    fn fin_stats_and_void_round_trip() {
        let c = conn();
        let out = fin_log_with_conn(
            &c,
            &json!({ "entries": [
                { "kind": "expense", "amount": "10", "currency": "BYN", "category": "еда" }
            ]}),
        )
        .expect("log");
        let id = out["entries"][0]["id"].as_i64().expect("id");
        let stats = fin_stats_with_conn(&c, &json!({ "period": "all" })).expect("stats");
        assert_eq!(stats["totals"][0]["expense"], "10");
        fin_void_with_conn(&c, &json!({ "id": id, "reason": "дубль" })).expect("void");
        let stats = fin_stats_with_conn(&c, &json!({ "period": "all" })).expect("stats");
        assert_eq!(stats["totals"].as_array().map(Vec::len), Some(0));
        assert_eq!(stats["voided_excluded"], 1);
    }

    #[test]
    fn fin_currencies_add_and_disable() {
        let c = conn();
        let added = fin_currencies_with_conn(
            &c,
            &json!({ "action": "add", "code": "doge", "kind": "crypto", "scale": 8 }),
        )
        .expect("add");
        assert_eq!(added["currency"]["code"], "DOGE");
        fin_currencies_with_conn(&c, &json!({ "action": "disable", "code": "DOGE" }))
            .expect("disable");
        let listed = fin_currencies_with_conn(&c, &json!({})).expect("list");
        let codes: Vec<&str> = listed["currencies"]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|c| c["code"].as_str())
            .collect();
        assert!(codes.contains(&"USDT") && !codes.contains(&"DOGE"));
    }
}
