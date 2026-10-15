#!/usr/bin/env python3
"""Stress the aurelius finance module through the built binaries (task c843b315).

Writes N entries in batches through the real MCP server (`aurelius`, stdio,
tool `fin_log`), voids a share of them through `fin_void`, then compares
`au fin stats --json` (whole range by category/month/currency, and every month
separately) with sums computed here with Python ints. Also feeds the owner's
phrasing through `fin_log` as JSON strings and JSON numbers.

Everything runs against a fresh AURELIUS_HOME=/tmp/fin-stress-<random>, which is
deleted at the end. Run with `python3 -I scripts/fin_stress.py [--entries N]`.
Exit code 0 = all sums match; 1 = mismatch; the phrase table is informational.
"""

import argparse
import json
import os
import random
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from collections import defaultdict
from datetime import datetime, timezone

CURRENCIES = {
    "BYN": 2, "RUB": 2, "USD": 2, "EUR": 2, "KZT": 2,
    "USDT": 6, "USDC": 6,
    "BTC": 8, "ETH": 8, "TON": 8,
}
CATEGORIES = [f"cat{i:02d}" for i in range(30)] + ["еда", "транспорт", "подписки"]
TS_LO = int(datetime(2024, 1, 1, tzinfo=timezone.utc).timestamp())
TS_HI = int(datetime(2026, 10, 1, tzinfo=timezone.utc).timestamp())


def fmt(minor, scale, comma=False):
    if scale == 0:
        return str(minor)
    unit = 10 ** scale
    i, f = divmod(minor, unit)
    if f == 0:
        return str(i)
    s = f"{i}.{f:0{scale}d}".rstrip("0")
    return s.replace(".", ",") if comma else s


def local_month(ts):
    t = time.localtime(ts)
    return f"{t.tm_year:04d}-{t.tm_mon:02d}"


class Mcp:
    def __init__(self, binary, env, log_path):
        self.log = open(log_path, "wb")
        self.p = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=self.log, env=env)
        self.next_id = 0
        self.noise = 0
        self.call_raw("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                     "clientInfo": {"name": "fin-stress", "version": "0"}})
        self.notify("notifications/initialized")

    def notify(self, method):
        self.p.stdin.write((json.dumps({"jsonrpc": "2.0", "method": method}) + "\n").encode())
        self.p.stdin.flush()

    def call_raw(self, method, params):
        self.next_id += 1
        msg = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.p.stdin.write((json.dumps(msg, ensure_ascii=False) + "\n").encode())
        self.p.stdin.flush()
        return self.read_response()

    def read_response(self):
        # The server's tracing subscriber writes log lines to stdout too
        # (tracing_subscriber::fmt::init); skip anything that is not JSON-RPC.
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("MCP server closed stdout")
            if line.lstrip().startswith(b"{"):
                return json.loads(line)
            self.noise += 1

    def tool(self, name, args):
        """Returns (ok, payload) where payload is parsed JSON on success, text on error."""
        r = self.call_raw("tools/call", {"name": name, "arguments": args})
        res = r.get("result") or {}
        text = res.get("content", [{}])[0].get("text", json.dumps(r))
        if res.get("isError") or "error" in r:
            return False, text
        return True, json.loads(text)

    def tool_raw_json(self, name, args_json):
        """Send arguments verbatim (to control JSON number literals)."""
        self.next_id += 1
        line = ('{"jsonrpc":"2.0","id":%d,"method":"tools/call","params":{"name":"%s","arguments":%s}}\n'
                % (self.next_id, name, args_json))
        self.p.stdin.write(line.encode())
        self.p.stdin.flush()
        r = self.read_response()
        res = r.get("result") or {}
        text = res.get("content", [{}])[0].get("text", json.dumps(r))
        if res.get("isError") or "error" in r:
            return False, text
        return True, json.loads(text)

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=30)
        self.log.close()


