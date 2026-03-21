#!/usr/bin/env bash
# Aurelius v1.2.0 — one-command install
# Usage: ./install.sh
set -euo pipefail

BOLD='\033[1m'
GOLD='\033[33m'
GREEN='\033[32m'
RED='\033[31m'
DIM='\033[2m'
RESET='\033[0m'

echo ""
echo -e "${GOLD}${BOLD}"
cat << 'BANNER'
   █████╗ ██╗   ██╗██████╗ ███████╗██╗     ██╗██╗   ██╗███████╗
  ██╔══██╗██║   ██║██╔══██╗██╔════╝██║     ██║██║   ██║██╔════╝
  ███████║██║   ██║██████╔╝█████╗  ██║     ██║██║   ██║███████╗
  ██╔══██║██║   ██║██╔══██╗██╔══╝  ██║     ██║██║   ██║╚════██║
  ██║  ██║╚██████╔╝██║  ██║███████╗███████╗██║╚██████╔╝███████║
  ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚══════╝╚══════╝╚═╝ ╚═════╝ ╚══════╝
BANNER
echo -e "${RESET}"
echo -e "  ${BOLD}v1.2.0${RESET} ${DIM}— Knowledge Graph Memory for AI Agents${RESET}"
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

# --- 5. Configure Brave Search API ---
BRAVE_KEY_DIR="${HOME}/.config/aurelius"
BRAVE_KEY_FILE="${BRAVE_KEY_DIR}/brave.key"
mkdir -p "$BRAVE_KEY_DIR"

if [ -f "$BRAVE_KEY_FILE" ] && [ -s "$BRAVE_KEY_FILE" ]; then
    echo -e "${GREEN}✓${RESET} Brave Search API key already configured"
else
    echo ""
    echo -e "${BOLD}Brave Search API ${DIM}(optional — enables search_web tool)${RESET}"
    echo -e "  Free: 2000 queries/month at ${DIM}https://brave.com/search/api/${RESET}"
    echo ""
    read -rp "  Brave API key (Enter to skip): " BRAVE_KEY
    if [ -n "$BRAVE_KEY" ]; then
        echo "$BRAVE_KEY" > "$BRAVE_KEY_FILE"
        chmod 600 "$BRAVE_KEY_FILE"
        echo -e "${GREEN}✓${RESET} Brave API key saved to ${BRAVE_KEY_FILE}"
    else
        echo -e "${DIM}  Skipped — search_web will be unavailable until key is added${RESET}"
        echo -e "${DIM}  Add later: echo 'YOUR_KEY' > ${BRAVE_KEY_FILE}${RESET}"
    fi
fi
echo ""

# --- 6. Install Claude Code hooks ---
echo -e "${BOLD}Installing Claude Code hooks...${RESET}"
HOOKS_DIR="${HOME}/.claude/hooks"
mkdir -p "$HOOKS_DIR"

/usr/bin/cp -f contrib/claude-code/aurelius-reindex.sh "$HOOKS_DIR/" 2>/dev/null || cp -f contrib/claude-code/aurelius-reindex.sh "$HOOKS_DIR/"
/usr/bin/cp -f contrib/claude-code/aurelius-track-edit.sh "$HOOKS_DIR/" 2>/dev/null || cp -f contrib/claude-code/aurelius-track-edit.sh "$HOOKS_DIR/"
chmod +x "$HOOKS_DIR/aurelius-reindex.sh" "$HOOKS_DIR/aurelius-track-edit.sh"
echo -e "${GREEN}✓${RESET} Hooks installed to ${HOOKS_DIR}"

# --- 7. Auto-configure Claude Code settings ---
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

# --- Hooks ---
# Claude Code hook format: [{matcher: 'string', hooks: [{type, command, timeout}]}]
hooks = settings.setdefault('hooks', {})
reindex_cmd = '$HOOKS_DIR/aurelius-reindex.sh'
track_cmd = '$HOOKS_DIR/aurelius-track-edit.sh'

def has_hook_cmd(hook_list, cmd):
    \"\"\"Check if command already exists anywhere in the hook entries.\"\"\"
    for entry in hook_list:
        # New format: {matcher, hooks: [...]}
        for h in entry.get('hooks', []):
            if h.get('command', '') == cmd:
                return True
        # Bare format (shouldn't be used, but check anyway)
        if entry.get('command', '') == cmd:
            return True
    return True if not hook_list else False  # skip if empty, we handle below

def add_hook_to_group(hook_list, matcher, cmd, timeout):
    \"\"\"Add a hook command to the correct matcher group, or create one.\"\"\"
    # Check if command already exists in any entry
    for entry in hook_list:
        for h in entry.get('hooks', []):
            if h.get('command', '') == cmd:
                return False  # already exists
    # Find existing entry with matching matcher
    for entry in hook_list:
        if entry.get('matcher', '') == matcher:
            entry.setdefault('hooks', []).append({
                'type': 'command', 'command': cmd, 'timeout': timeout
            })
            return True
    # No matching group — create new entry
    hook_list.append({
        'matcher': matcher,
        'hooks': [{'type': 'command', 'command': cmd, 'timeout': timeout}]
    })
    return True

# Stop hook — reindex on session end (matcher: '' = match all)
stop_hooks = hooks.setdefault('Stop', [])
if add_hook_to_group(stop_hooks, '', reindex_cmd, 15):
    print('  Added Stop hook: aurelius-reindex', file=sys.stderr)
else:
    print('  Stop hook already configured', file=sys.stderr)

# PostToolUse hook — track file edits (matcher: 'Edit|Write')
post_hooks = hooks.setdefault('PostToolUse', [])
if add_hook_to_group(post_hooks, 'Edit|Write', track_cmd, 5):
    print('  Added PostToolUse hook: aurelius-track-edit', file=sys.stderr)
else:
    print('  PostToolUse hook already configured', file=sys.stderr)

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

# --- 8. Install git hooks (for current repo) ---
if [ -d .git ]; then
    echo -e "${BOLD}Installing git hooks...${RESET}"
    /usr/bin/cp -f contrib/git-hooks/post-commit .git/hooks/post-commit 2>/dev/null || cp -f contrib/git-hooks/post-commit .git/hooks/post-commit
    chmod +x .git/hooks/post-commit
    echo -e "${GREEN}✓${RESET} Git post-commit hook installed"
fi

# --- 9. Index current project ---
echo -e "${BOLD}Indexing project...${RESET}"
"$INSTALL_DIR/au" reindex --path "$SCRIPT_DIR" 2>/dev/null || true
echo -e "${GREEN}✓${RESET} Project indexed"

# --- Done ---
echo ""
echo -e "${GOLD}${BOLD}Aurelius v1.2.0 installed!${RESET}"
echo ""
echo "  14 MCP tools ready for Claude Code."
echo "  Database: ~/.local/share/aurelius/aurelius.db"
if [ -f "$BRAVE_KEY_FILE" ] && [ -s "$BRAVE_KEY_FILE" ]; then
echo "  Brave Search: configured (2 search tools active)"
else
echo -e "  Brave Search: ${DIM}not configured (add key to enable)${RESET}"
fi
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
