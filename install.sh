#!/usr/bin/env bash
# Aurelius — one-command install
# Usage: ./install.sh
set -euo pipefail

BOLD='\033[1m'
GOLD='\033[33m'
GREEN='\033[32m'
DIM='\033[2m'
RESET='\033[0m'

echo -e "${GOLD}${BOLD}Aurelius${RESET} — Knowledge Graph Memory for AI Agents"
echo ""

# --- 1. Check prerequisites ---
echo -e "${DIM}Checking prerequisites...${RESET}"

command -v cargo >/dev/null 2>&1 || { echo "Error: cargo not found. Install Rust: https://rustup.rs"; exit 1; }
command -v npm >/dev/null 2>&1 || { echo "Error: npm not found. Install Node.js: https://nodejs.org"; exit 1; }

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

# --- 3. Build UI ---
echo -e "${BOLD}Building UI...${RESET}"
cd ui
npm install --silent 2>&1 | tail -1
npm run build 2>&1 | tail -3
cd "$SCRIPT_DIR"
echo -e "${GREEN}✓${RESET} UI built"

# --- 4. Initialize database ---
echo -e "${BOLD}Initializing database...${RESET}"
"$INSTALL_DIR/au" init 2>/dev/null || true

# --- 5. Install hooks ---
echo -e "${BOLD}Installing Claude Code hooks...${RESET}"
HOOKS_DIR="${HOME}/.claude/hooks"
mkdir -p "$HOOKS_DIR"

/usr/bin/cp -f contrib/claude-code/aurelius-reindex.sh "$HOOKS_DIR/" 2>/dev/null || cp -f contrib/claude-code/aurelius-reindex.sh "$HOOKS_DIR/"
/usr/bin/cp -f contrib/claude-code/aurelius-track-edit.sh "$HOOKS_DIR/" 2>/dev/null || cp -f contrib/claude-code/aurelius-track-edit.sh "$HOOKS_DIR/"
chmod +x "$HOOKS_DIR/aurelius-reindex.sh" "$HOOKS_DIR/aurelius-track-edit.sh"
echo -e "${GREEN}✓${RESET} Hooks installed to ${HOOKS_DIR}"

# --- 6. Configure MCP server ---
SETTINGS_FILE="${HOME}/.claude/settings.json"
if [ -f "$SETTINGS_FILE" ]; then
    if ! grep -q "aurelius" "$SETTINGS_FILE" 2>/dev/null; then
        echo -e "${DIM}  Add MCP server manually: /mcp in Claude Code, command: au, args: [\"mcp\"]${RESET}"
    else
        echo -e "${GREEN}✓${RESET} MCP server already configured"
    fi
else
    echo -e "${DIM}  Claude Code settings not found. Configure MCP after installing Claude Code.${RESET}"
fi

# --- 7. Configure hooks in settings.json ---
if [ -f "$SETTINGS_FILE" ]; then
    if ! grep -q "aurelius-reindex" "$SETTINGS_FILE" 2>/dev/null; then
        echo -e "${DIM}  Add hooks manually in Claude Code settings:${RESET}"
        echo -e "${DIM}    Stop: ${HOOKS_DIR}/aurelius-reindex.sh (timeout: 15)${RESET}"
        echo -e "${DIM}    PostToolUse Edit|Write: ${HOOKS_DIR}/aurelius-track-edit.sh (timeout: 5)${RESET}"
    else
        echo -e "${GREEN}✓${RESET} Hooks already configured in settings"
    fi
fi

# --- 8. Install git hooks ---
if [ -d .git ]; then
    echo -e "${BOLD}Installing git hooks...${RESET}"
    /usr/bin/cp -f contrib/git-hooks/post-commit .git/hooks/post-commit 2>/dev/null || cp -f contrib/git-hooks/post-commit .git/hooks/post-commit
    chmod +x .git/hooks/post-commit
    echo -e "${GREEN}✓${RESET} Git post-commit hook installed"
fi

# --- Done ---
echo ""
echo -e "${GOLD}${BOLD}Aurelius installed!${RESET}"
echo ""
echo "  Commands:"
echo "    au init        — initialize database"
echo "    au note        — add knowledge node"
echo "    au search      — search the graph"
echo "    au context     — explore around a topic"
echo "    au reindex     — index current project"
echo "    au sync        — pull from TimeForged"
echo "    au view        — open graph visualization"
echo "    au mcp         — start MCP server"
echo ""
echo "  Next steps:"
echo "    1. au reindex                — index your project"
echo "    2. au view                   — see your graph"
echo "    3. Configure MCP in Claude Code for AI memory"
