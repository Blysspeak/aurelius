#!/usr/bin/env bash
set -euo pipefail

GIT_DIR=$(git rev-parse --git-dir 2>/dev/null || echo "")
if [ -z "$GIT_DIR" ]; then
    echo "Not inside a git repository."
    exit 1
fi

cp "$(dirname "$0")/post-commit" "$GIT_DIR/hooks/post-commit"
chmod +x "$GIT_DIR/hooks/post-commit"
echo "✓ Aurelius git hook installed for this repository"
