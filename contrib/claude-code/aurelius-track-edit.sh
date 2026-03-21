#!/usr/bin/env bash
# Aurelius hook: touches file nodes on edit to track access frequency.
# Increments access_count on existing File nodes — creates NO new nodes.
# Runs on PostToolUse for Edit/Write tools.
set -euo pipefail

AU="${AU_BIN:-au}"
which "$AU" &>/dev/null || exit 0

# Read the tool input from stdin (Claude Code passes JSON)
INPUT=$(cat)

# Extract file path from the tool input
FILE_PATH=$(echo "$INPUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    ti = d.get('tool_input', {})
    print(ti.get('file_path', ti.get('path', '')))
except:
    pass
" 2>/dev/null || true)

[ -z "$FILE_PATH" ] && exit 0
[ -f "$FILE_PATH" ] || exit 0

# Touch the file node (increment access_count, no new nodes created)
"$AU" touch "$FILE_PATH" &>/dev/null || true
