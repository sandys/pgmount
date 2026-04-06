#!/usr/bin/env node

/**
 * End-to-end test: simulates two Claude Code sessions with persistence.
 *
 * Session 1:
 *   - Creates workspace
 *   - Writes Claude Code state files (.claude.json, .claude/settings.json, etc.)
 *   - Explores /db (schemas, tables, rows, info files)
 *   - Writes working files under /home/agent
 *   - Verifies pg custom command
 *   - Calls the Anthropic API (if ANTHROPIC_API_KEY is set)
 *
 * Session 2 (new shell, same workspaceId):
 *   - Verifies ALL files from Session 1 persist
 *   - Reads and verifies content
 *   - Appends to a file, verifies
 *   - Renames a file, verifies
 *   - Deletes a file, verifies it's gone
 *
 * Usage:
 *   DATABASE_URL='...' node test-e2e-claude.mjs
 *   DATABASE_URL='...' ANTHROPIC_API_KEY='...' node test-e2e-claude.mjs
 */

import { readFileSync } from 'node:fs';

// Load .env if present (does NOT override existing env vars)
try {
  const env = readFileSync(new URL('../.env', import.meta.url), 'utf8');
  for (const line of env.split('\n')) {
    const m = line.match(/^([A-Z_][A-Z0-9_]*)=(.*)$/);
    if (m && !process.env[m[1]]) process.env[m[1]] = m[2];
  }
} catch {}

const DB_URL = process.env.DATABASE_URL;
if (!DB_URL) {
  console.error('DATABASE_URL required');
  process.exit(1);
}
const API_KEY = process.env.ANTHROPIC_API_KEY || '';
const WORKSPACE = 'e2e-claude-' + Date.now();

let passed = 0;
let failed = 0;
let skipped = 0;

function ok(label) { console.log(`  \x1b[32m✓\x1b[0m ${label}`); passed++; }
function fail(label, detail) { console.error(`  \x1b[31m✗\x1b[0m ${label}`); if (detail) console.error(`    ${detail}`); failed++; }
function skip(label) { console.log(`  \x1b[33m○\x1b[0m ${label} (skipped)`); skipped++; }

async function exec(shell, cmd) {
  try {
    return await shell.exec(cmd);
  } catch (err) {
    return { stdout: '', stderr: err.message, exitCode: 99, _error: err };
  }
}

function check(label, r, opts = {}) {
  const { exitCode = 0, includes, equals, notEmpty } = opts;
  if (r.exitCode !== exitCode) {
    fail(label, `exitCode ${r.exitCode}, expected ${exitCode}. stderr: ${r.stderr.slice(0, 200)}`);
    return false;
  }
  if (includes && !r.stdout.includes(includes)) {
    fail(label, `stdout missing '${includes}'. got: ${r.stdout.slice(0, 200)}`);
    return false;
  }
  if (equals !== undefined && r.stdout.trim() !== equals.trim()) {
    fail(label, `expected '${equals.trim()}', got '${r.stdout.trim()}'`);
    return false;
  }
  if (notEmpty && !r.stdout.trim()) {
    fail(label, `stdout is empty`);
    return false;
  }
  ok(label);
  return true;
}

