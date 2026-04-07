#!/usr/bin/env bash
set -euo pipefail

# test_claude_e2e.sh — Runs real Claude Code via the built openeral bin and verifies
# end-to-end: persistence across sessions, pg command, file operations.
#
# This is the REAL test — not structural, not Docker shape. It launches
# Claude Code, has it write files, then launches again and reads them back.
#
# Requires: DATABASE_URL, ANTHROPIC_API_KEY, openeral-js built (dist/ + node_modules/)
# Usage:
#   source .env
#   DATABASE_URL='postgresql://...' ./tests/test_claude_e2e.sh

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root/openeral-js"

DB_URL="${DATABASE_URL:?DATABASE_URL required}"
API_KEY="${ANTHROPIC_API_KEY:?ANTHROPIC_API_KEY required}"
WORKSPACE="claude-e2e-$$"
PASSED=0
FAILED=0

pass() { echo "  ✓ $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ✗ $1"; FAILED=$((FAILED + 1)); }

# Ensure built
if [ ! -d dist ] || [ ! -d node_modules ]; then
  echo "Building openeral-js..."
  pnpm install && pnpm build
fi

# Clean any leftover home dir
rm -rf "/tmp/openeral-$WORKSPACE"

run_claude() {
  DATABASE_URL="$DB_URL" \
  ANTHROPIC_API_KEY="$API_KEY" \
  OPENERAL_WORKSPACE_ID="$WORKSPACE" \
  node dist/bin/openeral.js -- "$@" --dangerously-skip-permissions 2>&1
}

echo ""
echo "=== Session 1: Write files ==="
out=$(run_claude -p 'Run: printf "%s" "e2e-persist-check" > "$HOME/persist-test.txt" && echo WRITTEN — reply with just the output')
echo "$out" | tail -5
if echo "$out" | grep -qi 'WRITTEN\|written\|Done'; then
  pass "session 1: file written"
else
  fail "session 1: write failed"
fi

echo ""
echo "=== Session 1: pg command ==="
out=$(run_claude -p 'Run: pg "SELECT 1 as alive" — reply with just the output')
echo "$out" | tail -5
if echo "$out" | grep -q 'alive'; then
  pass "session 1: pg command works"
else
  fail "session 1: pg command failed"
fi

echo ""
echo "=== Session 1: Verify HOME ==="
out=$(run_claude -p 'Run: echo $HOME — reply with just the output')
echo "$out" | tail -5
if echo "$out" | grep -q "/tmp/openeral-$WORKSPACE"; then
  pass "session 1: HOME is correct"
else
  fail "session 1: HOME unexpected"
fi

# Delete the local home dir to prove session 2 restores from PostgreSQL
echo ""
echo "=== Deleting local home dir (force restore from PostgreSQL) ==="
rm -rf "/tmp/openeral-$WORKSPACE"

echo ""
echo "=== Session 2: Read persisted file ==="
out=$(run_claude -p 'Run: cat "$HOME/persist-test.txt" — reply with just the output')
echo "$out" | tail -5
if echo "$out" | grep -q 'e2e-persist-check'; then
  pass "session 2: file persisted from session 1"
else
  fail "session 2: file NOT persisted"
fi

echo ""
echo "=== Session 2: .claude state persisted ==="
out=$(run_claude -p 'Run: ls "$HOME/.claude/" — reply with just the output')
echo "$out" | tail -5
if echo "$out" | grep -qi 'settings\|projects'; then
  pass "session 2: .claude/ state persisted"
else
  fail "session 2: .claude/ state missing"
fi

# Cleanup
echo ""
echo "=== Cleanup ==="
DATABASE_URL="$DB_URL" node -e "
  import('pg').then(async({default:pg})=>{
    const pool=new pg.Pool({connectionString:process.env.DATABASE_URL});
    await pool.query('DELETE FROM _openeral.workspace_files WHERE workspace_id=\$1',['$WORKSPACE']);
    await pool.query('DELETE FROM _openeral.workspace_config WHERE id=\$1',['$WORKSPACE']);
    await pool.end();console.log('cleaned up');
  });
" 2>/dev/null || true
rm -rf "/tmp/openeral-$WORKSPACE"

echo ""
echo "=== Results: $PASSED passed, $FAILED failed ==="
[ "$FAILED" -eq 0 ] || exit 1
