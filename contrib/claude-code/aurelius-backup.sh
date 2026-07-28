#!/usr/bin/env bash
# Aurelius hook: rolling snapshot of the knowledge graph on session start.
#
# Snapshots are taken with `au db backup`, which uses SQLite's VACUUM INTO —
# the only safe way to copy a live database. Copying aurelius.db with
# cp/mv/rsync while `au` or an MCP server is running is what corrupts it.
#
# Cadence follows activity rather than the clock: the graph only changes when
# Claude Code, the CLI or the git hooks touch it, so a session start is exactly
# when a fresh snapshot is worth taking — and a machine left idle for a week
# does not accumulate seven identical copies. Several sessions in one day cost
# one snapshot (see AURELIUS_BACKUP_MIN_HOURS).
#
# Measured cost: ~50 ms for an 8 MB graph.
#
# Install: add to Claude Code settings.json SessionStart hook.
#
# Environment:
#   AU_BIN                     path to the au binary (default: au on PATH)
#   AURELIUS_BACKUP_KEEP       snapshots to retain (default: 7)
#   AURELIUS_BACKUP_MIN_HOURS  minimum age of the newest snapshot before a new
#                              one is taken (default: 24)
set -uo pipefail

AU="${AU_BIN:-au}"
command -v "$AU" >/dev/null 2>&1 || exit 0

KEEP="${AURELIUS_BACKUP_KEEP:-7}"
MIN_HOURS="${AURELIUS_BACKUP_MIN_HOURS:-24}"

# Resolve the database the same way au does: data_dir()/aurelius
if [ -n "${APPDATA:-}" ]; then
    DB_DIR="$APPDATA/aurelius"
else
    DB_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/aurelius"
fi
[ -f "$DB_DIR/aurelius.db" ] || exit 0

BACKUP_DIR="$DB_DIR/backups"
mkdir -p "$BACKUP_DIR" 2>/dev/null || exit 0

mtime() {
    date -r "$1" +%s 2>/dev/null || stat -c %Y "$1" 2>/dev/null || echo 0
}

# Throttle: skip while the newest snapshot is younger than MIN_HOURS.
newest=$(ls -1t "$BACKUP_DIR"/aurelius-*.db 2>/dev/null | head -1)
if [ -n "$newest" ]; then
    age_hours=$(( ( $(date +%s) - $(mtime "$newest") ) / 3600 ))
    [ "$age_hours" -lt "$MIN_HOURS" ] && exit 0
fi

dest="$BACKUP_DIR/aurelius-$(date -u +%Y%m%dT%H%M%SZ).db"

# A failed snapshot must never block the session. `au db backup` refuses a
# damaged source, so a file appearing here means the source was readable.
"$AU" db backup --out "$dest" >/dev/null 2>&1 || exit 0

# Verify the snapshot itself. An unverified backup is a guess; this is the whole
# reason `au db check` takes a path. A snapshot that fails is renamed out of the
# `aurelius-*.db` pattern so it can never be mistaken for a good backup, and is
# kept rather than deleted — a bad snapshot is evidence worth looking at.
if ! "$AU" db check "$dest" >/dev/null 2>&1; then
    mv -f "$dest" "$dest.FAILED-CHECK" 2>/dev/null
    # Opening a WAL database read-only leaves empty -wal/-shm siblings behind.
    # A healthy snapshot never has them (VACUUM INTO writes a rollback-journal
    # database), so this only fires on the damaged path.
    rm -f "$dest-wal" "$dest-shm" 2>/dev/null
    exit 0
fi

# Retention: keep the newest KEEP snapshots, drop the rest.
ls -1t "$BACKUP_DIR"/aurelius-*.db 2>/dev/null | tail -n "+$((KEEP + 1))" | while IFS= read -r old; do
    rm -f "$old"
done

exit 0
