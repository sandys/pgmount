#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

if [ -f ./.env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

for cmd in docker openshell python3 psql; do
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "Missing required command: $cmd" >&2
    exit 1
  }
done

if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
  echo "Missing ANTHROPIC_API_KEY in environment or .env" >&2
  exit 1
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
tag="ralph-${stamp,,}"
gateway_name="openeral-ralph-${stamp,,}"
claude_sandbox="claude-${stamp,,}"
secret_sandbox="secret-pass-${stamp,,}"
deny_sandbox="secret-deny-${stamp,,}"
bad_auth_sandbox="secret-badauth-${stamp,,}"
db_provider="openeral-db-${stamp,,}"
db_container="openeral-secret-${stamp,,}"
use_existing_images="${LIVE_SECRET_USE_EXISTING_IMAGES:-0}"
repo_base_in_cluster="${IMAGE_REPO_BASE:-172.17.0.1:5000/openshell/openeral}"
cluster_image_host="${OPENSHELL_CLUSTER_IMAGE:-127.0.0.1:5000/openshell/openeral/cluster:${tag}}"
gateway_image_cluster="${OPENERAL_GATEWAY_IMAGE:-${repo_base_in_cluster}/gateway:${tag}}"
sandbox_image_cluster="${OPENERAL_SANDBOX_IMAGE:-${repo_base_in_cluster}/sandbox:${tag}}"
result_file="$repo_root/.omx/logs/live-secret-${stamp}.env"
summary_file="$repo_root/.omx/logs/live-secret-${stamp}.summary.txt"
claude_policy_file="$repo_root/.omx/logs/live-secret-${stamp}.claude.policy.yaml"
secret_policy_file="$repo_root/.omx/logs/live-secret-${stamp}.secret.policy.yaml"
claude_logs_file="$repo_root/.omx/logs/live-secret-${stamp}.claude.logs.txt"
secret_logs_file="$repo_root/.omx/logs/live-secret-${stamp}.secret.logs.txt"
deny_logs_file="$repo_root/.omx/logs/live-secret-${stamp}.deny.logs.txt"
bad_auth_logs_file="$repo_root/.omx/logs/live-secret-${stamp}.badauth.logs.txt"

export OPENSHELL_CLUSTER_IMAGE="$cluster_image_host"
export OPENSHELL_REGISTRY_HOST="${OPENSHELL_REGISTRY_HOST:-172.17.0.1:5000}"
export OPENSHELL_REGISTRY_INSECURE="${OPENSHELL_REGISTRY_INSECURE:-true}"
export IMAGE_REPO_BASE="$repo_base_in_cluster"
export OPENERAL_SANDBOX_IMAGE="$sandbox_image_cluster"
export OPENSHELL_GATEWAY_NAME="$gateway_name"

pick_db_port() {
python3 - <<'PY'
import socket
for start in (15432, 15433, 15434, 15435, 15436, 15437, 15438, 15439, 15440, 15441, 15442, 15443, 15444, 15445, 15446, 15447, 15448, 15449, 15450):
    s = socket.socket()
    try:
        s.bind(('127.0.0.1', start))
    except OSError:
        continue
    else:
        print(start)
        s.close()
        break
PY
}

pick_gateway_port() {
python3 - <<'PY'
import socket
for start in (18080, 18081, 18082, 18083, 18084, 18085, 18086, 18087, 18088, 18089):
    s = socket.socket()
    try:
        s.bind(('127.0.0.1', start))
    except OSError:
        continue
    else:
        print(start)
        s.close()
        break
PY
}

db_port="$(pick_db_port)"
gateway_port="$(pick_gateway_port)"

