#!/usr/bin/env bash
set -euo pipefail

: "${OPENSHELL_CLUSTER_IMAGE:?must be set}"
: "${OPENERAL_GATEWAY_IMAGE:?must be set}"
: "${OPENERAL_SANDBOX_IMAGE:?must be set}"

GATEWAY_NAME="${OPENSHELL_GATEWAY_NAME:-openeral-smoke-${RANDOM}}"
GATEWAY_PORT="${OPENSHELL_GATEWAY_PORT:-8080}"
SANDBOX_NAME="${OPENERAL_SANDBOX_NAME:-openeral-smoke-${RANDOM}}"
DB_PROVIDER="${OPENERAL_DB_PROVIDER:-openeral-db-${RANDOM}}"
DB_CONTAINER="${OPENERAL_SMOKE_DB_CONTAINER:-openeral-smoke-postgres}"
DB_PORT="${OPENERAL_SMOKE_DB_PORT:-15432}"
DOWNLOAD_DIR=""

cleanup() {
    set +e
    openshell sandbox delete --gateway "$GATEWAY_NAME" "$SANDBOX_NAME" >/dev/null 2>&1 || true
    openshell gateway destroy --name "$GATEWAY_NAME" >/dev/null 2>&1 || true
    docker rm -f "$DB_CONTAINER" >/dev/null 2>&1 || true
    if [ -n "$DOWNLOAD_DIR" ]; then
        rm -rf "$DOWNLOAD_DIR" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

docker rm -f "$DB_CONTAINER" >/dev/null 2>&1 || true
docker run -d \
    --name "$DB_CONTAINER" \
    -e POSTGRES_USER=pgmount \
    -e POSTGRES_PASSWORD=pgmount \
    -e POSTGRES_DB=testdb \
    -p "${DB_PORT}:5432" \
    postgres:16 >/dev/null

for _ in $(seq 1 30); do
    if docker exec "$DB_CONTAINER" pg_isready -U pgmount -d testdb >/dev/null 2>&1; then
        DB_READY=1
        break
    fi
    sleep 1
done

if [ "${DB_READY:-0}" != "1" ]; then
    echo "PostgreSQL did not become ready in time" >&2
    exit 1
fi

for _ in $(seq 1 30); do
    if PGPASSWORD=pgmount psql -h localhost -p "$DB_PORT" -U pgmount -d testdb -Atqc 'SELECT 1' >/dev/null 2>&1; then
        DB_HOST_READY=1
        break
    fi
    sleep 1
done

if [ "${DB_HOST_READY:-0}" != "1" ]; then
    echo "PostgreSQL did not accept host connections in time" >&2
    exit 1
fi

PGPASSWORD=pgmount psql -h localhost -p "$DB_PORT" -U pgmount -d testdb -q <<'SQL'
DROP TABLE IF EXISTS public.users CASCADE;
CREATE TABLE public.users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT
);
INSERT INTO public.users (id, name, email) VALUES
    (1, 'Ada Lovelace', 'ada@example.com');
SQL

export DATABASE_URL="host=host.docker.internal port=${DB_PORT} user=pgmount password=pgmount dbname=testdb"

openshell gateway start --name "$GATEWAY_NAME" --port "$GATEWAY_PORT"

ACTUAL_GATEWAY_IMAGE="$(
    openshell doctor exec --name "$GATEWAY_NAME" -- \
        kubectl -n openshell get statefulset openshell \
        -o jsonpath='{.spec.template.spec.containers[0].image}'
)"
if [ "$ACTUAL_GATEWAY_IMAGE" != "$OPENERAL_GATEWAY_IMAGE" ]; then
    echo "Expected gateway image ${OPENERAL_GATEWAY_IMAGE}, got ${ACTUAL_GATEWAY_IMAGE}" >&2
    exit 1
fi

openshell provider create \
    --gateway "$GATEWAY_NAME" \
    --name "$DB_PROVIDER" \
    --type generic \
    --credential DATABASE_URL

SANDBOX_OUTPUT="$(
    openshell sandbox create \
        --gateway "$GATEWAY_NAME" \
        --name "$SANDBOX_NAME" \
        --from "$OPENERAL_SANDBOX_IMAGE" \
        --provider "$DB_PROVIDER" \
        --no-tty -- \
        sh -lc '
            set -e
            test -e /dev/fuse
            id
            grep -E " /db | /home/agent " /proc/mounts
            test -w /home/agent
            cat /db/public/users/.filter/id/1/1/row.json
            printf persist-ok > /home/agent/manual.txt
            cat /home/agent/manual.txt
        '
)"
printf '%s\n' "$SANDBOX_OUTPUT"

printf '%s\n' "$SANDBOX_OUTPUT" | grep -q 'Ada Lovelace'
printf '%s\n' "$SANDBOX_OUTPUT" | grep -q 'persist-ok'
printf '%s\n' "$SANDBOX_OUTPUT" | grep -q 'uid='

DOWNLOAD_DIR="$(mktemp -d)"
openshell sandbox download \
    --gateway "$GATEWAY_NAME" \
    "$SANDBOX_NAME" \
    /home/agent/manual.txt \
    "$DOWNLOAD_DIR"

DOWNLOADED_MANUAL="$DOWNLOAD_DIR/manual.txt"
if [ ! -f "$DOWNLOADED_MANUAL" ]; then
    echo "Expected downloaded manual.txt at ${DOWNLOADED_MANUAL}" >&2
    exit 1
fi

if [ "$(cat "$DOWNLOADED_MANUAL")" != "persist-ok" ]; then
    echo "Expected downloaded manual.txt to contain persist-ok" >&2
    exit 1
fi

MANUAL_COUNT="$(
    PGPASSWORD=pgmount psql -h localhost -p "$DB_PORT" -U pgmount -d testdb -Atqc \
        "SELECT count(*) FROM _openeral.workspace_files WHERE path = '/manual.txt' AND content = convert_to('persist-ok', 'UTF8')"
)"
if [ "$MANUAL_COUNT" != "1" ]; then
    echo "Expected one persisted /manual.txt row, got ${MANUAL_COUNT}" >&2
    exit 1
fi
