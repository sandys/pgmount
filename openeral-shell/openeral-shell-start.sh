#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# openeral-shell-start — Configure and start the persistent shell environment.
# Designed for OpenShell sandboxes.
#
# Usage (OpenShell):
#   openshell sandbox create --from . \
#     --upload .env:/sandbox/.env \
#     --policy openeral-shell/policy.yaml
#
# Usage (Docker Compose):
#   docker compose up -d
#
# Sets up:
#   /db         — read-only PostgreSQL database browsable as files
#   /home/agent — read-write persistent home directory (backed by PostgreSQL)
# =============================================================================

# --- Source .env file if uploaded via --upload .env:/sandbox/.env ---

if [ -f /sandbox/.env ]; then
    set -a
    # shellcheck source=/dev/null
    . /sandbox/.env
    set +a
fi

# --- Resolve database connection ---

DB_URL="${DATABASE_URL:-${OPENERAL_DATABASE_URL:-}}"

if [ -f /run/secrets/database_url ]; then
    DB_URL="$(cat /run/secrets/database_url)"
fi

if [ -z "$DB_URL" ]; then
    echo "ERROR: DATABASE_URL is not set." >&2
    echo "" >&2
    echo "Via OpenShell (from repo root):" >&2
    echo "  openshell sandbox create --from . \\" >&2
    echo "    --upload .env:/sandbox/.env \\" >&2
    echo "    --policy openeral-shell/policy.yaml" >&2
    echo "" >&2
    echo "Via Docker Compose:" >&2
    echo "  Set DATABASE_URL in openeral-shell/.env" >&2
    exit 1
fi

export OPENERAL_DATABASE_URL="$DB_URL"

# --- Workspace configuration ---

WS_ID="${WORKSPACE_ID:-default}"
WS_NAME="${WORKSPACE_NAME:-$WS_ID}"

# Default: broad agent directories covering Claude Code, Codex, Gemini, etc.
DEFAULT_CONFIG='{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions",".claude/tasks",".claude/todos",".claude/skills",".cache",".local",".config",".npm"]}'
WS_CONFIG="${WORKSPACE_CONFIG:-$DEFAULT_CONFIG}"

TIMEOUT="${STARTUP_TIMEOUT:-15}"

# --- Mount database at /db (read-only) ---

openeral mount /db &
DB_PID=$!

# --- Cleanup on exit ---

WS_PID=""
cleanup() {
    fusermount -u /home/agent 2>/dev/null || true
    [ -n "$WS_PID" ] && kill "$WS_PID" 2>/dev/null || true
    fusermount -u /db 2>/dev/null || true
    kill "$DB_PID" 2>/dev/null || true
    wait "$DB_PID" 2>/dev/null || true
    [ -n "$WS_PID" ] && wait "$WS_PID" 2>/dev/null || true
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

# --- Configure Claude Code ---
# Claude Code's config watcher races on FUSE mounts. Use a local directory
# for .claude.json config, while /home/agent remains the persistent workspace.

AGENT_LOCAL="/sandbox/.openeral-config"
mkdir -p "$AGENT_LOCAL"
if [ -f /home/agent/.claude.json ]; then
    cp /home/agent/.claude.json "$AGENT_LOCAL/.claude.json"
else
    echo '{}' > "$AGENT_LOCAL/.claude.json"
fi

export HOME="$AGENT_LOCAL"
cd /home/agent

echo ""
echo "openeral-shell ready."
echo "  Database: /db/"
echo "  Home:     /home/agent/ (persistent)"
echo ""

# --- Hand off (drop privileges if running as root) ---

if [ $# -eq 0 ]; then
    set -- sleep infinity
fi

if [ "$(id -u)" = "0" ] && id sandbox >/dev/null 2>&1; then
    chown sandbox:sandbox "$AGENT_LOCAL" "$AGENT_LOCAL/.claude.json" 2>/dev/null || true
    # Drop to sandbox user. Use su -p to preserve environment.
    # Construct command string safely for su -c.
    CMD=""
    for arg in "$@"; do CMD="$CMD '$(echo "$arg" | sed "s/'/'\\\\''/g")'"; done
    exec su -p -s /bin/bash sandbox -c "cd /home/agent; exec $CMD"
else
    exec "$@"
fi
