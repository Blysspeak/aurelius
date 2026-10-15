#!/usr/bin/env python3
"""Dump MCP tool schemas straight from crates/aurelius/src/mcp/tools.rs.

Parses the json! literal in tool_definitions() (no build, no live database),
marks the tools hidden by ADMIN_TOOLS in mcp/mod.rs, and prints either JSON
(--json) or the markdown parameter tables used in docs/mcp-tools.md (--tables).

    python3 scripts/dump-mcp-tools.py --json   > /tmp/tools.json
    python3 scripts/dump-mcp-tools.py --tables > /tmp/tables.md
    python3 scripts/dump-mcp-tools.py --update docs/mcp-tools.md   # refresh tables in place
    python3 scripts/dump-mcp-tools.py --check  docs/mcp-tools.md   # exit 1 when stale
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLS_RS = ROOT / "crates/aurelius/src/mcp/tools.rs"
MOD_RS = ROOT / "crates/aurelius/src/mcp/mod.rs"
MODELS_RS = ROOT / "crates/aurelius-core/src/models.rs"


def known(kind):
    src = MODELS_RS.read_text()
    block = src.split(f"impl {kind}", 1)[1].split("KNOWN", 1)[1].split("];", 1)[0]
    return re.findall(r'"([^"]+)"', block)


def load_tools():
    src = TOOLS_RS.read_text()
    body = src.split("json!(", 1)[1]
    body = body[: body.rindex("})") + 1]
    body = re.sub(r"^\s*//.*$", "", body, flags=re.M)  # Rust line comments
    # The single Rust expression inside the literal.
    body = re.sub(
        r'format!\("Relation type: \{\}", aurelius_core::models::Relation::KNOWN\.join\(", "\)\)',
        json.dumps("Relation type: " + ", ".join(known("Relation"))),
        body,
    )
    body = body.replace("aurelius_core::models::NodeType::KNOWN", json.dumps(known("NodeType")))
    if re.search(r"format!|concat!|::", body.replace("http://", "").replace("https://", "")):
        sys.exit("tools.rs holds a Rust expression this script does not know")
    body = re.sub(r",(\s*[}\]])", r"\1", body)
    return json.loads(body)["tools"]


def admin_tools():
    src = MOD_RS.read_text()
    block = src.split("const ADMIN_TOOLS", 1)[1].split("];", 1)[0]
    return re.findall(r'"([^"]+)"', block)


def type_of(spec):
    t = spec.get("type", "any")
    if isinstance(t, list):
        t = " \\| ".join(t)
    if t == "array" and isinstance(spec.get("items"), dict):
        t = f"array of {spec['items'].get('type', 'object')}"
    if "enum" in spec:
        t += ": " + ", ".join(f"`{v}`" for v in spec["enum"])
    return t


def cell(text):
    return " ".join(str(text).split()).replace("|", "\\|")


def table(tool):
    schema = tool.get("inputSchema", {})
    props = schema.get("properties", {})
    req = set(schema.get("required", []))
    if not props:
        return "No parameters.\n"
    rows = ["| Name | Type | Required | Meaning |", "|---|---|---|---|"]

    def add(prefix, props, req):
        for name, spec in props.items():
            meaning = spec.get("description", "")
            if "default" in spec:
                meaning += f" Default: `{json.dumps(spec['default'], ensure_ascii=False)}`."
            rows.append(
                f"| `{prefix}{name}` | {cell(type_of(spec))} | {'yes' if name in req else 'no'} | {cell(meaning)} |"
            )
            items = spec.get("items")
            if isinstance(items, dict) and items.get("properties"):
                add(f"{prefix}{name}[].", items["properties"], set(items.get("required", [])))

    add("", props, req)
    return "\n".join(rows) + "\n"


BLOCK = re.compile(r"(<!-- params:(\w+) -->\n)(.*?)(<!-- /params -->)", re.S)


def sync_doc(path, tools, write):
    """Rewrite every `<!-- params:NAME -->` block of a doc with the table built
    from the schema. Fails when a tool has no block or a block names no tool."""
    doc = Path(path).read_text()
    by_name = {t["name"]: t for t in tools}
    found = [m.group(2) for m in BLOCK.finditer(doc)]
    missing = sorted(set(by_name) - set(found))
    unknown = sorted(set(found) - set(by_name))
    if missing or unknown:
        sys.exit(f"{path}: no block for {missing}; blocks for unknown tools {unknown}")
    new = BLOCK.sub(lambda m: m.group(1) + table(by_name[m.group(2)]) + m.group(4), doc)
    if write:
        Path(path).write_text(new)
        print(f"{path}: {len(found)} parameter tables written")
    elif new != doc:
        sys.exit(f"{path}: parameter tables differ from tools.rs; run --update {path}")
    else:
        print(f"{path}: {len(found)} parameter tables match tools.rs")


def main():
    tools = load_tools()
    admin = set(admin_tools())
    for flag in ("--update", "--check"):
        if flag in sys.argv:
            sync_doc(sys.argv[sys.argv.index(flag) + 1], tools, flag == "--update")
            return
    if "--json" in sys.argv:
        for t in tools:
            t["admin"] = t["name"] in admin
        json.dump(tools, sys.stdout, ensure_ascii=False, indent=2)
        return
    if "--tables" in sys.argv:
        for t in tools:
            print(f"### {t['name']}{' (admin)' if t['name'] in admin else ''}\n")
            print(table(t))
        return
    print(f"{len(tools)} tools, {len(admin)} admin, {len(tools) - len(admin)} listed by default")


if __name__ == "__main__":
    main()
