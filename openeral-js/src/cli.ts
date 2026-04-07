#!/usr/bin/env node

/**
 * openeral CLI — run Claude Code with persistent PostgreSQL-backed home,
 * or refresh Claude's native memory files inside that persisted home.
 *
 * Usage:
 *   npx openeral
 *   npx openeral -- -p 'hello'
 *   npx openeral --workspace myid
 *   npx openeral memory refresh --query 'openshell proxy'
 */

import { spawn } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { hostname } from 'node:os';
import { join } from 'node:path';
import { createPool } from './db/pool.js';
import { runMigrations } from './db/migrations.js';
import { syncFromFs, syncToFs, watchAndSync } from './sync.js';
import { refreshClaudeMemory } from './memory/refresh.js';

function writePgHelper(path: string): void {
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

function writeClaudeGuide(homeDir: string): void {
  const claudeMdPath = join(homeDir, 'CLAUDE.md');
  if (existsSync(claudeMdPath)) return;

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

async function ensureWorkspaceConfig(pool: import('pg').Pool, workspaceId: string): Promise<void> {
  await pool.query(
    `INSERT INTO _openeral.workspace_config (id, display_name, config)
     VALUES ($1, $2, '{}'::jsonb)
     ON CONFLICT (id) DO NOTHING`,
    [workspaceId, workspaceId],
  );
}

function formatUsage(): string {
  return [
    'Usage:',
    '  openeral [--workspace <id>] [-- <claude args...>]',
    '  openeral memory refresh [--workspace <id>] [--project-root <path>] [--query <text>] [--dry-run] [--no-backup]',
    '',
    'Environment:',
    '  DATABASE_URL           Optional for launch, optional for memory refresh',
    '  ANTHROPIC_API_KEY      Required by Claude Code itself',
    '  OPENERAL_WORKSPACE_ID  Default workspace ID',
    '  OPENERAL_HOME          Default local home path',
  ].join('\n');
}

type LaunchCommand = {
  kind: 'launch';
  workspaceId: string;
  claudeArgs: string[];
};

type MemoryRefreshCommand = {
  kind: 'memory-refresh';
  workspaceId: string;
  query?: string;
  projectRoot?: string;
  dryRun: boolean;
  backup: boolean;
};

type CliCommand = LaunchCommand | MemoryRefreshCommand | { kind: 'help' };

export function parseCliArgs(argv: string[]): CliCommand {
  let workspaceId = process.env.OPENERAL_WORKSPACE_ID || hostname();

  if (argv[0] === 'memory') {
    if (argv[1] !== 'refresh') {
      throw new Error(`unknown subcommand: ${argv.slice(0, 2).join(' ') || 'memory'}`);
    }

    let query: string | undefined;
    let projectRoot: string | undefined;
    let dryRun = false;
    let backup = true;

    for (let i = 2; i < argv.length; i++) {
      const arg = argv[i];
      if (arg === '--workspace' || arg === '-w') {
        if (!argv[i + 1]) throw new Error(`${arg} requires a value`);
        workspaceId = argv[++i];
      } else if (arg === '--query' || arg === '-q') {
        if (!argv[i + 1]) throw new Error(`${arg} requires a value`);
        query = argv[++i];
      } else if (arg === '--project-root') {
        if (!argv[i + 1]) throw new Error(`${arg} requires a value`);
        projectRoot = argv[++i];
      } else if (arg === '--dry-run') {
        dryRun = true;
      } else if (arg === '--no-backup') {
        backup = false;
      } else if (arg === '--help' || arg === '-h') {
        return { kind: 'help' };
      } else {
        throw new Error(`unknown argument: ${arg}`);
      }
    }

    return {
      kind: 'memory-refresh',
      workspaceId,
      query,
      projectRoot,
      dryRun,
      backup,
    };
  }

  const dashIdx = argv.indexOf('--');
  const ownArgs = dashIdx >= 0 ? argv.slice(0, dashIdx) : argv;
  const claudeArgs = dashIdx >= 0 ? argv.slice(dashIdx + 1) : [];

  for (let i = 0; i < ownArgs.length; i++) {
    const arg = ownArgs[i];
    if (arg === '--workspace' || arg === '-w') {
      if (!ownArgs[i + 1]) throw new Error(`${arg} requires a value`);
      workspaceId = ownArgs[++i];
    } else if (arg === '--help' || arg === '-h') {
      return { kind: 'help' };
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }

  return {
    kind: 'launch',
    workspaceId,
    claudeArgs,
  };
}

function resolveHomeDir(workspaceId: string): string {
  return process.env.OPENERAL_HOME || `/tmp/openeral-${workspaceId}`;
}

function printLaunchHeader(workspaceId: string, homeDir: string, persistenceEnabled: boolean): void {
  process.stderr.write(`\x1b[2mopeneral: workspace  ${workspaceId}\x1b[0m\n`);
  process.stderr.write(`\x1b[2mopeneral: home       ${homeDir}\x1b[0m\n`);
  process.stderr.write(`\x1b[2mopeneral: persist    ${persistenceEnabled ? 'PostgreSQL' : 'local only'}\x1b[0m\n`);
}

function printMemorySummary(result: Awaited<ReturnType<typeof refreshClaudeMemory>>, persistenceEnabled: boolean): void {
  const modeLabel = result.mode === 'default' ? 'default' : 'focus';
  const verb = result.dryRun ? 'plan' : 'wrote';

  process.stdout.write(`openeral memory: mode         ${modeLabel}\n`);
  process.stdout.write(`openeral memory: persist      ${persistenceEnabled ? 'PostgreSQL' : 'local only'}\n`);
  process.stdout.write(`openeral memory: content root ${result.context.contentRoot}\n`);
  process.stdout.write(`openeral memory: memory root  ${result.context.memoryDir}\n`);
  if (result.backupDir) {
    process.stdout.write(`openeral memory: backup       ${result.backupDir}\n`);
  }
  process.stdout.write(`openeral memory: ${verb}        ${result.plannedFiles.join(', ')}\n`);
  if (result.topSources.length > 0) {
    process.stdout.write('openeral memory: top sources\n');
    for (const chunk of result.topSources.slice(0, 8)) {
      process.stdout.write(`  - ${chunk.relPath} (${chunk.score.toFixed(1)}): ${chunk.title}\n`);
    }
  }
}

async function runLaunchCommand(command: LaunchCommand): Promise<void> {
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

  const homeDir = resolveHomeDir(command.workspaceId);
  mkdirSync(homeDir, { recursive: true });
  printLaunchHeader(command.workspaceId, homeDir, persistenceEnabled);

  let pool: import('pg').Pool | null = null;
  let stopWatch: (() => void) | null = null;

  if (persistenceEnabled) {
    pool = createPool(databaseUrl);

    process.stderr.write('\x1b[2mopeneral: running migrations...\x1b[0m\n');
    await runMigrations(pool);
    await ensureWorkspaceConfig(pool, command.workspaceId);

    process.stderr.write('\x1b[2mopeneral: syncing workspace...\x1b[0m\n');
    const synced = await syncToFs(pool, command.workspaceId, homeDir);
    process.stderr.write(`\x1b[2mopeneral: restored ${synced} files\x1b[0m\n`);

    const pgHelper = join(homeDir, '.local', 'bin', 'pg');
    mkdirSync(join(homeDir, '.local', 'bin'), { recursive: true });
    writePgHelper(pgHelper);
    writeClaudeGuide(homeDir);

    process.stderr.write('\x1b[2mopeneral: watching for changes...\x1b[0m\n');
    stopWatch = watchAndSync(pool, command.workspaceId, homeDir);
  }

  // --- StringCost auto-presign ---
  const claudeEnv: Record<string, string | undefined> = {
    ...process.env,
    HOME: homeDir,
    PATH: `${join(homeDir, '.local', 'bin')}:${process.env.PATH}`,
  };

  if (process.env.STRINGCOST_API_KEY && process.env.ANTHROPIC_API_KEY) {
    process.stderr.write('\x1b[2mopeneral: presigning with StringCost...\x1b[0m\n');
    try {
      const res = await fetch('https://app.stringcost.com/v1/presign', {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${process.env.STRINGCOST_API_KEY}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          provider: 'anthropic',
          client_api_key: process.env.ANTHROPIC_API_KEY,
          path: ['/v1/messages'],
          expires_in: -1,
          max_uses: -1,
          tags: ['openeral'],
          metadata: { source: 'openeral' },
        }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json() as { url?: string };
      if (data.url) {
        claudeEnv.ANTHROPIC_BASE_URL = data.url.replace(/\/v1\/.*$/, '');
        process.stderr.write('\x1b[2mopeneral: StringCost enabled — costs tracked automatically\x1b[0m\n');
      }
    } catch (err: any) {
      process.stderr.write(`\x1b[33mopeneral: StringCost presign failed: ${err.message} — continuing without cost tracking\x1b[0m\n`);
    }
  }

  // --- Launch Claude Code ---
  process.stderr.write('\x1b[2mopeneral: starting Claude Code\x1b[0m\n\n');

  const child = spawn('claude', command.claudeArgs, {
    stdio: 'inherit',
    env: claudeEnv,
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
        const saved = await syncFromFs(pool, command.workspaceId, homeDir);
        process.stderr.write(`\x1b[2mopeneral: saved ${saved} files\x1b[0m\n`);
      } catch (err: any) {
        process.stderr.write(`\x1b[31mopeneral: sync failed: ${err.message}\x1b[0m\n`);
      }
      await pool.end();
    }
    process.exit(code ?? 0);
  });

  for (const sig of ['SIGTERM', 'SIGINT', 'SIGHUP'] as const) {
    process.on(sig, () => child.kill(sig));
  }
}

async function runMemoryRefreshCommand(command: MemoryRefreshCommand): Promise<void> {
  const databaseUrl = process.env.DATABASE_URL;
  const persistenceEnabled = !!databaseUrl;
  const homeDir = resolveHomeDir(command.workspaceId);
  mkdirSync(homeDir, { recursive: true });

  let pool: import('pg').Pool | null = null;

  try {
    if (persistenceEnabled) {
      pool = createPool(databaseUrl);
      await runMigrations(pool);
      await ensureWorkspaceConfig(pool, command.workspaceId);
      await syncToFs(pool, command.workspaceId, homeDir);
    }

    const result = await refreshClaudeMemory({
      homeDir,
      cwd: process.cwd(),
      projectRoot: command.projectRoot,
      query: command.query,
      dryRun: command.dryRun,
      backup: command.backup,
    });

    if (pool && !command.dryRun) {
      await syncFromFs(pool, command.workspaceId, homeDir);
    }

    printMemorySummary(result, persistenceEnabled);
  } finally {
    await pool?.end();
  }
}

export async function main(argv = process.argv.slice(2)): Promise<void> {
  const command = parseCliArgs(argv);
  if (command.kind === 'help') {
    process.stdout.write(`${formatUsage()}\n`);
    return;
  }
  if (command.kind === 'memory-refresh') {
    await runMemoryRefreshCommand(command);
    return;
  }
  await runLaunchCommand(command);
}
