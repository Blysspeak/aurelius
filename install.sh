#!/usr/bin/env bash
# Aurelius v1.0.0 — one-command install
# Usage: ./install.sh
set -euo pipefail

BOLD='\033[1m'
GOLD='\033[33m'
GREEN='\033[32m'
RED='\033[31m'
DIM='\033[2m'
RESET='\033[0m'

echo -e "${GOLD}${BOLD}Aurelius v1.0.0${RESET} — Knowledge Graph Memory for AI Agents"
echo ""

# --- 1. Check prerequisites ---
echo -e "${DIM}Checking prerequisites...${RESET}"
command -v cargo >/dev/null 2>&1 || { echo -e "${RED}Error:${RESET} cargo not found. Install Rust: https://rustup.rs"; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# --- 2. Build Rust binaries ---
echo -e "${BOLD}Building binaries...${RESET}"
cargo build --release 2>&1 | tail -3

INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"

/usr/bin/cp -f target/release/au "$INSTALL_DIR/au" 2>/dev/null || cp -f target/release/au "$INSTALL_DIR/au"
/usr/bin/cp -f target/release/aurelius "$INSTALL_DIR/aurelius" 2>/dev/null || cp -f target/release/aurelius "$INSTALL_DIR/aurelius"
echo -e "${GREEN}✓${RESET} Installed au and aurelius to ${INSTALL_DIR}"

# Check PATH
if ! echo "$PATH" | tr ':' '\n' | grep -q "$INSTALL_DIR"; then
    echo -e "  ${GOLD}Warning:${RESET} ${INSTALL_DIR} is not in PATH"
    echo "  Add to your shell config: export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# --- 3. Build UI (optional) ---
if command -v npm >/dev/null 2>&1; then
    echo -e "${BOLD}Building UI...${RESET}"
    cd ui
    npm install --silent 2>&1 | tail -1
    npm run build 2>&1 | tail -3
    cd "$SCRIPT_DIR"
    echo -e "${GREEN}✓${RESET} UI built"
else
    echo -e "${DIM}npm not found — skipping UI build (optional)${RESET}"
fi

# --- 4. Initialize database ---
echo -e "${BOLD}Initializing database...${RESET}"
"$INSTALL_DIR/au" init 2>/dev/null || true
echo -e "${GREEN}✓${RESET} Database ready"

# --- 5. Install Claude Code hooks ---
echo -e "${BOLD}Installing Claude Code hooks...${RESET}"
HOOKS_DIR="${HOME}/.claude/hooks"
mkdir -p "$HOOKS_DIR"

/usr/bin/cp -f contrib/claude-code/aurelius-reindex.sh "$HOOKS_DIR/" 2>/dev/null || cp -f contrib/claude-code/aurelius-reindex.sh "$HOOKS_DIR/"
/usr/bin/cp -f contrib/claude-code/aurelius-track-edit.sh "$HOOKS_DIR/" 2>/dev/null || cp -f contrib/claude-code/aurelius-track-edit.sh "$HOOKS_DIR/"
chmod +x "$HOOKS_DIR/aurelius-reindex.sh" "$HOOKS_DIR/aurelius-track-edit.sh"
echo -e "${GREEN}✓${RESET} Hooks installed to ${HOOKS_DIR}"

# --- 6. Auto-configure Claude Code settings ---
SETTINGS_FILE="${HOME}/.claude/settings.json"
mkdir -p "${HOME}/.claude"

configure_settings() {
    local tmp
    tmp=$(mktemp)

    if [ ! -f "$SETTINGS_FILE" ]; then
        echo '{}' > "$SETTINGS_FILE"
    fi

    python3 -c "
import json, sys

with open('$SETTINGS_FILE') as f:
    settings = json.load(f)

# MCP server
mcp = settings.setdefault('mcpServers', {})
if 'aurelius' not in mcp:
    mcp['aurelius'] = {'command': 'au', 'args': ['mcp']}
    print('  Added MCP server: aurelius', file=sys.stderr)
else:
    print('  MCP server already configured', file=sys.stderr)

# Hooks
hooks = settings.setdefault('hooks', {})

# Stop hook — reindex on session end
stop_hooks = hooks.setdefault('Stop', [])
reindex_cmd = '$HOOKS_DIR/aurelius-reindex.sh'
if not any(h.get('command', '') == reindex_cmd for h in stop_hooks):
    stop_hooks.append({'type': 'command', 'command': reindex_cmd, 'timeout': 15000})
    print('  Added Stop hook: aurelius-reindex', file=sys.stderr)

# PostToolUse hook — track file edits
post_hooks = hooks.setdefault('PostToolUse', [])
track_cmd = '$HOOKS_DIR/aurelius-track-edit.sh'
if not any(h.get('command', '') == track_cmd for h in post_hooks):
    post_hooks.append({
        'type': 'command',
        'command': track_cmd,
        'timeout': 5000,
        'matcher': {'toolName': ['Edit', 'Write']}
    })
    print('  Added PostToolUse hook: aurelius-track-edit', file=sys.stderr)

with open('$tmp', 'w') as f:
    json.dump(settings, f, indent=2)
    f.write('\n')
" 2>&1

    if [ -s "$tmp" ]; then
        mv "$tmp" "$SETTINGS_FILE"
        echo -e "${GREEN}✓${RESET} Claude Code settings configured"
    else
        rm -f "$tmp"
        echo -e "${DIM}  Could not auto-configure settings. Configure manually.${RESET}"
    fi
}

if command -v python3 >/dev/null 2>&1; then
    configure_settings
else
    echo -e "${DIM}  python3 not found — configure MCP and hooks manually${RESET}"
    echo -e "${DIM}  MCP: /mcp in Claude Code → command: au, args: [\"mcp\"]${RESET}"
fi

# --- 7. Install git hooks (for current repo) ---
if [ -d .git ]; then
    echo -e "${BOLD}Installing git hooks...${RESET}"
    /usr/bin/cp -f contrib/git-hooks/post-commit .git/hooks/post-commit 2>/dev/null || cp -f contrib/git-hooks/post-commit .git/hooks/post-commit
    chmod +x .git/hooks/post-commit
    echo -e "${GREEN}✓${RESET} Git post-commit hook installed"
fi

# --- 8. Index current project ---
echo -e "${BOLD}Indexing project...${RESET}"
"$INSTALL_DIR/au" reindex --path "$SCRIPT_DIR" 2>/dev/null || true
echo -e "${GREEN}✓${RESET} Project indexed"

# --- Done ---
echo ""
echo -e "${GOLD}${BOLD}Aurelius v1.0.0 installed!${RESET}"
echo ""
echo "  12 MCP tools ready for Claude Code."
echo "  Database: ~/.local/share/aurelius/aurelius.db"
echo ""
echo "  Commands:"
echo "    au view        — open graph visualization"
echo "    au search      — search the knowledge graph"
echo "    au mcp         — start MCP server (auto-configured)"
echo ""
echo "  To install git hooks in other repos:"
echo "    cp contrib/git-hooks/post-commit /path/to/repo/.git/hooks/"
echo ""
echo -e "  ${DIM}Restart Claude Code to activate MCP server.${RESET}"
