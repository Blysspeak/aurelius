#!/usr/bin/env bash
# DEPRECATED since 3.4.0: replaced by the aurelius Claude Code plugin (plugin/hooks.json calls au <cmd> --hook directly).
# Kept for hand installs until the next major release. Do not add features here.
set -euo pipefail

HOOKS_DIR="$HOME/.claude/hooks"
mkdir -p "$HOOKS_DIR"

cp "$(dirname "$0")/aurelius-capture.sh" "$HOOKS_DIR/aurelius-capture.sh"
chmod +x "$HOOKS_DIR/aurelius-capture.sh"

SETTINGS="$HOME/.claude/settings.json"
if [ ! -f "$SETTINGS" ]; then
    echo '{}' > "$SETTINGS"
fi

echo "✓ Aurelius Claude Code hooks installed"
echo "  Add to ~/.claude/settings.json:"
cat <<'EOF'
{
  "mcpServers": {
    "aurelius": {
      "command": "au",
      "args": ["mcp"]
    }
  },
  "hooks": {
    "Stop": [{ "type": "command", "command": "~/.claude/hooks/aurelius-capture.sh" }]
  }
}
EOF
