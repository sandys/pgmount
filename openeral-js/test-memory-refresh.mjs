#!/usr/bin/env node

import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import pg from 'pg';

const DB_URL = process.env.DATABASE_URL;
if (!DB_URL) {
  console.error('DATABASE_URL required');
  process.exit(1);
}

const workspaceId = `memory-refresh-${Date.now()}`;
const repoDir = mkdtempSync(join(tmpdir(), 'openeral-memory-project-'));
const homeDir = mkdtempSync(join(tmpdir(), 'openeral-memory-home-'));

let passed = 0;
let failed = 0;

function ok(label) {
  console.log(`  \x1b[32m✓\x1b[0m ${label}`);
  passed++;
}

function fail(label, detail) {
  console.error(`  \x1b[31m✗\x1b[0m ${label}`);
  if (detail) console.error(`    ${detail}`);
  failed++;
}

try {
  mkdirSync(join(repoDir, '.claude', 'rules'), { recursive: true });
  writeFileSync(join(repoDir, 'README.md'), [
    '# Demo Project',
    '',
    'Use `pnpm install` and `pnpm check` to build and verify.',
    '',
    'For sandbox launch run `openshell sandbox create --from demo`.',
  ].join('\n'));
  writeFileSync(join(repoDir, 'CLAUDE.md'), [
    '# Project Rules',
    '',
    '- Never skip integration tests.',
    '- Always review the real runtime path.',
  ].join('\n'));
  writeFileSync(join(repoDir, '.claude', 'rules', 'workflow.md'), [
    '# Workflow',
    '',
    'Debug with reproducible commands.',
    '',
    '`node test-memory-refresh.mjs` verifies persistence.',
  ].join('\n'));
  writeFileSync(join(repoDir, 'package.json'), JSON.stringify({
    name: 'demo-memory-project',
    scripts: {
      build: 'tsc',
      check: 'vitest run',
    },
  }, null, 2));

  execFileSync('git', ['init'], { cwd: repoDir, stdio: 'ignore' });
  execFileSync('git', ['config', 'user.email', 'test@example.com'], { cwd: repoDir, stdio: 'ignore' });
  execFileSync('git', ['config', 'user.name', 'Test User'], { cwd: repoDir, stdio: 'ignore' });
  execFileSync('git', ['add', '.'], { cwd: repoDir, stdio: 'ignore' });
  execFileSync('git', ['commit', '-m', 'init'], { cwd: repoDir, stdio: 'ignore' });

  execFileSync(process.execPath, [
    'dist/bin/openeral.js',
    'memory',
    'refresh',
    '--workspace', workspaceId,
    '--project-root', repoDir,
    '--query', 'build and test',
  ], {
    cwd: new URL('.', import.meta.url),
    env: {
      ...process.env,
      DATABASE_URL: DB_URL,
      OPENERAL_HOME: homeDir,
    },
    stdio: 'pipe',
  });
  ok('memory refresh command completed');

  const { resolveProjectContext } = await import('./dist/memory/resolve.js');
  const context = resolveProjectContext({
    homeDir,
    cwd: repoDir,
    projectRoot: repoDir,
  });

  const indexPath = join(context.memoryDir, 'MEMORY.md');
  const focusPath = join(context.memoryDir, 'focus-build-and-test.md');

  if (existsSync(indexPath)) ok('MEMORY.md written');
  else fail('MEMORY.md missing');

  if (existsSync(focusPath)) ok('focus file written');
  else fail('focus file missing');

  if (existsSync(focusPath)) {
    const focusContent = readFileSync(focusPath, 'utf8');
    if (focusContent.includes('pnpm install') && focusContent.includes('pnpm check')) {
      ok('focus file captured project commands');
    } else {
      fail('focus file missing expected commands', focusContent);
    }
  }

  const pool = new pg.Pool({ connectionString: DB_URL });
  try {
    const { rows } = await pool.query(
      `SELECT path FROM _openeral.workspace_files
       WHERE workspace_id = $1 AND path LIKE $2
       ORDER BY path`,
      [workspaceId, `/.claude/projects/${context.projectSlug}/memory/%`],
    );

    if (rows.some((row) => row.path.endsWith('/MEMORY.md'))) ok('MEMORY.md persisted to PostgreSQL');
    else fail('MEMORY.md missing in PostgreSQL');

    if (rows.some((row) => row.path.endsWith('/focus-build-and-test.md'))) ok('focus file persisted to PostgreSQL');
    else fail('focus file missing in PostgreSQL');

    await pool.query('DELETE FROM _openeral.workspace_files WHERE workspace_id = $1', [workspaceId]);
    await pool.query('DELETE FROM _openeral.workspace_config WHERE id = $1', [workspaceId]);
  } finally {
    await pool.end();
  }
} catch (err) {
  fail('script crashed', err instanceof Error ? err.message : String(err));
} finally {
  rmSync(repoDir, { recursive: true, force: true });
  rmSync(homeDir, { recursive: true, force: true });
}

console.log(`\n=== RESULTS: ${passed} passed, ${failed} failed ===`);
process.exit(failed > 0 ? 1 : 0);