async function run() {
  const { createOpeneralShell } = await import('./dist/shell.js');
  const { createPool } = await import('./dist/db/pool.js');

  // =========================================================================
  console.log(`\n\x1b[1m=== SESSION 1: Create workspace, write state, explore /db ===\x1b[0m`);
  console.log(`  workspace: ${WORKSPACE}`);
  console.log(`  database:  ${DB_URL.replace(/password=[^&\s]+/, 'password=***')}\n`);

  const shell1 = await createOpeneralShell({
    connectionString: DB_URL,
    workspaceId: WORKSPACE,
  });

  // -- /db exploration --
  console.log('\x1b[1m--- /db read-only database ---\x1b[0m');
  let r;

  r = await exec(shell1, 'ls /db');
  check('ls /db', r, { includes: 'public' });

  r = await exec(shell1, 'ls /db/public');
  check('ls /db/public lists tables', r, { notEmpty: true });
  const tables = r.stdout.trim().split('\n').filter(Boolean);
  const table = tables[0];
  console.log(`  (using table: public.${table})`);

  r = await exec(shell1, `cat /db/public/${table}/.info/columns.json`);
  check('columns.json is valid JSON', r, { includes: '"name"' });

  r = await exec(shell1, `cat /db/public/${table}/.info/schema.sql`);
  check('schema.sql has CREATE TABLE', r, { includes: 'CREATE TABLE' });

  r = await exec(shell1, `cat /db/public/${table}/.info/count`);
  const countOk = check('count is a number', r);
  if (countOk) {
    const count = parseInt(r.stdout.trim());
    if (isNaN(count)) fail('count parses to int', `got '${r.stdout.trim()}'`);
    else ok(`count = ${count}`);
  }

  r = await exec(shell1, `cat /db/public/${table}/.info/primary_key`);
  check('primary_key', r, { notEmpty: true });

  r = await exec(shell1, `ls /db/public/${table}/.indexes`);
  check('ls .indexes', r);

  r = await exec(shell1, `ls /db/public/${table}/.export`);
  check('ls .export', r, { includes: 'data.json' });

  // /db read-only enforcement
  try {
    r = await exec(shell1, 'echo nope > /db/readonly.txt');
    if (r.exitCode !== 0 || r._error) ok('/db write fails');
    else fail('/db write should fail', `exitCode=${r.exitCode}`);
  } catch (err) {
    if (err.code === 'EROFS' || err.message?.includes('EROFS')) ok('/db write throws EROFS');
    else fail('/db write throws unexpected error', err.message);
  }

  // -- /home/agent workspace --
  console.log('\n\x1b[1m--- /home/agent workspace (session 1 writes) ---\x1b[0m');

  // Simulate Claude Code writing its state files
  r = await exec(shell1, 'mkdir -p /home/agent/.claude/projects/default');
  check('mkdir .claude/projects/default', r);

  r = await exec(shell1, `echo '{"claudeVersion":"4.6","lastActive":"2026-04-05"}' > /home/agent/.claude.json`);
  check('write .claude.json', r);

  r = await exec(shell1, `echo '{"theme":"dark","autoSave":true}' > /home/agent/.claude/settings.json`);
  check('write settings.json', r);

  r = await exec(shell1, `echo '# Project Notes\n\nThis is a test project.' > /home/agent/.claude/projects/default/NOTES.md`);
  check('write project NOTES.md', r);

  r = await exec(shell1, `echo 'line1\nline2\nline3' > /home/agent/worklog.txt`);
  check('write worklog.txt', r);

  // Verify reads in same session
  r = await exec(shell1, 'cat /home/agent/.claude.json');
  check('read .claude.json', r, { includes: 'claudeVersion' });

  r = await exec(shell1, 'cat /home/agent/.claude/settings.json');
  check('read settings.json', r, { includes: 'autoSave' });

  r = await exec(shell1, 'ls /home/agent/.claude/projects/default');
  check('ls projects/default', r, { includes: 'NOTES.md' });

  r = await exec(shell1, 'cat /home/agent/worklog.txt');
  check('read worklog.txt', r, { includes: 'line1' });

  // -- Shell features --
  console.log('\n\x1b[1m--- Shell features ---\x1b[0m');

  r = await exec(shell1, 'echo $HOME');
  check('$HOME is /home/agent', r, { equals: '/home/agent' });

  r = await exec(shell1, 'cat /home/agent/worklog.txt | wc -l');
  check('pipe: wc -l', r);

  r = await exec(shell1, 'x=hello && echo "$x world"');
  check('variables', r, { equals: 'hello world' });

  r = await exec(shell1, 'for i in 1 2 3; do echo $i; done');
  check('for loop', r, { includes: '1' });

  // -- pg command --
  console.log('\n\x1b[1m--- pg custom command ---\x1b[0m');

  r = await exec(shell1, 'pg SELECT 1 as alive');
  check('pg SELECT 1', r, { includes: '"alive"' });

  r = await exec(shell1, `pg "SELECT count(*)::int as n FROM information_schema.tables WHERE table_schema = 'public'"`);
  check('pg count tables', r, { includes: '"n"' });

  // -- Anthropic API test (if key available) --
  console.log('\n\x1b[1m--- Anthropic API ---\x1b[0m');

  if (API_KEY && API_KEY !== 'openshell:resolve:env:ANTHROPIC_API_KEY') {
    try {
      const resp = await fetch('https://api.anthropic.com/v1/messages', {
        method: 'POST',
        headers: {
          'x-api-key': API_KEY,
          'anthropic-version': '2023-06-01',
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          model: 'claude-haiku-4-5-20251001',
          max_tokens: 16,
          messages: [{ role: 'user', content: 'Reply with exactly: READY' }],
        }),
      });

      if (resp.ok) {
        const data = await resp.json();
        const text = data.content?.[0]?.text || '';
        if (text.includes('READY')) ok('Claude API responds READY');
        else fail('Claude API unexpected response', text);
      } else {
        fail('Claude API HTTP error', `${resp.status} ${resp.statusText}`);
      }
    } catch (err) {
      fail('Claude API call failed', err.message);
    }
  } else {
    skip('Claude API (no ANTHROPIC_API_KEY)');
  }

  // =========================================================================
  console.log(`\n\x1b[1m=== SESSION 2: New shell, verify persistence ===\x1b[0m\n`);

  // Create a completely new shell with the same workspaceId
  const shell2 = await createOpeneralShell({
    connectionString: DB_URL,
    workspaceId: WORKSPACE,
    migrate: false, // already migrated
  });

  console.log('\x1b[1m--- Verify all files from Session 1 persist ---\x1b[0m');

  r = await exec(shell2, 'cat /home/agent/.claude.json');
  check('persist: .claude.json', r, { includes: 'claudeVersion' });

  r = await exec(shell2, 'cat /home/agent/.claude/settings.json');
  check('persist: settings.json', r, { includes: 'autoSave' });

  r = await exec(shell2, 'cat /home/agent/.claude/projects/default/NOTES.md');
  check('persist: NOTES.md', r, { includes: 'test project' });

  r = await exec(shell2, 'cat /home/agent/worklog.txt');
  check('persist: worklog.txt', r, { includes: 'line1' });

  r = await exec(shell2, 'ls /home/agent/.claude/projects/default');
  check('persist: ls projects/default', r, { includes: 'NOTES.md' });

  // -- Mutate in session 2 --
  console.log('\n\x1b[1m--- Mutate in Session 2 ---\x1b[0m');

  r = await exec(shell2, `echo 'line4' >> /home/agent/worklog.txt`);
  check('append to worklog.txt', r);

  r = await exec(shell2, 'cat /home/agent/worklog.txt');
  check('appended content visible', r, { includes: 'line4' });

  r = await exec(shell2, 'mv /home/agent/worklog.txt /home/agent/worklog-old.txt');
  check('rename worklog.txt', r);

  r = await exec(shell2, 'cat /home/agent/worklog-old.txt');
  check('renamed file readable', r, { includes: 'line1' });

  r = await exec(shell2, 'ls /home/agent');
  if (r.stdout.includes('worklog.txt') && !r.stdout.includes('worklog-old.txt')) {
    fail('old name should be gone', r.stdout);
  } else {
    ok('rename: old name gone, new name present');
  }

  r = await exec(shell2, 'rm /home/agent/worklog-old.txt');
  check('rm file', r);

  r = await exec(shell2, 'ls /home/agent');
  if (r.stdout.includes('worklog')) {
    fail('deleted file should be gone', r.stdout);
  } else {
    ok('delete: file gone');
  }

  // -- /db still works in session 2 --
  console.log('\n\x1b[1m--- /db still accessible in Session 2 ---\x1b[0m');

  r = await exec(shell2, 'ls /db/public');
  check('session 2: ls /db/public', r, { notEmpty: true });

  r = await exec(shell2, `cat /db/public/${table}/.info/count`);
  check('session 2: count', r, { notEmpty: true });

  // =========================================================================
  console.log(`\n\x1b[1m=== SESSION 3: Fresh shell, verify Session 2 mutations persisted ===\x1b[0m\n`);

  const shell3 = await createOpeneralShell({
    connectionString: DB_URL,
    workspaceId: WORKSPACE,
    migrate: false,
  });

  r = await exec(shell3, 'cat /home/agent/.claude.json');
  check('session 3: .claude.json persists', r, { includes: 'claudeVersion' });

  r = await exec(shell3, 'ls /home/agent');
  if (r.stdout.includes('worklog')) {
    fail('session 3: deleted file should still be gone', r.stdout);
  } else {
    ok('session 3: delete persisted');
  }

  // =========================================================================
  // Verify in PostgreSQL directly
  console.log(`\n\x1b[1m=== Direct PostgreSQL verification ===\x1b[0m\n`);

  const pool = createPool(DB_URL);

  const { rows } = await pool.query(
    `SELECT path, is_dir, size FROM _openeral.workspace_files
     WHERE workspace_id = $1 ORDER BY path`,
    [WORKSPACE],
  );

  console.log(`  Files in PostgreSQL for workspace ${WORKSPACE}:`);
  for (const row of rows) {
    console.log(`    ${row.is_dir ? 'd' : 'f'} ${String(row.size).padStart(6)} ${row.path}`);
  }

  if (rows.some(r => r.path === '/.claude.json')) ok('PG: .claude.json exists');
  else fail('PG: .claude.json missing');

  if (rows.some(r => r.path === '/.claude/settings.json')) ok('PG: settings.json exists');
  else fail('PG: settings.json missing');

  if (!rows.some(r => r.path.includes('worklog'))) ok('PG: deleted worklog is gone');
  else fail('PG: deleted worklog still in DB');

  // Cleanup
  await pool.query('DELETE FROM _openeral.workspace_files WHERE workspace_id = $1', [WORKSPACE]);
  await pool.query('DELETE FROM _openeral.workspace_config WHERE id = $1', [WORKSPACE]);
  await pool.end();

  // =========================================================================
  console.log(`\n\x1b[1m=== RESULTS: ${passed} passed, ${failed} failed, ${skipped} skipped ===\x1b[0m\n`);
  process.exit(failed > 0 ? 1 : 0);
}

run().catch((err) => {
  console.error('\x1b[31mE2E test crashed:\x1b[0m', err);
  process.exit(1);
});
