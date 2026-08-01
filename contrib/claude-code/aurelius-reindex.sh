#!/usr/bin/env bash
# Aurelius hook: re-indexes the current project on session Stop, then pushes
# any sync-enabled projects (US2 — CLI-level equivalent of the automatic
# push memory_session does over MCP, for non-MCP flows).
# Lightweight — only updates changed files (compares content_hash).
# Install: add to Claude Code settings.json Stop hook.
set -euo pipefail

AU="${AU_BIN:-au}"
which "$AU" &>/dev/null || exit 0

# Detect project root
ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)

# Re-index silently (don't block Claude Code)
"$AU" reindex --path "$ROOT" &>/dev/null || true

# Push every sync-enabled project (best-effort — never blocks Claude Code;
# `au share push` with no project argument already targets all of them and
# warns-and-continues on a per-project failure, per FR-006/FR-011).
"$AU" share push &>/dev/null || true
