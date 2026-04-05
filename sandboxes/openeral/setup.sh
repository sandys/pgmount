#!/bin/bash
set -euo pipefail

# setup.sh — OpenEral sandbox entry point
#
# Called by: openshell sandbox create ... -- /opt/openeral/setup.sh
#
# Steps:
#   1. Run database migrations
#   2. Seed the workspace
#   3. Start openeral-bash daemon
#   4. Exec Claude Code

OPENERAL_DIR=/opt/openeral

# Resolve database connection string (OpenShell provider injects DATABASE_URL)
export DATABASE_URL="${DATABASE_URL:-${OPENERAL_DATABASE_URL:-}}"
if [ -z "$DATABASE_URL" ]; then
  echo "setup.sh: DATABASE_URL or OPENERAL_DATABASE_URL required" >&2
  exit 1
fi

# Workspace ID defaults to sandbox ID (set by OpenShell supervisor)
export WORKSPACE_ID="${OPENSHELL_SANDBOX_ID:-default}"

echo "setup.sh: running migrations..."
node -e "
  import('$OPENERAL_DIR/src/db/pool.js').then(async ({ createPool }) => {
    const { runMigrations } = await import('$OPENERAL_DIR/src/db/migrations.js');
    const pool = createPool(process.env.DATABASE_URL);
    await runMigrations(pool);
    await pool.end();
    console.log('setup.sh: migrations complete');
  }).catch(err => {
    console.error('setup.sh: migration failed:', err.message);
    process.exit(1);
  });
"

echo "setup.sh: seeding workspace $WORKSPACE_ID..."
node -e "
  import('$OPENERAL_DIR/src/db/pool.js').then(async ({ createPool }) => {
    const { runMigrations } = await import('$OPENERAL_DIR/src/db/migrations.js');
    const ws = await import('$OPENERAL_DIR/src/db/workspace-queries.js');
    const pool = createPool(process.env.DATABASE_URL);

    // Ensure workspace config exists
    try {
      await pool.query(
        \"INSERT INTO _openeral.workspace_config (id, display_name, config) VALUES (\\\$1, \\\$2, '{}'::jsonb) ON CONFLICT (id) DO NOTHING\",
        [process.env.WORKSPACE_ID, 'sandbox']
      );
    } catch {}

    // Seed root and .claude dirs
    await ws.seedFromConfig(pool, process.env.WORKSPACE_ID, {
      autoDirs: ['/', '/.claude', '/.claude/projects'],
      seedFiles: {},
    });

    await pool.end();
    console.log('setup.sh: workspace seeded');
  }).catch(err => {
    console.error('setup.sh: seed failed:', err.message);
    process.exit(1);
  });
"

echo "setup.sh: starting openeral-bash daemon..."
node "$OPENERAL_DIR/openeral-bash.mjs" --daemon &
DAEMON_PID=$!

# Wait for socket to appear
for i in $(seq 1 30); do
  [ -S /tmp/openeral-bash.sock ] && break
  sleep 0.1
done

if [ ! -S /tmp/openeral-bash.sock ]; then
  echo "setup.sh: daemon failed to start" >&2
  exit 1
fi

echo "setup.sh: daemon ready (pid $DAEMON_PID)"

# Clean up daemon on exit
trap "kill $DAEMON_PID 2>/dev/null; rm -f /tmp/openeral-bash.sock" EXIT

# Launch Claude Code with persistent home
echo "setup.sh: launching Claude Code..."
exec env \
  HOME=/home/agent \
  SHELL=/usr/local/bin/openeral-bash \
  claude "$@"
