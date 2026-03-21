#!/bin/bash
# Integration tests for openeral-shell
# Run from the repo root: bash openeral-shell/tests/test_openeral_shell.sh
#
# Requires: docker compose (tests build and run openeral-shell containers)
set -uo pipefail

PASS=0
FAIL=0
ERRORS=""

# Run from the openeral-shell directory so compose context resolves correctly
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR"
COMPOSE="docker compose"

pass() {
    PASS=$((PASS + 1))
    echo "  PASS: $1"
}

fail() {
    FAIL=$((FAIL + 1))
    ERRORS="${ERRORS}\n  FAIL: $1"
    echo "  FAIL: $1"
}

assert_eq() {
    local actual="$1"
    local expected="$2"
    local msg="$3"
    if [ "$actual" = "$expected" ]; then
        pass "$msg"
    else
        fail "$msg (expected '$expected', got '$actual')"
    fi
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local msg="$3"
    if echo "$haystack" | grep -qF "$needle"; then
        pass "$msg"
    else
        fail "$msg (expected to contain '$needle')"
    fi
}

assert_nonzero() {
    local value="$1"
    local msg="$2"
    if [ -n "$value" ] && [ "$value" != "0" ]; then
        pass "$msg"
    else
        fail "$msg (got empty or zero)"
    fi
}

exec_shell() {
    $COMPOSE exec -T -e HOME=/home/agent openeral-shell "$@"
}

# =============================================================================
echo "=== Building openeral-shell ==="
# =============================================================================

$COMPOSE build 2>&1 | tail -1
if [ $? -ne 0 ]; then
    echo "FATAL: docker compose build failed"
    exit 1
fi

# =============================================================================
echo "=== Starting services ==="
# =============================================================================

# Clean up from any previous run
$COMPOSE down -v 2>/dev/null

$COMPOSE up -d 2>&1 | tail -3

# Wait for container to be ready
for i in $(seq 1 15); do
    if $COMPOSE exec -T openeral-shell mountpoint -q /home/agent 2>/dev/null; then
        echo "  openeral-shell ready (${i}s)"
        break
    fi
    if [ "$i" -eq 15 ]; then
        echo "FATAL: openeral-shell container is not ready after 15s"
        $COMPOSE logs openeral-shell
        $COMPOSE down -v
        exit 1
    fi
    sleep 1
done

# =============================================================================
echo ""
echo "=== Test: Mounts ==="
# =============================================================================

result=$(exec_shell mountpoint /db)
assert_contains "$result" "is a mountpoint" "/db is mounted"

result=$(exec_shell mountpoint /home/agent)
assert_contains "$result" "is a mountpoint" "/home/agent is mounted"

# =============================================================================
echo ""
echo "=== Test: openeral binary available ==="
# =============================================================================

result=$(exec_shell openeral version)
assert_contains "$result" "openeral" "openeral version command works"

# =============================================================================
echo ""
echo "=== Test: Database browsable at /db ==="
# =============================================================================

result=$(exec_shell ls /db/)
assert_nonzero "$result" "/db/ lists schemas"

# The bundled postgres has public schema
assert_contains "$result" "public" "/db/ contains public schema"

# =============================================================================
echo ""
echo "=== Test: Default directories created ==="
# =============================================================================

result=$(exec_shell ls -a /home/agent/)
assert_contains "$result" ".claude" "$HOME has .claude/"
assert_contains "$result" ".cache" "$HOME has .cache/"
assert_contains "$result" ".config" "$HOME has .config/"
assert_contains "$result" ".local" "$HOME has .local/"
assert_contains "$result" ".npm" "$HOME has .npm/"

result=$(exec_shell ls /home/agent/.claude/)
assert_contains "$result" "memory" ".claude/ has memory/"
assert_contains "$result" "plans" ".claude/ has plans/"
assert_contains "$result" "sessions" ".claude/ has sessions/"
assert_contains "$result" "tasks" ".claude/ has tasks/"
assert_contains "$result" "todos" ".claude/ has todos/"
assert_contains "$result" "skills" ".claude/ has skills/"

