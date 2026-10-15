#!/usr/bin/env python3
"""Build the flag tables of docs/cli.md from the `au` binary itself.

Walks `au --help` recursively, and for every subcommand rewrites the block
between `<!-- au:PATH -->` and `<!-- /au -->` (PATH is the subcommand path,
e.g. `fin rate set`) with its usage line and a table of arguments and options.
Help strings that are in Russian in the binary are replaced by the English
text in TRANSLATED; an untranslated one stops the script, so the docs never
drift into a second language silently.

    python3 scripts/dump-au-help.py --list                   # every subcommand path
    python3 scripts/dump-au-help.py --update docs/cli.md     # refresh blocks in place
    python3 scripts/dump-au-help.py --check  docs/cli.md     # exit 1 when stale
    AU=target/release/au python3 scripts/dump-au-help.py ...  # another binary
"""
import os
import re
import subprocess
import sys
from pathlib import Path

AU = os.environ.get("AU", "au")
CYRILLIC = re.compile("[Ѐ-ӿ]")

# English text for help strings the binary prints in Russian, keyed by the
# command path and the flag or argument.
TRANSLATED = {
    "daemon --interval": "Pause between ticks, seconds",
    "daemon --grace": "How long after the due moment a reminder waits for a live session before the out-of-session channel takes it; same delay grammar as `au remind add --in`",
    "daemon --once": "One tick instead of a loop, then exit: how the daemon is tested without sleeping, and how a systemd timer can replace the long-running service",
    "daemon --json": "Print each tick as one JSON line instead of human-readable output",
    "fin log --amount": "Amount in currency units, e.g. `25` or `11.6`; for an exchange, the amount given",
    "fin log --currency": "Currency code; for an exchange, the currency given",
    "fin log --counter-amount": "Exchange only: the amount received",
    "fin log --counter-currency": "Exchange only: the currency received",
    "fin log --category": "Category, lowercase; required for expense and income",
    "fin log --at": "When: `YYYY-MM-DD`, `YYYY-MM-DD HH:MM` or RFC 3339; defaults to now",
    "fin log --provenance": "Where the entry came from, e.g. a bot message id",
    "fin list --all": "Show voided entries too",
    "fin stats --period": "today | yesterday | week | month | year | all | <N>d; defaults to month",
    "fin stats --base": "USD | none (none: native currencies only, no rates, no network)",
    "fin currency add --scale": "Decimal places, 0-8",
    "fin rate set ENTRY_ID": "Entry id (the #N in entry lines)",
    "fin rate set RATE": "The rate as a decimal string, e.g. 3.25",
    "fin rate set --currency": "Exchange only: which side the rate is for; defaults to the entry currency",
    "fin rate set --unit": "per_usd (units for 1 USD, default for fiat) | usd_per_unit (USD for 1 unit, default for crypto)",
    "fin rate list --date": "Exactly this date, YYYY-MM-DD",
    "fin rate refresh --date": "YYYY-MM-DD",
    "fin rate refresh --currency": "One currency; defaults to every enabled one",
    "secret add --name": "Secret name, e.g. STRIPE_SECRET_KEY",
    "secret add --where": "Where it is kept: an environment variable, a file path or a password-manager entry, NOT the value",
    "secret add --purpose": "What the secret is for, in plain words",
    "secret add --project": "Project the secret belongs to",
    "secret rm NAME": "Secret name",
}

# Meanings for flags the binary prints without any help text.
FILLED = {
    "context TOPIC": "Topic to expand the graph around",
    "search QUERY": "Search query",
    "task list --json": "Print JSON instead of human-readable text",
    "task ripe --json": "Print JSON instead of human-readable text",
    "task renew --lease-minutes": "New lease duration in minutes",
    "task renew --json": "Print JSON instead of human-readable text",
    "task release --id": "Work order id issued by `claim`",
    "task release --owner": "The same owner that took the work order",
    "task release --json": "Print JSON instead of human-readable text",
    "task give-up --id": "Work order id issued by `claim`",
    "task give-up --owner": "The same owner that took the work order",
    "task give-up --json": "Print JSON instead of human-readable text",
    "task fitness --json": "Print JSON instead of human-readable text",
    "remind list --json": "Print JSON instead of human-readable text",
    "remind show --json": "Print JSON instead of human-readable text",
    "remind snooze --in": "Delay from now, same grammar as `au remind add --in`",
    "remind snooze --at": "Absolute moment, same grammar as `au remind add --at`",
    "fin log --note": "Anything else worth keeping: card, place, counterparty",
    "fin log --json": "Print the written entry as JSON",
    "fin list --since": "Start, inclusive: YYYY-MM-DD or RFC 3339; overrides the period start",
    "fin list --until": "End, exclusive: YYYY-MM-DD or RFC 3339",
    "fin list --kind": "expense | income | exchange",
    "fin list --category": "Only this category",
    "fin list --currency": "Only this currency, either side of an exchange",
    "fin list --json": "Print JSON instead of entry lines",
    "fin list --limit": "Most entries to print",
    "fin rate list --limit": "Most rates to print",
    "fin stats --since": "Start, inclusive: YYYY-MM-DD or RFC 3339",
    "fin stats --until": "End, exclusive: YYYY-MM-DD or RFC 3339",
    "fin stats --json": "Print JSON instead of the text summary",
    "fin void ID": "Entry id (the #N in entry lines)",
    "fin void --reason": "Why the entry is voided, e.g. `wrong amount, re-logged as #14`",
    "fin void --json": "Print JSON instead of the entry line",
    "fin categories --json": "Print JSON instead of human-readable lines",
    "fin currency list --all": "Include disabled currencies",
    "fin currency list --json": "Print JSON instead of human-readable lines",
    "fin currency add CODE": "Currency code, 2-10 latin letters or digits",
    "fin currency add --json": "Print JSON instead of a human-readable line",
    "fin currency disable CODE": "Currency code",
    "fin currency disable --json": "Print JSON instead of a human-readable line",
    "fin rate set --json": "Print JSON instead of the entry line",
    "fin rate clear ENTRY_ID": "Entry id (the #N in entry lines)",
    "fin rate clear --currency": "Exchange only: which side to clear; defaults to the entry currency",
    "fin rate clear --json": "Print JSON instead of the entry line",
    "fin rate list --currency": "Only this currency",
    "fin rate list --since": "From this date, YYYY-MM-DD, inclusive",
    "fin rate list --until": "Up to this date, YYYY-MM-DD, inclusive",
    "fin rate list --entry-id": "Only the dictated rate of this entry",
    "fin rate list --json": "Print JSON instead of human-readable lines",
    "fin rate refresh --json": "Print JSON instead of human-readable lines",
    "secret list --json": "Print JSON instead of human-readable lines",
    "secret rm --project": "Project the secret belongs to",
    "capture --hook": "Accepted for parsing compatibility only",
    "capture --command": "Accepted for parsing compatibility only",
    "identity set --name": "Your name",
    "identity set --email": "Your email",
}

