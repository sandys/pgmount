#!/usr/bin/env bash
set -euo pipefail

# --- Resolve database connection string ---

# Prefer file-based secret (Docker secrets / K8s volume mount)
if [ -f /run/secrets/openeral_database_url ]; then
    export OPENERAL_DATABASE_URL
    OPENERAL_DATABASE_URL="$(cat /run/secrets/openeral_database_url)"
fi

if [ -z "${OPENERAL_DATABASE_URL:-}" ]; then
    echo "ERROR: OPENERAL_DATABASE_URL is not set." >&2
    echo "Provide it via environment variable or mount a secret at /run/secrets/openeral_database_url" >&2
    exit 1
fi

# --- Build openeral flags ---

OPENERAL_FLAGS=()

if [ -n "${OPENERAL_SCHEMAS:-}" ]; then
    OPENERAL_FLAGS+=(--schemas "$OPENERAL_SCHEMAS")
fi

if [ -n "${OPENERAL_PAGE_SIZE:-}" ]; then
    OPENERAL_FLAGS+=(--page-size "$OPENERAL_PAGE_SIZE")
fi

if [ -n "${OPENERAL_CACHE_TTL:-}" ]; then
    OPENERAL_FLAGS+=(--cache-ttl "$OPENERAL_CACHE_TTL")
fi

if [ -n "${OPENERAL_STATEMENT_TIMEOUT:-}" ]; then
    OPENERAL_FLAGS+=(--statement-timeout "$OPENERAL_STATEMENT_TIMEOUT")
fi

MOUNT_POINT="/db"
TIMEOUT="${OPENERAL_TIMEOUT:-15}"

# --- Start openeral in background ---

openeral mount "${OPENERAL_FLAGS[@]}" "$MOUNT_POINT" &
OPENERAL_PID=$!

# --- Cleanup on exit ---

cleanup() {
    fusermount -u "$MOUNT_POINT" 2>/dev/null || true
    kill "$OPENERAL_PID" 2>/dev/null || true
    wait "$OPENERAL_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- Wait for mount to be ready ---

elapsed=0
while ! mountpoint -q "$MOUNT_POINT" 2>/dev/null; do
    if ! kill -0 "$OPENERAL_PID" 2>/dev/null; then
        echo "ERROR: openeral process exited before mount was ready." >&2
        echo "Check OPENERAL_DATABASE_URL and ensure PostgreSQL is reachable." >&2
        wait "$OPENERAL_PID" 2>/dev/null
        exit 1
    fi

    if [ "$elapsed" -ge "$TIMEOUT" ]; then
        echo "ERROR: openeral did not mount $MOUNT_POINT within ${TIMEOUT}s." >&2
        echo "Check connection string, network access, and PostgreSQL status." >&2
        kill "$OPENERAL_PID" 2>/dev/null || true
        exit 1
    fi

    sleep 1
    elapsed=$((elapsed + 1))
done

echo "openeral mounted at $MOUNT_POINT (PID $OPENERAL_PID)"

# --- Optionally mount workspace ---

if [ -n "${OPENERAL_WORKSPACE_ID:-}" ]; then
    WS_MOUNT="${OPENERAL_WORKSPACE_MOUNT:-/home/agent}"
    WS_NAME="${OPENERAL_WORKSPACE_NAME:-$OPENERAL_WORKSPACE_ID}"
    WS_CONFIG="${OPENERAL_WORKSPACE_CONFIG:-{}}"

    # Create workspace if it doesn't exist (idempotent — will fail silently if exists)
    openeral workspace create \
        --display-name "$WS_NAME" \
        --config "$WS_CONFIG" \
        "$OPENERAL_WORKSPACE_ID" 2>/dev/null || true

    # Mount workspace
    openeral workspace mount "$OPENERAL_WORKSPACE_ID" "$WS_MOUNT" &
    WS_PID=$!

    # Update cleanup to also unmount workspace
    cleanup() {
        fusermount -u "$WS_MOUNT" 2>/dev/null || true
        kill "$WS_PID" 2>/dev/null || true
        fusermount -u "$MOUNT_POINT" 2>/dev/null || true
        kill "$OPENERAL_PID" 2>/dev/null || true
        wait "$OPENERAL_PID" 2>/dev/null || true
        wait "$WS_PID" 2>/dev/null || true
    }
    trap cleanup EXIT INT TERM

    # Wait for workspace mount
    ws_elapsed=0
    while ! mountpoint -q "$WS_MOUNT" 2>/dev/null; do
        if ! kill -0 "$WS_PID" 2>/dev/null; then
            echo "ERROR: workspace mount process exited before mount was ready." >&2
            exit 1
        fi
        if [ "$ws_elapsed" -ge "$TIMEOUT" ]; then
            echo "ERROR: workspace did not mount $WS_MOUNT within ${TIMEOUT}s." >&2
            kill "$WS_PID" 2>/dev/null || true
            exit 1
        fi
        sleep 1
        ws_elapsed=$((ws_elapsed + 1))
    done

    echo "workspace mounted at $WS_MOUNT (PID $WS_PID, workspace=$OPENERAL_WORKSPACE_ID)"
    export HOME="$WS_MOUNT"
fi

# --- Hand off to agent ---

exec "$@"
