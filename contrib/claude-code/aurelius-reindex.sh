#!/usr/bin/env bash
# Aurelius hook: re-indexes the current project on session Stop.
# Lightweight — only updates changed files (compares content_hash).
# Install: add to Claude Code settings.json Stop hook.
set -euo pipefail

AU="${AU_BIN:-au}"
which "$AU" &>/dev/null || exit 0

# Detect project root
ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)

# Re-index silently (don't block Claude Code)
"$AU" reindex --path "$ROOT" &>/dev/null || true
