#!/usr/bin/env bash
# Aurelius hook: injects the skill index into context on SessionStart.
# Outputs Claude Code hook JSON (hookSpecificOutput.additionalContext) so the
# agent always knows which reusable skill cards exist — then it can call
# skill_get <name> on demand (progressive disclosure).
set -euo pipefail

AU="${AU_BIN:-au}"
which "$AU" &>/dev/null || exit 0

"$AU" skills --hook 2>/dev/null || true
