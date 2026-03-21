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

# --- Hand off to agent ---

exec "$@"
