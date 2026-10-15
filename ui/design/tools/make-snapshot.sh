#!/usr/bin/env bash
# Rebuild ui/design/snapshot.js from the local aurelius database.
# snapshot.js holds personal finance entries and memory labels, so it is git-ignored:
# the repository is public. Needs `au view` running on :9872 (or set AU_VIEW).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out=/tmp/aud
view="${AU_VIEW:-http://localhost:9872}"
mkdir -p "$out"
curl -sf "$view/api/graph" -o "$out/graph.json"
au fin stats --period all --json > "$out/stats_cat.json"
au fin stats --period all --by month --json > "$out/stats_month.json"
au fin list --period all --all --limit 100000 --json > "$out/listall.json"
au task list --project aurelius --limit 40 --json > "$out/tasks.json"
gh pr list -R Blysspeak/aurelius --state all --limit 50 --json number,state,headRefName > "$out/prs.json"
python3 -I "$here/build_snapshot.py" "$here/../snapshot.js"
