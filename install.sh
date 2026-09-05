#!/usr/bin/env bash
# Aurelius — one-command install
# Usage: ./install.sh [--migrate-only]
set -euo pipefail

BOLD='\033[1m'
GOLD='\033[33m'
GREEN='\033[32m'
RED='\033[31m'
DIM='\033[2m'
RESET='\033[0m'

# Removes hook and MCP-server entries that older versions of this script wrote
# directly into ~/.claude/settings.json and ~/.claude.json, now that the
# aurelius Claude Code plugin (plugin/.claude-plugin/plugin.json, plugin/hooks.json)
# owns them. A function rather than inline code so `--migrate-only` and the
# normal install path share it, and so it can be pointed at throwaway copies
# via CLAUDE_HOME/CLAUDE_JSON instead of the real files (used by tests).
migrate_legacy() {
    local claude_home="${CLAUDE_HOME:-$HOME/.claude}"
    local claude_json="${CLAUDE_JSON:-$HOME/.claude.json}"

    if ! command -v python3 >/dev/null 2>&1; then
        echo -e "${GOLD}Warning:${RESET} python3 not found — skipping legacy Claude Code entry migration."
        echo "  Remove aurelius-*.sh hooks and mcpServers.aurelius by hand; see specs/009-claude-code-plugin/quickstart.md"
        return 0
    fi

    CLAUDE_HOME="$claude_home" CLAUDE_JSON="$claude_json" python3 - <<'PYEOF'
import json, os, re, shutil
from datetime import datetime, timezone

claude_home = os.environ.get("CLAUDE_HOME") or os.path.join(os.path.expanduser("~"), ".claude")
claude_json = os.environ.get("CLAUDE_JSON") or os.path.join(os.path.expanduser("~"), ".claude.json")

HOOK_RE = re.compile(r"aurelius-(reindex|track-edit|skills|backup|capture)\.sh")
AU_HOOK_RE = re.compile(r"^au\b.*--hook")


def is_legacy(cmd):
    return bool(HOOK_RE.search(cmd) or AU_HOOK_RE.match(cmd))


def backup(path):
    ts = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    shutil.copy2(path, f"{path}.bak-{ts}")


def mcp_value(entry):
    if isinstance(entry, dict) and "command" in entry:
        return entry["command"]
    return json.dumps(entry)


removed = []

settings_path = os.path.join(claude_home, "settings.json")
if os.path.isfile(settings_path):
    with open(settings_path) as f:
        settings = json.load(f)
    changed = False

    hooks = settings.get("hooks", {})
    for event in list(hooks.keys()):
        new_groups = []
        for group in hooks[event]:
            entries = group.get("hooks", [])
            for h in entries:
                if is_legacy(h.get("command", "")):
                    removed.append(
                        'removed: settings.json hooks.{} "{}" -> {} (moved into the aurelius plugin)'.format(
                            event, group.get("matcher", ""), h.get("command", "")
                        )
                    )
                    changed = True
            kept = [h for h in entries if not is_legacy(h.get("command", ""))]
            if kept:
                group["hooks"] = kept
                new_groups.append(group)
        if new_groups:
            hooks[event] = new_groups
        else:
            del hooks[event]

    mcp = settings.get("mcpServers", {})
    if "aurelius" in mcp:
        removed.append(
            "removed: settings.json mcpServers.aurelius -> {} (the plugin registers the server now)".format(
                mcp_value(mcp["aurelius"])
            )
        )
        del mcp["aurelius"]
        changed = True
    if "mcpServers" in settings and not settings["mcpServers"]:
        del settings["mcpServers"]

    if changed:
        backup(settings_path)
        with open(settings_path, "w") as f:
            json.dump(settings, f, indent=2)
            f.write("\n")

if os.path.isfile(claude_json):
    with open(claude_json) as f:
        data = json.load(f)
    mcp = data.get("mcpServers", {})
    if "aurelius" in mcp:
        removed.append(
            "removed: ~/.claude.json mcpServers.aurelius -> {} (the plugin registers the server now)".format(
                mcp_value(mcp["aurelius"])
            )
        )
        del mcp["aurelius"]
        if not data.get("mcpServers"):
            data.pop("mcpServers", None)
        backup(claude_json)
        with open(claude_json, "w") as f:
            json.dump(data, f, indent=2)
            f.write("\n")

if removed:
    for line in removed:
        print(line)
    print("~/.claude/hooks/aurelius-*.sh and ~/.local/share/mcp/aurelius are no longer used and may be deleted by hand.")
else:
    print("nothing to migrate: no legacy aurelius entries found")
PYEOF
}

