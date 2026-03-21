#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# openeral-shell entrypoint
#
# Sets up a persistent shell environment with:
#   /db         — read-only PostgreSQL database browsable as files
#   /home/agent — read-write persistent home directory (backed by PostgreSQL)
#
# openeral is invisible infrastructure — users interact with a normal shell.
# =============================================================================

# --- Resolve database connection ---

DB_URL="${DATABASE_URL:-${OPENERAL_DATABASE_URL:-}}"

if [ -f /run/secrets/database_url ]; then
    DB_URL="$(cat /run/secrets/database_url)"
fi

if [ -z "$DB_URL" ]; then
    echo "ERROR: DATABASE_URL is not set." >&2
    echo "" >&2
    echo "Option 1: Set it in docker-compose.yml under environment:" >&2
    echo "  DATABASE_URL: \"host=your-host user=your-user password=your-pass dbname=your-db\"" >&2
    echo "" >&2
    echo "Option 2: Pass it directly:" >&2
    echo "  docker compose exec -e DATABASE_URL='postgres://user:pass@host/db' openeral-shell bash" >&2
    exit 1
fi

export OPENERAL_DATABASE_URL="$DB_URL"

# --- Workspace configuration ---

WS_ID="${WORKSPACE_ID:-default}"
WS_NAME="${WORKSPACE_NAME:-$WS_ID}"

# Default: broad agent directories covering Claude Code, Codex, Gemini, etc.
DEFAULT_CONFIG='{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions",".claude/tasks",".claude/todos",".claude/skills",".cache",".local",".config",".npm"]}'
WS_CONFIG="${WORKSPACE_CONFIG:-$DEFAULT_CONFIG}"

export HOME=/home/agent
TIMEOUT="${STARTUP_TIMEOUT:-15}"

# --- Mount database at /db (read-only) ---

openeral mount /db &
DB_PID=$!

# --- Cleanup on exit ---

cleanup() {
    fusermount -u /home/agent 2>/dev/null || true
    kill "$WS_PID" 2>/dev/null || true
    fusermount -u /db 2>/dev/null || true
    kill "$DB_PID" 2>/dev/null || true
    wait "$DB_PID" 2>/dev/null || true
    wait "$WS_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- Wait for database mount ---

elapsed=0
while ! mountpoint -q /db 2>/dev/null; do
    if ! kill -0 "$DB_PID" 2>/dev/null; then
        echo "ERROR: Failed to mount database at /db." >&2
        echo "Check DATABASE_URL and ensure PostgreSQL is reachable." >&2
        wait "$DB_PID" 2>/dev/null
        exit 1
    fi
    if [ "$elapsed" -ge "$TIMEOUT" ]; then
        echo "ERROR: Database mount did not become ready within ${TIMEOUT}s." >&2
        kill "$DB_PID" 2>/dev/null || true
        exit 1
    fi
    sleep 1
    elapsed=$((elapsed + 1))
done

echo "Database mounted at /db"

# --- Create and mount workspace (persistent home) ---

openeral workspace create "$WS_ID" \
    --display-name "$WS_NAME" \
    --config "$WS_CONFIG" \
    --skip-migrations 2>/dev/null || true

openeral workspace mount "$WS_ID" /home/agent --skip-migrations &
WS_PID=$!

ws_elapsed=0
while ! mountpoint -q /home/agent 2>/dev/null; do
    if ! kill -0 "$WS_PID" 2>/dev/null; then
        echo "ERROR: Failed to mount workspace at /home/agent." >&2
        exit 1
    fi
    if [ "$ws_elapsed" -ge "$TIMEOUT" ]; then
        echo "ERROR: Workspace mount did not become ready within ${TIMEOUT}s." >&2
        kill "$WS_PID" 2>/dev/null || true
        exit 1
    fi
    sleep 1
    ws_elapsed=$((ws_elapsed + 1))
done

echo "Workspace mounted at /home/agent (id=$WS_ID)"

# --- Install skills into workspace ---

if [ -d /etc/openeral/skills ] && [ -d /home/agent/.claude/skills ]; then
    cp -rn /etc/openeral/skills/* /home/agent/.claude/skills/ 2>/dev/null || true
fi

# --- Hand off to user command ---

exec "$@"
