#!/usr/bin/env node

/**
 * openeral CLI — run Claude Code with persistent PostgreSQL-backed home.
 *
 * Usage:
 *   npx openeral                      # interactive Claude Code
 *   npx openeral -- -p 'hello'        # non-interactive
 *   npx openeral --workspace myid     # custom workspace ID
 *
 * Required env:
 *   DATABASE_URL          PostgreSQL connection string
 *   ANTHROPIC_API_KEY     Claude API key
 *
 * Optional env:
 *   OPENERAL_WORKSPACE_ID   Workspace ID (default: hostname)
 *   OPENERAL_HOME           Home directory path (default: /tmp/openeral-<id>)
 */

import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync, existsSync, chmodSync } from 'node:fs';

function writePgHelper(path: string): void {
  // pg helper reads DATABASE_URL from the environment at runtime.
  // Never hardcode credentials — rely on env propagation from OpenShell providers.
  const script = `#!/bin/bash
# pg — query the database from Claude Code
# Usage: pg "SELECT * FROM public.users LIMIT 5"
if [ -z "$DATABASE_URL" ]; then
  echo "pg: DATABASE_URL is not set" >&2; exit 1
fi
if command -v psql >/dev/null 2>&1; then
  exec psql "$DATABASE_URL" -c "$*"
else
  exec node -e 'const p=require("pg"),o=new p.Pool({connectionString:process.env.DATABASE_URL});o.query(process.argv[1]).then(r=>{console.log(JSON.stringify(r.rows,null,2));o.end()}).catch(e=>{console.error(e.message);process.exit(1)})' "$*"
fi
`;
  writeFileSync(path, script);
  chmodSync(path, 0o755);
}
import { hostname } from 'node:os';
import { join } from 'node:path';
import { createPool } from './db/pool.js';
import { runMigrations } from './db/migrations.js';
import { syncToFs, syncFromFs, watchAndSync } from './sync.js';

async function main() {
  // --- Parse args ---
  const args = process.argv.slice(2);
  let workspaceId = process.env.OPENERAL_WORKSPACE_ID || hostname();
  let claudeArgs: string[] = [];

  // Split on -- to separate openeral args from claude args
  const dashIdx = args.indexOf('--');
  const ownArgs = dashIdx >= 0 ? args.slice(0, dashIdx) : args;
  claudeArgs = dashIdx >= 0 ? args.slice(dashIdx + 1) : [];

  for (let i = 0; i < ownArgs.length; i++) {
    if ((ownArgs[i] === '--workspace' || ownArgs[i] === '-w') && ownArgs[i + 1]) {
      workspaceId = ownArgs[++i];
    }
  }

  // --- Validate env ---
  const databaseUrl = process.env.DATABASE_URL;
  const persistenceEnabled = !!databaseUrl;

  if (!persistenceEnabled) {
    process.stderr.write(
      '\x1b[33mopeneral: DATABASE_URL not set — running without persistence\x1b[0m\n' +
      '\x1b[2m  Set DATABASE_URL to enable PostgreSQL-backed home directory\x1b[0m\n',
    );
  }

  if (!process.env.ANTHROPIC_API_KEY) {
    process.stderr.write(
      '\x1b[33mopeneral: ANTHROPIC_API_KEY not set — Claude Code may not work\x1b[0m\n',
    );
  }

  // --- Setup home directory ---
  const homeDir = process.env.OPENERAL_HOME || `/tmp/openeral-${workspaceId}`;
  mkdirSync(homeDir, { recursive: true });

  process.stderr.write(`\x1b[2mopeneral: workspace  ${workspaceId}\x1b[0m\n`);
  process.stderr.write(`\x1b[2mopeneral: home       ${homeDir}\x1b[0m\n`);
  process.stderr.write(`\x1b[2mopeneral: persist    ${persistenceEnabled ? 'PostgreSQL' : 'local only'}\x1b[0m\n`);

  // --- Database setup (only if DATABASE_URL is set) ---
  let pool: import('pg').Pool | null = null;
  let stopWatch: (() => void) | null = null;

  if (persistenceEnabled) {
    pool = createPool(databaseUrl);

    process.stderr.write('\x1b[2mopeneral: running migrations...\x1b[0m\n');
    await runMigrations(pool);

    // Ensure workspace config exists
    await pool.query(
      `INSERT INTO _openeral.workspace_config (id, display_name, config)
       VALUES ($1, $2, '{}'::jsonb)
       ON CONFLICT (id) DO NOTHING`,
      [workspaceId, workspaceId],
    );

    // Sync from PostgreSQL → filesystem
    process.stderr.write('\x1b[2mopeneral: syncing workspace...\x1b[0m\n');
    const synced = await syncToFs(pool, workspaceId, homeDir);
    process.stderr.write(`\x1b[2mopeneral: restored ${synced} files\x1b[0m\n`);

    // Write pg helper
    const pgHelper = join(homeDir, '.local', 'bin', 'pg');
    mkdirSync(join(homeDir, '.local', 'bin'), { recursive: true });
    writePgHelper(pgHelper);

    // Write CLAUDE.md
    const claudeMdPath = join(homeDir, 'CLAUDE.md');
    if (!existsSync(claudeMdPath)) {
      writeFileSync(claudeMdPath, `# OpenEral

Your home directory persists across sessions.

## Database

Query the connected database:

    pg "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'"
    pg "SELECT * FROM public.users LIMIT 5"
    pg "\\d public.users"

The \`pg\` command uses psql if available, otherwise Node.js pg.
`);
    }

    // Start file watcher
    process.stderr.write('\x1b[2mopeneral: watching for changes...\x1b[0m\n');
    stopWatch = watchAndSync(pool, workspaceId, homeDir);
  }

  // --- Launch Claude Code ---
  process.stderr.write('\x1b[2mopeneral: starting Claude Code\x1b[0m\n\n');

  const child = spawn('claude', claudeArgs, {
    stdio: 'inherit',
    env: {
      ...process.env,
      HOME: homeDir,
      PATH: `${join(homeDir, '.local', 'bin')}:${process.env.PATH}`,
    },
  });

  child.on('error', (err: any) => {
    if (err.code === 'ENOENT') {
      process.stderr.write(
        '\x1b[31mopeneral: `claude` not found. Install Claude Code:\x1b[0m\n' +
        '  npm install -g @anthropic-ai/claude-code\n' +
        '  # or: curl -fsSL https://claude.ai/install.sh | bash\n\n',
      );
    } else {
      process.stderr.write(`openeral: ${err.message}\n`);
    }
    process.exit(1);
  });

  child.on('exit', async (code) => {
    if (pool && stopWatch) {
      stopWatch();
      process.stderr.write('\n\x1b[2mopeneral: saving workspace...\x1b[0m\n');
      try {
        const saved = await syncFromFs(pool, workspaceId, homeDir);
        process.stderr.write(`\x1b[2mopeneral: saved ${saved} files\x1b[0m\n`);
      } catch (err: any) {
        process.stderr.write(`\x1b[31mopeneral: sync failed: ${err.message}\x1b[0m\n`);
      }
      await pool.end();
    }
    process.exit(code ?? 0);
  });

  // Forward signals to child
  for (const sig of ['SIGTERM', 'SIGINT', 'SIGHUP'] as const) {
    process.on(sig, () => child.kill(sig));
  }
}

main().catch((err) => {
  process.stderr.write(`\x1b[31mopeneral: ${err.message}\x1b[0m\n`);
  process.exit(1);
});