cat > "$result_file" <<ENVVARS
STAMP=$stamp
TAG=$tag
GATEWAY_NAME=$gateway_name
CLAUDE_SANDBOX=$claude_sandbox
SECRET_SANDBOX=$secret_sandbox
DENY_SANDBOX=$deny_sandbox
BADAUTH_SANDBOX=$bad_auth_sandbox
DB_PROVIDER=$db_provider
DB_CONTAINER=$db_container
DB_PORT=$db_port
GATEWAY_PORT=$gateway_port
CLUSTER_IMAGE_HOST=$cluster_image_host
GATEWAY_IMAGE_CLUSTER=$gateway_image_cluster
SANDBOX_IMAGE_CLUSTER=$sandbox_image_cluster
SUMMARY_FILE=$summary_file
ENVVARS

log() { printf '\n[%s] %s\n' "$(date -u +%H:%M:%S)" "$*"; }

collect_logs() {
  local sandbox_name="$1"
  local output_file="$2"
  local pod_name=""

  for _ in $(seq 1 15); do
    : >"$output_file"
    openshell logs --gateway "$gateway_name" "$sandbox_name" --source sandbox --since 10m -n 500 >"$output_file" 2>/dev/null || true
    if ! grep -q 'L7_REQUEST\|CONNECT' "$output_file"; then
      openshell logs --gateway "$gateway_name" "$sandbox_name" --source gateway --since 10m -n 500 >>"$output_file" 2>/dev/null || true
    fi
    if ! grep -q 'L7_REQUEST\|CONNECT' "$output_file"; then
      pod_name="$({ openshell doctor exec --name "$gateway_name" -- kubectl -n openshell get pods -o name | sed 's#pod/##' | grep "^${sandbox_name}$" | head -n1; echo; } | tail -n1)"
      if [ -n "$pod_name" ]; then
        openshell doctor exec --name "$gateway_name" -- kubectl -n openshell logs "$pod_name" --tail=500 >>"$output_file" 2>/dev/null || true
      fi
    fi
    if grep -q 'L7_REQUEST\|CONNECT' "$output_file"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_policy_match() {
  local sandbox_name="$1"
  local output_file="$2"
  local pattern="$3"
  for _ in $(seq 1 20); do
    openshell policy get --gateway "$gateway_name" "$sandbox_name" --full >"$output_file" 2>/dev/null || true
    if grep -Eq "$pattern" "$output_file"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

cleanup_old_named_resources() {
  docker rm -f "$db_container" >/dev/null 2>&1 || true
  openshell sandbox delete --gateway "$gateway_name" "$claude_sandbox" >/dev/null 2>&1 || true
  openshell sandbox delete --gateway "$gateway_name" "$secret_sandbox" >/dev/null 2>&1 || true
  openshell sandbox delete --gateway "$gateway_name" "$deny_sandbox" >/dev/null 2>&1 || true
  openshell sandbox delete --gateway "$gateway_name" "$bad_auth_sandbox" >/dev/null 2>&1 || true
  openshell gateway destroy --name "$gateway_name" >/dev/null 2>&1 || true
}

cleanup_old_named_resources

log "Checking registry"
if ! docker ps --format '{{.Names}}' | grep -qx openshell-local-registry; then
  docker run -d --restart=always -p 5000:5000 --name openshell-local-registry registry:2 >/dev/null
fi

log "Starting fresh postgres on port $db_port"
docker run -d \
  --name "$db_container" \
  -e POSTGRES_USER=pgmount \
  -e POSTGRES_PASSWORD=pgmount \
  -e POSTGRES_DB=testdb \
  -p "${db_port}:5432" \
  postgres:16 >/dev/null

for _ in $(seq 1 60); do
  if docker exec "$db_container" pg_isready -U pgmount -d testdb >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

for _ in $(seq 1 60); do
  if PGPASSWORD=pgmount psql -h localhost -p "$db_port" -U pgmount -d testdb -Atqc 'SELECT 1' >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

PGPASSWORD=pgmount psql -h localhost -p "$db_port" -U pgmount -d testdb -Atqc 'SELECT 1' >/dev/null
PGPASSWORD=pgmount psql -h localhost -p "$db_port" -U pgmount -d testdb <<'SQL'
DROP TABLE IF EXISTS public.users CASCADE;
CREATE TABLE public.users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT
);
INSERT INTO public.users (id, name, email) VALUES (1, 'Ada Lovelace', 'ada@example.com');
SQL
export DATABASE_URL="host=host.docker.internal port=${db_port} user=pgmount password=pgmount dbname=testdb"

if [ "$use_existing_images" = "1" ]; then
  log "Using existing image refs"
  printf 'USE_EXISTING_IMAGES=1\n' | tee -a "$result_file"
else
  log "Building no-cache cluster image $cluster_image_host"
  docker build --no-cache \
    -f vendor/openshell/deploy/docker/Dockerfile.images \
    --target cluster \
    --build-arg OPENERAL_DEFAULT_IMAGE_REPO_BASE="$repo_base_in_cluster" \
    --build-arg OPENERAL_DEFAULT_IMAGE_TAG="$tag" \
    -t "$cluster_image_host" \
    vendor/openshell

  docker push "$cluster_image_host"

  log "Building no-cache gateway image ${gateway_image_cluster/172.17.0.1/127.0.0.1}"
  docker build --no-cache \
    -f vendor/openshell/deploy/docker/Dockerfile.images \
    --target gateway \
    -t "127.0.0.1:5000/openshell/openeral/gateway:${tag}" \
    vendor/openshell

  docker push "127.0.0.1:5000/openshell/openeral/gateway:${tag}"

  log "Building no-cache sandbox image ${sandbox_image_cluster/172.17.0.1/127.0.0.1}"
  docker build --no-cache \
    -f sandboxes/openeral/Dockerfile \
    -t "127.0.0.1:5000/openshell/openeral/sandbox:${tag}" \
    .

  docker push "127.0.0.1:5000/openshell/openeral/sandbox:${tag}"
fi

log "Starting gateway $gateway_name on port $gateway_port"
openshell gateway start --name "$gateway_name" --port "$gateway_port"

actual_gateway_image="$({ openshell doctor exec --name "$gateway_name" -- kubectl -n openshell get statefulset openshell -o jsonpath='{.spec.template.spec.containers[0].image}'; echo; } | tail -n1)"
printf 'ACTUAL_GATEWAY_IMAGE=%s\n' "$actual_gateway_image" | tee -a "$result_file"
if [ -n "${gateway_image_cluster:-}" ] && [ "$actual_gateway_image" != "$gateway_image_cluster" ]; then
  echo "Expected gateway image $gateway_image_cluster but got $actual_gateway_image" >&2
  exit 1
fi

log "Creating database provider $db_provider"
openshell provider create \
  --gateway "$gateway_name" \
  --name "$db_provider" \
  --type generic \
  --credential DATABASE_URL

log "Running Claude ready check sandbox"
claude_status=0
claude_output_file="$repo_root/.omx/logs/live-secret-${stamp}.claude.out"
if ! openshell sandbox create \
  --gateway "$gateway_name" \
  --name "$claude_sandbox" \
  --from "$sandbox_image_cluster" \
  --provider "$db_provider" \
  --provider claude \
  --auto-providers \
  --no-tty -- \
  sh -lc 'set -e
    echo "RUN_ID='"$stamp"'"
    id
    echo "ANTHROPIC_ENV=$ANTHROPIC_API_KEY"
    grep -E " /db | /home/agent " /proc/mounts
    stat -c "%u:%g %a %n" /home/agent
    HOME=/home/agent claude -p "Reply with READY and nothing else."
  ' \
  >"$claude_output_file" 2>&1; then
  claude_status=$?
fi
printf 'CLAUDE_STATUS=%s\n' "$claude_status" | tee -a "$result_file"
if [ "$claude_status" -ne 0 ]; then
  echo "Claude positive control failed" >&2
  exit "$claude_status"
fi
grep -q 'ANTHROPIC_ENV=openshell:resolve:env:ANTHROPIC_API_KEY' "$claude_output_file"
grep -q '^READY$' "$claude_output_file"
wait_for_policy_match "$claude_sandbox" "$claude_policy_file" 'claude_code:'
wait_for_policy_match "$claude_sandbox" "$claude_policy_file" 'secret_injection:'
grep -q 'match_headers: \[x-api-key\]' "$claude_policy_file"
collect_logs "$claude_sandbox" "$claude_logs_file"
grep -q 'L7_REQUEST' "$claude_logs_file"
grep -q 'policy=claude_code' "$claude_logs_file"
grep -q 'secret_injection_action=applied' "$claude_logs_file"
grep -q 'secret_swaps' "$claude_logs_file"

log "Querying persisted Claude workspace rows"
claude_row_count="$(PGPASSWORD=pgmount psql -h localhost -p "$db_port" -U pgmount -d testdb -Atqc "SELECT count(*) FROM _openeral.workspace_files WHERE path LIKE '/.claude%'")"
printf 'CLAUDE_ROW_COUNT=%s\n' "$claude_row_count" | tee -a "$result_file"
if [ "${claude_row_count:-0}" -le 0 ]; then
  echo "Expected persisted /.claude* rows for Claude positive control" >&2
  exit 1
fi

log "Running curl positive control sandbox"
secret_output_file="$repo_root/.omx/logs/live-secret-${stamp}.secret.out"
openshell sandbox create \
  --gateway "$gateway_name" \
  --name "$secret_sandbox" \
  --from "$sandbox_image_cluster" \
  --provider "$db_provider" \
  --provider claude \
  --auto-providers \
  --no-tty -- \
  sh -lc 'set -e
    echo "RUN_ID='"$stamp"'"
    test -e /dev/fuse
    command -v curl
    grep -E " /db | /home/agent " /proc/mounts
    stat -c "%u:%g %a %n" /home/agent
    echo "ANTHROPIC_ENV=$ANTHROPIC_API_KEY"
    test "$ANTHROPIC_API_KEY" = "openshell:resolve:env:ANTHROPIC_API_KEY"
    printf persist-ok > /home/agent/manual.txt
    code=$(curl -sS -o /tmp/models.json -w "%{http_code}" \
      https://api.anthropic.com/v1/models \
      -H "x-api-key: $ANTHROPIC_API_KEY" \
      -H "anthropic-version: 2023-06-01")
    printf "HTTP_CODE=%s\n" "$code"
    grep -q '"type":"list"' /tmp/models.json
    head -c 200 /tmp/models.json; echo
    cat /db/public/users/.filter/id/1/1/row.json
  ' >"$secret_output_file" 2>&1
grep -q 'ANTHROPIC_ENV=openshell:resolve:env:ANTHROPIC_API_KEY' "$secret_output_file"
grep -q 'HTTP_CODE=200' "$secret_output_file"
grep -q 'claude-' "$secret_output_file"
wait_for_policy_match "$secret_sandbox" "$secret_policy_file" 'anthropic_secret_test:'
wait_for_policy_match "$secret_sandbox" "$secret_policy_file" 'secret_injection:'
grep -q 'match_headers: \[x-api-key\]' "$secret_policy_file"
collect_logs "$secret_sandbox" "$secret_logs_file"
grep -q 'L7_REQUEST' "$secret_logs_file"
grep -q 'policy=anthropic_secret_test' "$secret_logs_file"
grep -q 'secret_injection_action=applied' "$secret_logs_file"
grep -q 'l7_target=/v1/models' "$secret_logs_file"

log "Querying persisted workspace rows"
manual_count="$(PGPASSWORD=pgmount psql -h localhost -p "$db_port" -U pgmount -d testdb -Atqc "SELECT count(*) FROM _openeral.workspace_files WHERE path = '/manual.txt' AND content = convert_to('persist-ok', 'UTF8')")"
printf 'MANUAL_COUNT=%s\n' "$manual_count" | tee -a "$result_file"
if [ "$manual_count" != "1" ]; then
  echo "Expected one persisted /manual.txt row, got ${manual_count}" >&2
  exit 1
fi

log "Running boundary-denial negative control"
deny_output_file="$repo_root/.omx/logs/live-secret-${stamp}.deny.out"
openshell sandbox create \
  --gateway "$gateway_name" \
  --name "$deny_sandbox" \
  --from "$sandbox_image_cluster" \
  --provider "$db_provider" \
  --provider claude \
  --auto-providers \
  --no-tty -- \
  sh -lc 'set -e
    echo "RUN_ID='"$stamp"'"
    echo "ANTHROPIC_ENV=$ANTHROPIC_API_KEY"
    code=$(curl -sS -o /tmp/deny.txt -w "%{http_code}" \
      https://api.anthropic.com/v1/models \
      -H "Authorization: Bearer $ANTHROPIC_API_KEY" \
      -H "anthropic-version: 2023-06-01")
    printf "HTTP_CODE=%s\n" "$code"
    head -c 200 /tmp/deny.txt; echo
  ' >"$deny_output_file" 2>&1
grep -q 'HTTP_CODE=403' "$deny_output_file"
collect_logs "$deny_sandbox" "$deny_logs_file"
grep -q 'L7_REQUEST' "$deny_logs_file"
grep -q 'l7_decision=deny' "$deny_logs_file"
grep -q 'secret_injection_action=denied' "$deny_logs_file"

log "Running upstream-auth negative control"
bad_auth_output_file="$repo_root/.omx/logs/live-secret-${stamp}.badauth.out"
openshell sandbox create \
  --gateway "$gateway_name" \
  --name "$bad_auth_sandbox" \
  --from "$sandbox_image_cluster" \
  --provider "$db_provider" \
  --provider claude \
  --auto-providers \
  --no-tty -- \
  sh -lc 'set -e
    echo "RUN_ID='"$stamp"'"
    code=$(curl -sS -o /tmp/badauth.txt -w "%{http_code}" \
      https://api.anthropic.com/v1/models \
      -H "x-api-key: not-a-real-key" \
      -H "anthropic-version: 2023-06-01")
    printf "HTTP_CODE=%s\n" "$code"
    head -c 200 /tmp/badauth.txt; echo
  ' >"$bad_auth_output_file" 2>&1
grep -Eq 'HTTP_CODE=40[13]' "$bad_auth_output_file"
collect_logs "$bad_auth_sandbox" "$bad_auth_logs_file"
grep -q 'L7_REQUEST' "$bad_auth_logs_file"
grep -q 'policy=anthropic_secret_test' "$bad_auth_logs_file"
grep -q 'secret_injection_action=none' "$bad_auth_logs_file"

{
  echo "stamp=$stamp"
  echo "tag=$tag"
  echo "gateway=$gateway_name"
  echo "db_provider=$db_provider"
  echo "claude_status=$claude_status"
  echo "claude_row_count=$claude_row_count"
  echo "manual_count=$manual_count"
  echo "actual_gateway_image=$actual_gateway_image"
  echo "claude_policy_file=$claude_policy_file"
  echo "secret_policy_file=$secret_policy_file"
  echo "claude_output_file=$claude_output_file"
  echo "secret_output_file=$secret_output_file"
  echo "deny_output_file=$deny_output_file"
  echo "bad_auth_output_file=$bad_auth_output_file"
  echo "claude_logs_file=$claude_logs_file"
  echo "secret_logs_file=$secret_logs_file"
  echo "deny_logs_file=$deny_logs_file"
  echo "bad_auth_logs_file=$bad_auth_logs_file"
} > "$summary_file"

log "DONE"
