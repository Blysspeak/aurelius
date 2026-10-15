# Personal finance

[Back to README](../README.md)

A personal spending ledger inside the same memory the agent already uses. The owner says
"spent 25 BYN on groceries" by voice or text, the agent turns the phrase into entries through
`fin_log`, and reads back one line per entry with its USD equivalent. Totals come from
`fin_stats` or `au fin stats`. Added in v3.8.0.

## Contents

- [What it is for](#what-it-is-for)
- [Entry model](#entry-model)
- [Currencies](#currencies)
- [Rates and conversion to USD](#rates-and-conversion-to-usd)
- [MCP tools](#mcp-tools)
- [CLI](#cli)
- [Storage](#storage)

## What it is for

One person's money: how much, in which currency, on what. There are no accounts, cards, places
or budgets — only amount, currency and category. A card or a shop the owner mentions may go into
the free-text `note`. The agent is the input device: one spoken phrase may produce several
entries, and they are logged together as one batch.

The rules the agent follows are part of the `fin_log` tool description, so every session gets
them: reuse a category from `fin_categories` instead of inventing a near-duplicate, ask the owner
about an unknown currency code instead of adding one, and never fill in a missing side of an
exchange from a bank rate — ask.

## Entry model

Every entry has a `kind`:

| Kind | Meaning | Counted as |
|------|---------|------------|
| `expense` | money spent | expense |
| `income` | money received | income |
| `exchange` | one currency turned into another | neither — reported as `exchanged_out` / `exchanged_in` |

- **Exact amounts.** An amount is stored as an integer number of the currency's minor units
  (`amount_minor`); the currency's `scale` (decimal places) says where the point goes —
  `25` BYN at scale 2 is `2500`, `0.00000001` BTC at scale 8 is `1`. Amounts come in as decimal
  strings (`"25"`, `"11.6"`). More decimals than the scale allows is refused, never rounded: a
  rounded amount is no longer the one the owner said. A fractional JSON number is refused too
  (it already went through a float), and `"1,000"` is refused as ambiguous.
- **Both sides of an exchange.** An exchange stores what was given (`amount`, `currency`) and
  what was received (`counter_amount`, `counter_currency`). The effective rate is derived from
  the two sides when the entry is read, not stored.
- **Category.** Lowercase free text, required for expense and income, optional for exchange.
- **When.** `occurred_at` is `YYYY-MM-DD`, `YYYY-MM-DD HH:MM` (local time) or RFC 3339; it
  defaults to now. Conversion uses this date, not the date the entry was written.
- **Source.** `voice`, `text`, `manual` or `import` — how the owner said it — plus an optional
  free-text `provenance` such as a bot message id.
- **Void, never delete.** A wrong entry is voided with a reason (`fin_void`, `au fin void`): the
  row stays with `voided_at` and `void_reason`, and leaves lists and totals. The correction is a
  new entry. Voiding an already voided entry is refused.
- **All or nothing.** A `fin_log` batch with one invalid entry writes nothing, and the error
  names the entry by its index.

## Currencies

A fresh database knows these currencies:

| Kind | Code | Scale |
|------|------|-------|
| fiat | BYN, EUR, KZT, RUB, USD | 2 |
| crypto | USDT, USDC | 6 |
| crypto | BTC, ETH, TON, SOL, BNB, XRP, TRX | 8 |

Adding one takes a code (2-10 latin letters or digits), a kind (`fiat` or `crypto`) and a scale
(0-8). The agent adds a currency only after the owner confirms the code. The scale of an existing
currency never changes; adding a disabled currency re-enables it. Disabling a currency refuses new
entries in it and keeps old ones in totals.

```bash
au fin currency add GEL --kind fiat --scale 2
au fin currency disable XRP
au fin currency list --all
```

## Rates and conversion to USD

Totals are in USD. USDT is not assumed equal to USD: it is priced by the market like any crypto.

**Conversion happens at query time**, at the rate for each entry's own date. Nothing is converted
when an entry is written, so correcting a rate re-prices the history that depends on it.

**Which rate an entry gets**, highest priority first:

1. **Dictated** — the owner named a rate for this operation ("at 3.25"): `fin_rate action=set`
   or `au fin rate set`. Beats every other source.
2. **Own exchange** — an exchange with USD on one side converts at its own actual rate.
3. **The currency's official source** on the entry date — a national bank for fiat, the Kraken
   market for crypto:

| Currency | Source |
|----------|--------|
| BYN | NBRB (National Bank of the Republic of Belarus) |
| RUB | CBR (Central Bank of Russia) |
| KZT | NBK (National Bank of Kazakhstan) |
| EUR and any other fiat | ECB (European Central Bank, through EUR) |
| USDT, USDC and every crypto | Kraken, daily candle |

No source needs an API key. "At the bank rate" without a number needs no call: the national bank
rate on the entry date is already the default. `fin_rate action=clear` removes a dictated rate
and the order above applies again.

**Missing days.** When a source has no rate for the date itself (weekends, the CBR's January
holidays), the nearest earlier cached rate within 14 days is used. A rate more than a day older
than the operation, taken from the cache because the source did not answer, is marked **stale**:
`fin_stats` reports `usd.stale_entries` and `usd.oldest_rate_date`, and the total should be read as
approximate. A past date without its own rate gets the rate that was in force pinned to it, so the
source is not asked again.

**Unconverted bucket.** An entry with no rate at all is never dropped from a total. Its line says
`нет курса в USD (не пересчитано)` ("no USD rate, not converted"), and `fin_stats` lists it under
`usd.unconverted` with the count, the ids and the native sums.

**Network budget.** One list or summary may spend at most 20 seconds on the network; after that
only the cache answers. A source that failed once in a pass is not asked again in that pass.
Failures and skips are reported in `rate_errors`, and a network failure only leaves entries
unconverted — it never fails the call. `base=none` (`au fin stats --base none`) skips rates and
the network entirely.

`au fin rate refresh --date D` forgets the cached bank and market rates for that date (dictated
rates stay) and fetches them again — for when a source corrected a published rate.

## MCP tools

Seven tools; parameter tables are in [MCP Tools](mcp-tools.md#finance).

| Tool | What it does |
|------|--------------|
| [`fin_log`](mcp-tools.md#fin_log) | Record a batch of entries; returns one line per entry to read back |
| [`fin_list`](mcp-tools.md#fin_list) | Entries for a period, each with its USD equivalent |
| [`fin_stats`](mcp-tools.md#fin_stats) | Totals in USD and per currency, grouped by category, month or currency |
| [`fin_rate`](mcp-tools.md#fin_rate) | Set or clear a dictated rate, list the cache, refresh a date |
| [`fin_void`](mcp-tools.md#fin_void) | Void an entry with a reason |
| [`fin_categories`](mcp-tools.md#fin_categories) | Existing categories with counts and last use |
| [`fin_currencies`](mcp-tools.md#fin_currencies) | List, add or disable currencies |

A phrase with three movements becomes one call:

```json
{
  "source": "voice",
  "entries": [
    {"kind": "expense", "amount": "25", "currency": "BYN", "category": "groceries", "note": "Green, card"},
    {"kind": "income", "amount": "1500", "currency": "USD", "category": "salary"},
    {"kind": "exchange", "amount": "100", "currency": "USD", "counter_amount": "325", "counter_currency": "BYN"}
  ]
}
```

Each returned entry carries `line`, the text to read back. Lines are printed in Russian, the
owner's language; for the batch above:

```
#1 расход 25 BYN ≈ 8.3 USD (3.0113 BYN за 1 USD, НБРБ) · groceries · 15.10.2026 · Green, card
#2 доход 1500 USD ≈ 1500 USD (USD) · salary · 15.10.2026
#3 обмен 100 USD -> 325 BYN · курс 1 USD = 3.25 BYN ≈ 100 USD (3.25 BYN за 1 USD, фактический обмен) · 15.10.2026
```

Entry #1 used the NBRB rate of the day; #3 converted at its own rate because one side is USD.
Then the owner says the groceries were paid at 3.25:

```json
{"action": "set", "entry_id": 1, "rate": "3.25"}
```

`rate_unit` defaults to `per_usd` for fiat (3.25 BYN for 1 USD) and `usd_per_unit` for crypto
(1.4 USD for 1 TON); pass it to read the number the other way.

## CLI

`au fin` does the same from the shell, one entry per `au fin log`. Every subcommand takes `--json`.
Flags: [CLI reference](cli.md#finance).

```bash
au fin log --kind expense --amount 12.50 --currency EUR --category cafe --at 2026-10-14
au fin log --kind exchange --amount 50 --currency USDT --counter-amount 4100 --counter-currency RUB
au fin list --period week               # today | yesterday | week | month | year | all | <N>d
au fin stats                            # this month, by category, in USD
au fin stats --period year --by month   # by month
au fin stats --base none                # native currencies only, no network
au fin void 4 --reason "wrong amount, re-logged as #6"
au fin categories
au fin rate set 1 3.25                  # dictated rate for entry #1
au fin rate clear 1
au fin rate list --currency BYN --limit 10
au fin rate refresh --date 2026-10-14 --currency RUB
```

A refused entry exits `1` and says why, for example an amount with more decimals than the
currency's scale.

## Storage

The tables live in the same SQLite database as the graph: `fin_currencies`, `fin_entries` and
`fin_rates`. They are created lazily, outside the schema version (which stays at 19), so an older
binary keeps working on the same database. `fin_currencies` and `fin_entries` appear on the first
finance call. `fin_rates` appears on the first call that converts to USD: the first summary in USD
(`fin_stats`, `au fin stats`) creates it in a live database that has none yet, and so do
`fin_log`/`fin_list` (their lines carry the USD equivalent) and every `fin_rate` action. A summary
with `base=none` never touches it.

Rates are stored as exact fractions (`rate_num / rate_den` = USD per unit); no floating point is
used for money. A dictated rate is a `fin_rates` row bound to its entry; the entry row itself never
changes.