ENTRY = re.compile(r"^ {2,6}(?:(-\S, )?(--?[\w-]+)(?: <([^>]+)>)?|([<\[])([A-Z_]+)[>\]](\.\.\.)?)(?:\s{2,}(.*))?$")


def run_help(path):
    out = subprocess.run([AU, *path, "--help"], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"au {' '.join(path)} --help exited {out.returncode}")
    return out.stdout


def walk(path=()):
    text = run_help(list(path))
    yield path, text
    m = re.search(r"\nCommands:\n(.*?)(?:\n\n|\Z)", text, re.S)
    if m:
        for line in m.group(1).splitlines():
            name = re.match(r"  (\S+)", line)
            if name and name.group(1) != "help":
                yield from walk(path + (name.group(1),))


def section(text, title):
    m = re.search(rf"\n{title}:\n(.*?)(?:\n\n(?=\S)|\Z)", text, re.S)
    return m.group(1) if m else ""


def entries(text):
    """(key, display, default, meaning) for each argument and option."""
    out = []
    for title in ("Arguments", "Options"):
        cur = None
        for line in section(text, title).splitlines():
            m = ENTRY.match(line)
            if m:
                if m.group(2):
                    flag = m.group(2)
                    if flag in ("-h", "--help", "-V", "--version"):
                        cur = None
                        continue
                    shown = (m.group(1) or "") + flag + (f" <{m.group(3)}>" if m.group(3) else "")
                    cur = [flag, shown, m.group(7) or ""]
                else:
                    name = m.group(5)
                    shown = f"{m.group(4)}{name}{'>' if m.group(4) == '<' else ']'}{m.group(6) or ''}"
                    cur = [name, shown, m.group(7) or ""]
                out.append(cur)
            elif cur is not None and line.strip():
                cur[2] = (cur[2] + " " + line.strip()).strip()
    rows = []
    for key, shown, desc in out:
        default = ""
        d = re.search(r"\s*\[default: ([^\]]*)\]\s*$", desc)
        if d:
            default, desc = d.group(1), desc[: d.start()]
        rows.append((key, shown, default, desc.strip()))
    return rows


def cell(text):
    return " ".join(text.split()).replace("|", "\\|")


def block(path, text):
    usage = re.search(r"^Usage: (.*)$", text, re.M).group(1)
    lines = ["```", usage, "```", ""]
    rows = entries(text)
    if not rows:
        lines.append("No arguments or options besides `--help`.")
        return "\n".join(lines) + "\n"
    lines += ["| Argument / option | Default | Meaning |", "|---|---|---|"]
    for key, shown, default, desc in rows:
        k = f"{' '.join(path)} {key}"
        if CYRILLIC.search(desc) or not desc:
            desc = TRANSLATED.get(k) or FILLED.get(k) or desc
        if CYRILLIC.search(desc):
            sys.exit(f"untranslated help for `au {k}`: {desc}")
        lines.append(f"| `{cell(shown)}` | {f'`{cell(default)}`' if default else ''} | {cell(desc) or '—'} |")
    return "\n".join(lines) + "\n"


BLOCK = re.compile(r"(<!-- au:([a-z -]*?) -->\n)(.*?)(<!-- /au -->)", re.S)


def main():
    cmds = {" ".join(p): t for p, t in walk() if p}
    if "--list" in sys.argv:
        print("\n".join(cmds))
        return
    for flag in ("--update", "--check"):
        if flag in sys.argv:
            path = Path(sys.argv[sys.argv.index(flag) + 1])
            doc = path.read_text()
            found = [m.group(2) for m in BLOCK.finditer(doc)]
            missing = [c for c in cmds if c not in found]
            unknown = [c for c in found if c not in cmds]
            if missing or unknown:
                sys.exit(f"{path}: no block for {missing}; blocks for unknown commands {unknown}")
            new = BLOCK.sub(lambda m: m.group(1) + block(m.group(2).split(), cmds[m.group(2)]) + m.group(4), doc)
            if flag == "--update":
                path.write_text(new)
                print(f"{path}: {len(found)} command blocks written")
            elif new != doc:
                sys.exit(f"{path}: command blocks differ from `{AU} --help`; run --update {path}")
            else:
                print(f"{path}: {len(found)} command blocks match `{AU} --help`")
            return
    print(f"{len(cmds)} subcommands")


if __name__ == "__main__":
    main()
