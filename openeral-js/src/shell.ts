/**
 * Shell factory — creates a just-bash instance with PostgreSQL-backed
 * virtual filesystems mounted at /db (read-only) and /home/agent (read-write).
 *
 * Usage:
 *   const shell = await createOpeneralShell({ connectionString, workspaceId })
 *   const result = await shell.exec('cat /db/public/users/.info/count')
 */

import { Bash, defineCommand, MountableFs, InMemoryFs } from 'just-bash';
import type pg from 'pg';
import { PgFs } from './pg-fs/pg-fs.js';
import { WorkspaceFs } from './workspace-fs/workspace-fs.js';
import { createPool } from './db/pool.js';
import { runMigrations } from './db/migrations.js';
import { analyzeCommand } from './safety.js';
import type { AnalysisResult } from './safety.js';

export const EXECUTION_LIMITS = {
  maxCommandCount: 1000,
  maxLoopIterations: 1000,
  maxCallDepth: 50,
  maxSubstitutionDepth: 20,
  maxSourceDepth: 10,
  maxFileDescriptors: 100,
  maxAwkIterations: 1000,
  maxSedIterations: 1000,
  maxJqIterations: 1000,
  maxGlobOperations: 10000,
  maxArrayElements: 10000,
  maxBraceExpansionResults: 1000,
  maxOutputSize: 1024 * 1024,
  maxStringLength: 1024 * 1024,
  maxHeredocSize: 1024 * 1024,
};

export interface OpeneralShellOptions {
  /** PostgreSQL connection string */
  connectionString: string;
  /** Workspace ID for /home/agent persistence */
  workspaceId: string;
  /** Rows per page in /db (default: 1000) */
  pageSize?: number;
  /** Cache TTL in milliseconds (default: 30000) */
  cacheTtlMs?: number;
  /** Enable Python WASM runtime (default: false) */
  python?: boolean;
  /** Enable JavaScript QuickJS runtime (default: false) */
  javascript?: boolean;
  /** Run migrations on startup (default: true) */
  migrate?: boolean;
  /** Execution limits override */
  executionLimits?: Partial<typeof EXECUTION_LIMITS>;
  /** Extra environment variables */
  env?: Record<string, string>;
  /** Extra custom commands */
  customCommands?: any[];
  /** Provide an existing pg.Pool instead of creating one */
  pool?: pg.Pool;
}

function makePgCommand(pool: pg.Pool) {
  return defineCommand('pg', async (args: string[]) => {
    const sql = args.join(' ');
    if (!sql.trim()) {
      return { stdout: '', stderr: 'Usage: pg <SQL query>\n', exitCode: 1 };
    }
    try {
      const result = await pool.query(sql);
      return {
        stdout: JSON.stringify(result.rows, null, 2) + '\n',
        stderr: '',
        exitCode: 0,
      };
    } catch (err: any) {
      return {
        stdout: '',
        stderr: `pg error: ${err.message}\n`,
        exitCode: 1,
      };
    }
  });
}

/**
 * Create a sandboxed Bash instance with PostgreSQL-backed virtual filesystems.
 *
 * Mount points:
 *   /db          — read-only view of the database (PgFs)
 *   /home/agent  — read-write persistent workspace (WorkspaceFs)
 *   /tmp         — ephemeral in-memory storage
 */
export async function createOpeneralShell(opts: OpeneralShellOptions): Promise<Bash> {
  const pool = opts.pool ?? createPool(opts.connectionString);

  if (opts.migrate !== false) {
    await runMigrations(pool);
  }

  const pgFs = new PgFs(pool, {
    pageSize: opts.pageSize,
    cacheTtlMs: opts.cacheTtlMs,
  });
  const wsFs = new WorkspaceFs(pool, opts.workspaceId);

  const fs = new MountableFs({
    base: new InMemoryFs({ '/tmp': {} }),
    mounts: [
      { mountPoint: '/db', filesystem: pgFs },
      { mountPoint: '/home/agent', filesystem: wsFs },
    ],
  });

  const pgCommand = makePgCommand(pool);

  const customCommands = [pgCommand, ...(opts.customCommands ?? [])];

  const bash = new Bash({
    fs,
    cwd: '/home/agent',
    env: {
      HOME: '/home/agent',
      DATABASE_URL: opts.connectionString,
      WORKSPACE_ID: opts.workspaceId,
      ...(opts.env ?? {}),
    },
    customCommands,
    defenseInDepth: true,
    executionLimits: { ...EXECUTION_LIMITS, ...(opts.executionLimits ?? {}) },
    python: opts.python ?? false,
    javascript: opts.javascript ?? false,
  });

  await bash.exec('shopt -s expand_aliases');
  return bash;
}

export interface ExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

/**
 * Create an agent tool handler that wraps shell.exec() with optional
 * pre-execution safety analysis.
 */
export function createToolHandler(
  shell: Bash,
  opts?: { enforceSafety?: boolean },
): (command: string) => Promise<ExecResult> {
  return async (command: string): Promise<ExecResult> => {
    if (opts?.enforceSafety) {
      const analysis = await analyzeCommand(command);
      if (!analysis.safe) {
        return {
          stdout: '',
          stderr: `blocked: ${analysis.reason}\n`,
          exitCode: 1,
        };
      }
    }

    const result = await shell.exec(command);
    return {
      stdout: result.stdout,
      stderr: result.stderr,
      exitCode: result.exitCode,
    };
  };
}

export { analyzeCommand } from './safety.js';
export type { AnalysisResult } from './safety.js';