MIGRATE_ONLY=0
if [ "${1:-}" = "--migrate-only" ]; then
    MIGRATE_ONLY=1
fi

if [ "$MIGRATE_ONLY" = "1" ]; then
    migrate_legacy
    exit 0
fi

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
# Read from the manifest rather than hardcoding: a version literal in a banner
# rots silently, and an installer that lies about the version is worse than one
# that says nothing.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$(dirname "$0")/Cargo.toml" | head -1)"
echo -e "  ${BOLD}v${VERSION:-?}${RESET} ${DIM}— Knowledge Graph Memory for AI Agents${RESET}"
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

# Replace each binary through a temp file plus `mv`, never `cp` in place:
# `cp` overwriting a binary that a running MCP server holds open fails with
# ETXTBSY (text file busy). `mv` instead replaces the directory entry, so a
# process that already opened the old file keeps running against the old
# inode until it exits — no crash, no interrupted write.
install_binary() {
    local src="$1" name="$2"
    local dest="$INSTALL_DIR/$name"
    if [ -f "$dest" ]; then
        local ts
        ts="$(date -u +%Y%m%dT%H%M%SZ)"
        /usr/bin/cp -f "$dest" "$dest.bak-$ts" 2>/dev/null || cp -f "$dest" "$dest.bak-$ts"
    fi
    /usr/bin/cp -f "$src" "$dest.new" 2>/dev/null || cp -f "$src" "$dest.new"
    mv -f "$dest.new" "$dest"
}

install_binary target/release/au au
install_binary target/release/aurelius aurelius
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

# --- 6. Install Claude Code plugin ---
echo -e "${BOLD}Installing Claude Code plugin...${RESET}"
if command -v claude >/dev/null 2>&1; then
    MARKETPLACE_LOG="$(mktemp)"
    if ! claude plugin marketplace add "$SCRIPT_DIR" >"$MARKETPLACE_LOG" 2>&1; then
        echo -e "${DIM}  Marketplace already registered — updating instead${RESET}"
        claude plugin marketplace update blysspeak || cat "$MARKETPLACE_LOG"
    fi
    rm -f "$MARKETPLACE_LOG"

    INSTALL_LOG="$(mktemp)"
    if ! claude plugin install aurelius@blysspeak -s user -y >"$INSTALL_LOG" 2>&1; then
        if grep -qi "already installed" "$INSTALL_LOG"; then
            echo -e "${DIM}  Plugin already installed — updating instead${RESET}"
            if claude plugin --help 2>&1 | grep -q '  update '; then
                claude plugin update aurelius
            else
                claude plugin install aurelius@blysspeak -s user -y
            fi
        else
            cat "$INSTALL_LOG"
        fi
    fi
    rm -f "$INSTALL_LOG"

    claude plugin list || true
    echo -e "${GREEN}✓${RESET} Claude Code plugin ready"
else
    echo -e "${GOLD}Warning:${RESET} claude CLI not found on PATH — install the plugin by hand:"
    echo "    claude plugin marketplace add \"$SCRIPT_DIR\""
    echo "    claude plugin install aurelius@blysspeak -s user"
fi
echo ""

# --- 7. Migrate legacy Claude Code entries ---
echo -e "${BOLD}Migrating legacy Claude Code entries...${RESET}"
migrate_legacy
echo ""

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
echo -e "${GOLD}${BOLD}Aurelius v${VERSION:-?} installed!${RESET}"
echo ""
echo "  MCP tools ready for Claude Code."
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
echo "    au snapshot    — seven-layer memory slice (--json for programs)"
echo "    au mcp         — start MCP server (auto-configured)"
echo ""
echo "  To install git hooks in other repos:"
echo "    cp contrib/git-hooks/post-commit /path/to/repo/.git/hooks/"
echo ""
echo -e "  ${DIM}Restart Claude Code to activate MCP server.${RESET}"
