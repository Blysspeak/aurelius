#!/usr/bin/env bash
# Captures key terms and decisions from Claude Code sessions into Aurelius.
# Runs on session Stop hook.
set -euo pipefail

which au &>/dev/null || exit 0

PROJECT=$(git rev-parse --show-toplevel 2>/dev/null | xargs basename 2>/dev/null || echo "unknown")
BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")

# Heartbeat: update project node
au note "Active session: $PROJECT ($BRANCH)" --type concept --label "$PROJECT" 2>/dev/null || true
