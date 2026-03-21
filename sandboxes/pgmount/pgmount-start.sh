#!/usr/bin/env bash
set -euo pipefail

# --- Resolve database connection string ---

# Prefer file-based secret (Docker secrets / K8s volume mount)
if [ -f /run/secrets/pgmount_database_url ]; then
    export PGMOUNT_DATABASE_URL
    PGMOUNT_DATABASE_URL="$(cat /run/secrets/pgmount_database_url)"
fi

if [ -z "${PGMOUNT_DATABASE_URL:-}" ]; then
    echo "ERROR: PGMOUNT_DATABASE_URL is not set." >&2
    echo "Provide it via environment variable or mount a secret at /run/secrets/pgmount_database_url" >&2
    exit 1
fi

# --- Build pgmount flags ---

PGMOUNT_FLAGS=()

if [ -n "${PGMOUNT_SCHEMAS:-}" ]; then
    PGMOUNT_FLAGS+=(--schemas "$PGMOUNT_SCHEMAS")
fi

if [ -n "${PGMOUNT_PAGE_SIZE:-}" ]; then
    PGMOUNT_FLAGS+=(--page-size "$PGMOUNT_PAGE_SIZE")
fi

if [ -n "${PGMOUNT_CACHE_TTL:-}" ]; then
    PGMOUNT_FLAGS+=(--cache-ttl "$PGMOUNT_CACHE_TTL")
fi

if [ -n "${PGMOUNT_STATEMENT_TIMEOUT:-}" ]; then
    PGMOUNT_FLAGS+=(--statement-timeout "$PGMOUNT_STATEMENT_TIMEOUT")
fi

MOUNT_POINT="/db"
TIMEOUT="${PGMOUNT_TIMEOUT:-15}"

# --- Start pgmount in background ---

pgmount mount "${PGMOUNT_FLAGS[@]}" "$MOUNT_POINT" &
PGMOUNT_PID=$!

# --- Cleanup on exit ---

cleanup() {
    fusermount -u "$MOUNT_POINT" 2>/dev/null || true
    kill "$PGMOUNT_PID" 2>/dev/null || true
    wait "$PGMOUNT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- Wait for mount to be ready ---

elapsed=0
while ! mountpoint -q "$MOUNT_POINT" 2>/dev/null; do
    if ! kill -0 "$PGMOUNT_PID" 2>/dev/null; then
        echo "ERROR: pgmount process exited before mount was ready." >&2
        echo "Check PGMOUNT_DATABASE_URL and ensure PostgreSQL is reachable." >&2
        wait "$PGMOUNT_PID" 2>/dev/null
        exit 1
    fi

    if [ "$elapsed" -ge "$TIMEOUT" ]; then
        echo "ERROR: pgmount did not mount $MOUNT_POINT within ${TIMEOUT}s." >&2
        echo "Check connection string, network access, and PostgreSQL status." >&2
        kill "$PGMOUNT_PID" 2>/dev/null || true
        exit 1
    fi

    sleep 1
    elapsed=$((elapsed + 1))
done

echo "pgmount mounted at $MOUNT_POINT (PID $PGMOUNT_PID)"

# --- Optionally mount workspace ---

if [ -n "${PGMOUNT_WORKSPACE_ID:-}" ]; then
    WS_MOUNT="${PGMOUNT_WORKSPACE_MOUNT:-/home/agent}"
    WS_NAME="${PGMOUNT_WORKSPACE_NAME:-$PGMOUNT_WORKSPACE_ID}"
    WS_CONFIG="${PGMOUNT_WORKSPACE_CONFIG:-{}}"

    # Create workspace if it doesn't exist (idempotent — will fail silently if exists)
    pgmount workspace create \
        --display-name "$WS_NAME" \
        --config "$WS_CONFIG" \
        "$PGMOUNT_WORKSPACE_ID" 2>/dev/null || true

    # Mount workspace
    pgmount workspace mount "$PGMOUNT_WORKSPACE_ID" "$WS_MOUNT" &
    WS_PID=$!

    # Update cleanup to also unmount workspace
    cleanup() {
        fusermount -u "$WS_MOUNT" 2>/dev/null || true
        kill "$WS_PID" 2>/dev/null || true
        fusermount -u "$MOUNT_POINT" 2>/dev/null || true
        kill "$PGMOUNT_PID" 2>/dev/null || true
        wait "$PGMOUNT_PID" 2>/dev/null || true
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

    echo "workspace mounted at $WS_MOUNT (PID $WS_PID, workspace=$PGMOUNT_WORKSPACE_ID)"
    export HOME="$WS_MOUNT"
fi

# --- Hand off to agent ---

exec "$@"