def gen_entry(rng):
    r = rng.random()
    kind = "expense" if r < 0.6 else ("income" if r < 0.8 else "exchange")
    cur = rng.choice(list(CURRENCIES))
    scale = CURRENCIES[cur]
    # Log-uniform from 1 minor unit up to ~10^12 minor units.
    minor = max(1, int(10 ** rng.uniform(0, 12)))
    ts = rng.randrange(TS_LO, TS_HI)
    e = {"kind": kind, "currency": cur, "minor": minor, "ts": ts}
    item = {"kind": kind, "amount": fmt(minor, scale, comma=rng.random() < 0.1),
            "currency": cur.lower() if rng.random() < 0.1 else cur,
            "occurred_at": datetime.fromtimestamp(ts, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "source": "import"}
    if kind == "exchange":
        ccur = rng.choice([c for c in CURRENCIES if c != cur])
        cminor = max(1, int(10 ** rng.uniform(0, 12)))
        e.update(ccur=ccur, cminor=cminor)
        item.update(counter_amount=fmt(cminor, CURRENCIES[ccur]), counter_currency=ccur)
        if rng.random() < 0.5:
            item["category"] = "обмен"
    else:
        cat = rng.choice(CATEGORIES)
        e["category"] = cat
        item["category"] = cat.upper() if rng.random() < 0.05 else cat
    return e, item


def expected(entries, since=None, until=None, by="category"):
    totals = defaultdict(lambda: [0, 0, 0, 0, 0])  # expense income out in entries
    groups = defaultdict(lambda: [0, 0, 0])
    voided = 0
    for e in entries:
        if since is not None and e["ts"] < since:
            continue
        if until is not None and e["ts"] >= until:
            continue
        if e.get("voided"):
            voided += 1
            continue
        t = totals[e["currency"]]
        t[4] += 1
        if e["kind"] == "exchange":
            t[2] += e["minor"]
            totals[e["ccur"]][3] += e["cminor"]
            continue
        key = {"category": lambda: e["category"], "month": lambda: local_month(e["ts"]),
               "currency": lambda: e["currency"]}[by]()
        g = groups[(e["currency"], key)]
        g[2] += 1
        if e["kind"] == "expense":
            t[0] += e["minor"]
            g[0] += e["minor"]
        else:
            t[1] += e["minor"]
            g[1] += e["minor"]
    return ({k: tuple(v) for k, v in totals.items()},
            {k: tuple(v) for k, v in groups.items()}, voided)


def from_stats(s):
    totals = {t["currency"]: (t["expense_minor"], t["income_minor"], t["exchanged_out_minor"],
                              t["exchanged_in_minor"], t["entries"]) for t in s["totals"]}
    groups = {(g["currency"], g["key"]): (g["expense_minor"], g["income_minor"], g["entries"])
              for g in s["groups"]}
    return totals, groups, s["voided_excluded"]


def au_stats(au, env, *args):
    t0 = time.perf_counter()
    p = subprocess.run([au, "fin", "stats", "--json", *args], env=env, capture_output=True, text=True)
    dt = time.perf_counter() - t0
    if p.returncode != 0:
        raise RuntimeError(f"au fin stats {args} exit {p.returncode}: {p.stderr.strip()}")
    return json.loads(p.stdout), dt


def diff(name, got, want):
    if got == want:
        return True
    gt, gg, gv = got
    wt, wg, wv = want
    print(f"MISMATCH {name}: voided got={gv} want={wv}")
    for k in sorted(set(gt) | set(wt)):
        if gt.get(k) != wt.get(k):
            print(f"  total {k}: got={gt.get(k)} want={wt.get(k)}")
    bad = [k for k in set(gg) | set(wg) if gg.get(k) != wg.get(k)]
    for k in sorted(bad)[:10]:
        print(f"  group {k}: got={gg.get(k)} want={wg.get(k)}")
    print(f"  ({len(bad)} groups differ)")
    return False


PHRASES = [
    # (label, raw JSON item text, expected minor or None if it should be refused)
    ("25 BYN string", '{"kind":"expense","amount":"25","currency":"BYN","category":"еда"}', 2500),
    ("25.5 BYN string", '{"kind":"expense","amount":"25.5","currency":"BYN","category":"еда"}', 2550),
    ("25,50 BYN string", '{"kind":"expense","amount":"25,50","currency":"BYN","category":"еда"}', 2550),
    ("75000 RUB string", '{"kind":"expense","amount":"75000","currency":"RUB","category":"еда"}', 7500000),
    ("0.00000001 BTC string", '{"kind":"expense","amount":"0.00000001","currency":"BTC","category":"комиссии"}', 1),
    ("11.6 TON string", '{"kind":"expense","amount":"11.6","currency":"TON","category":"еда"}', 1160000000),
    ("25 BYN number", '{"kind":"expense","amount":25,"currency":"BYN","category":"еда"}', 2500),
    ("25.5 BYN number", '{"kind":"expense","amount":25.5,"currency":"BYN","category":"еда"}', 2550),
    ("25.50 BYN number", '{"kind":"expense","amount":25.50,"currency":"BYN","category":"еда"}', 2550),
    ("75000 RUB number", '{"kind":"expense","amount":75000,"currency":"RUB","category":"еда"}', 7500000),
    ("0.00000001 BTC number", '{"kind":"expense","amount":0.00000001,"currency":"BTC","category":"комиссии"}', 1),
    ("11.6 TON number", '{"kind":"expense","amount":11.6,"currency":"TON","category":"еда"}', 1160000000),
    ("8589934592.064006 USDT number", '{"kind":"expense","amount":8589934592.064006,"currency":"USDT","category":"x"}', 8589934592064006),
    ("1500 RUB -> 11.6 TON exchange numbers", '{"kind":"exchange","amount":1500,"currency":"RUB","counter_amount":11.6,"counter_currency":"TON"}', 150000),
    ("25.123 BYN string (too precise)", '{"kind":"expense","amount":"25.123","currency":"BYN","category":"еда"}', None),
    ("1,000 USDT string (thousands comma?)", '{"kind":"expense","amount":"1,000","currency":"USDT","category":"x"}', 1000000000),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--entries", type=int, default=200_000)
    ap.add_argument("--batch", type=int, default=1000)
    ap.add_argument("--void-share", type=float, default=0.05)
    ap.add_argument("--seed", type=int, default=20261015)
    ap.add_argument("--bin-dir", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                      "..", "target", "release"))
    a = ap.parse_args()
    au = os.path.abspath(os.path.join(a.bin_dir, "au"))
    srv = os.path.abspath(os.path.join(a.bin_dir, "aurelius"))

    home = tempfile.mkdtemp(prefix="fin-stress-", dir="/tmp")
    assert home.startswith("/tmp/fin-stress-"), home
    env = dict(os.environ, AURELIUS_HOME=home)
    print(f"AURELIUS_HOME={home}")
    ok_all = True
    rng = random.Random(a.seed)
    try:
        mcp = Mcp(srv, env, os.path.join(home, "mcp.stderr"))

        # --- log in batches -------------------------------------------------
        entries = []
        batch_times = []
        t_all = time.perf_counter()
        n_batches = (a.entries + a.batch - 1) // a.batch
        for b in range(n_batches):
            size = min(a.batch, a.entries - b * a.batch)
            pairs = [gen_entry(rng) for _ in range(size)]
            t0 = time.perf_counter()
            ok, out = mcp.tool("fin_log", {"entries": [p[1] for p in pairs]})
            batch_times.append(time.perf_counter() - t0)
            if not ok:
                print(f"batch {b} refused: {out[:300]}")
                return 1
            for (e, _), w in zip(pairs, out["entries"]):
                e["id"] = w["id"]
                if w["amount_minor"] != e["minor"]:
                    print(f"amount mismatch id={w['id']}: got {w['amount_minor']} want {e['minor']}")
                    ok_all = False
            entries.extend(p[0] for p in pairs)
        log_wall = time.perf_counter() - t_all
        print(f"logged {len(entries)} entries in {n_batches} batches of {a.batch}: "
              f"wall {log_wall:.1f}s, batch median {statistics.median(batch_times)*1000:.0f}ms, "
              f"p95 {sorted(batch_times)[int(len(batch_times)*0.95)]*1000:.0f}ms, "
              f"max {max(batch_times)*1000:.0f}ms, first {batch_times[0]*1000:.0f}ms, "
              f"last {batch_times[-1]*1000:.0f}ms")

        # --- void a share ---------------------------------------------------
        to_void = rng.sample(entries, int(len(entries) * a.void_share))
        t0 = time.perf_counter()
        for e in to_void:
            ok, out = mcp.tool("fin_void", {"id": e["id"], "reason": "stress"})
            if not ok:
                print(f"void {e['id']} failed: {out[:200]}")
                return 1
            e["voided"] = True
        dt = time.perf_counter() - t0
        print(f"voided {len(to_void)} entries one call each: {dt:.1f}s ({dt/len(to_void)*1000:.2f}ms/call)")
        mcp.close()

        # --- whole range ----------------------------------------------------
        for by in ("category", "month", "currency"):
            s, dt = au_stats(au, env, "--period", "all", "--by", by)
            good = diff(f"all/{by}", from_stats(s), expected(entries, by=by))
            ok_all &= good
            print(f"au fin stats --period all --by {by}: {dt*1000:.0f}ms, "
                  f"{len(s['totals'])} currencies, {len(s['groups'])} groups, match={good}")

        # --- every month ----------------------------------------------------
        month_times = []
        months = sorted({local_month(e["ts"]) for e in entries})
        bad_months = 0
        for m in months:
            y, mo = map(int, m.split("-"))
            ny, nmo = (y + 1, 1) if mo == 12 else (y, mo + 1)
            since = f"{y:04d}-{mo:02d}-01"
            until = f"{ny:04d}-{nmo:02d}-01"
            lo = int(time.mktime((y, mo, 1, 0, 0, 0, 0, 0, -1)))
            hi = int(time.mktime((ny, nmo, 1, 0, 0, 0, 0, 0, -1)))
            s, dt = au_stats(au, env, "--since", since, "--until", until, "--by", "category")
            month_times.append(dt)
            if not diff(f"month {m}", from_stats(s), expected(entries, lo, hi, "category")):
                bad_months += 1
        ok_all &= bad_months == 0
        print(f"per-month stats: {len(months)} months, {bad_months} mismatched, "
              f"median {statistics.median(month_times)*1000:.0f}ms, max {max(month_times)*1000:.0f}ms")

        # --- owner's phrasing through fin_log -------------------------------
        mcp = Mcp(srv, env, os.path.join(home, "mcp2.stderr"))
        print("phrases through MCP fin_log:")
        for label, item, want in PHRASES:
            ok, out = mcp.tool_raw_json("fin_log", '{"entries":[%s]}' % item)
            got = out["entries"][0]["amount_minor"] if ok else None
            if want is None:
                verdict = "OK (refused)" if not ok else f"BUG accepted as {got}"
            elif ok and got == want:
                verdict = "OK"
            elif ok:
                verdict = f"{'NOTE' if '?' in label else 'BUG'} stored {got}, said {want}"
            else:
                verdict = f"REFUSED: {out[:140]}"
            print(f"  {label:40s} -> {verdict}")
        mcp.close()
        live = os.path.expanduser("~/.local/share/aurelius")
        assert not home.startswith(live)
    finally:
        shutil.rmtree(home, ignore_errors=True)
        print(f"removed {home}: {not os.path.exists(home)}")
    print("RESULT", "PASS" if ok_all else "FAIL")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