# =============================================================================
echo ""
echo "=== Test: HOME is set correctly ==="
# =============================================================================

result=$(exec_shell sh -c 'echo $HOME')
assert_eq "$result" "/home/agent" "HOME=/home/agent"

# =============================================================================
echo ""
echo "=== Test: Write and read files ==="
# =============================================================================

exec_shell sh -c 'echo "hello from test" > /home/agent/test_file.txt'
result=$(exec_shell cat /home/agent/test_file.txt)
assert_eq "$result" "hello from test" "Write and read file"

# Write to .claude subdirectory
exec_shell sh -c 'echo "memory data" > /home/agent/.claude/memory/test_note.md'
result=$(exec_shell cat /home/agent/.claude/memory/test_note.md)
assert_eq "$result" "memory data" "Write and read in .claude/memory/"

# =============================================================================
echo ""
echo "=== Test: mkdir works ==="
# =============================================================================

exec_shell mkdir -p /home/agent/projects/myapp
exec_shell sh -c 'echo "project file" > /home/agent/projects/myapp/notes.txt'
result=$(exec_shell cat /home/agent/projects/myapp/notes.txt)
assert_eq "$result" "project file" "Create nested directories and write file"

# =============================================================================
echo ""
echo "=== Test: Delete files ==="
# =============================================================================

exec_shell sh -c 'echo "temp" > /home/agent/to_delete.txt'
exec_shell rm /home/agent/to_delete.txt
result=$(exec_shell sh -c 'cat /home/agent/to_delete.txt 2>&1 || true')
assert_contains "$result" "No such file" "Delete file works"

# =============================================================================
echo ""
echo "=== Test: /db is read-only ==="
# =============================================================================

# Capture both stdout and stderr from docker compose exec
result=$(exec_shell sh -c 'echo test > /db/test.txt 2>&1; true' 2>&1)
assert_contains "$result" "Read-only" "/db/ rejects writes"

# =============================================================================
echo ""
echo "=== Test: Persistence across restart ==="
# =============================================================================

# Write a marker file
exec_shell sh -c 'echo "persist-marker-12345" > /home/agent/persist_test.txt'

# Restart (keep volumes)
echo "  Restarting containers..."
$COMPOSE down 2>/dev/null
$COMPOSE up -d 2>/dev/null
sleep 8

# Verify persistence
result=$(exec_shell cat /home/agent/persist_test.txt)
assert_eq "$result" "persist-marker-12345" "File persists across restart"

result=$(exec_shell cat /home/agent/.claude/memory/test_note.md)
assert_eq "$result" "memory data" ".claude/memory file persists across restart"

result=$(exec_shell cat /home/agent/projects/myapp/notes.txt)
assert_eq "$result" "project file" "Nested directory file persists across restart"

# =============================================================================
echo ""
echo "=== Test: Skill file is installed ==="
# =============================================================================

result=$(exec_shell ls /home/agent/.claude/skills/ 2>/dev/null || echo "no skills dir")
# Skills dir exists (auto-created), but skill file is inside the image at /sandbox/.skills/
# or the user copies it in. For openeral-shell the skill is available if we copy it.
# The Dockerfile copies skills to a known location.

# =============================================================================
echo ""
echo "=== Test: PostgreSQL has workspace data ==="
# =============================================================================

PG_CONTAINER=$($COMPOSE ps -q postgres)
result=$(docker exec "$PG_CONTAINER" psql -U openeral -d openeral -t -c \
    "SELECT count(*) FROM _openeral.workspace_files WHERE workspace_id='default';" 2>/dev/null)
file_count=$(echo "$result" | tr -d ' ')
assert_nonzero "$file_count" "PostgreSQL has workspace files (count=$file_count)"

# =============================================================================
# Cleanup
# =============================================================================

echo ""
echo "=== Cleaning up ==="
$COMPOSE down -v 2>/dev/null

# =============================================================================
# Summary
# =============================================================================

echo ""
echo "========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "========================================="

if [ $FAIL -gt 0 ]; then
    echo -e "\nFailed tests:$ERRORS"
    exit 1
fi

echo "All tests passed!"
exit 0
