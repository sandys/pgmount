#!/usr/bin/env node

/**
 * Integration test: verifies openeral-js against a live PostgreSQL database.
 *
 * Usage: DATABASE_URL='...' node test-integration.mjs
 */

import { createOpeneralShell } from './dist/shell.js';

const connectionString = process.env.DATABASE_URL;
if (!connectionString) {
  console.error('DATABASE_URL required');
  process.exit(1);
}

const workspaceId = 'integration-test-' + Date.now();
let shell;
let passed = 0;
let failed = 0;

function assert(label, actual, expected) {
  const actualTrimmed = typeof actual === 'string' ? actual.trim() : actual;
  const expectedTrimmed = typeof expected === 'string' ? expected.trim() : expected;
  if (actualTrimmed === expectedTrimmed) {
    console.log(`  ✓ ${label}`);
    passed++;
  } else {
    console.error(`  ✗ ${label}`);
    console.error(`    expected: ${JSON.stringify(expectedTrimmed)}`);
    console.error(`    actual:   ${JSON.stringify(actualTrimmed)}`);
    failed++;
  }
}

function assertIncludes(label, actual, substring) {
  if (actual.includes(substring)) {
    console.log(`  ✓ ${label}`);
    passed++;
  } else {
    console.error(`  ✗ ${label}`);
    console.error(`    expected to include: ${JSON.stringify(substring)}`);
    console.error(`    actual: ${JSON.stringify(actual.slice(0, 200))}`);
    failed++;
  }
}

function assertExitCode(label, result, code) {
  if (result.exitCode === code) {
    console.log(`  ✓ ${label}`);
    passed++;
  } else {
    console.error(`  ✗ ${label}`);
    console.error(`    expected exitCode ${code}, got ${result.exitCode}`);
    console.error(`    stderr: ${result.stderr.slice(0, 200)}`);
    failed++;
  }
}

async function run() {
  console.log(`\nCreating shell (workspace: ${workspaceId})...\n`);

  shell = await createOpeneralShell({
    connectionString,
    workspaceId,
    migrate: true,
  });

  // --- /db tests ---
  console.log('=== /db (read-only database) ===');

  let r = await shell.exec('ls /db');
  assertExitCode('ls /db exits 0', r, 0);
  assertIncludes('ls /db includes "public"', r.stdout, 'public');

  r = await shell.exec('ls /db/public');
  assertExitCode('ls /db/public exits 0', r, 0);
  // Should list tables in the healthtracker database

  // Find a table to test with
  const tables = r.stdout.trim().split('\n').filter(Boolean);
  if (tables.length > 0) {
    const table = tables[0];
    console.log(`  (testing with table: ${table})`);

    r = await shell.exec(`cat /db/public/${table}/.info/columns.json`);
    assertExitCode(`cat columns.json exits 0`, r, 0);
    assertIncludes('columns.json is valid JSON', r.stdout, '"name"');

    r = await shell.exec(`cat /db/public/${table}/.info/count`);
    assertExitCode('cat count exits 0', r, 0);
    // Count should be a number
    const count = parseInt(r.stdout.trim());
    assert('count is a number', !isNaN(count), true);

    r = await shell.exec(`cat /db/public/${table}/.info/primary_key`);
    assertExitCode('cat primary_key exits 0', r, 0);

    r = await shell.exec(`cat /db/public/${table}/.info/schema.sql`);
    assertExitCode('cat schema.sql exits 0', r, 0);
    assertIncludes('schema.sql contains CREATE TABLE', r.stdout, 'CREATE TABLE');

    r = await shell.exec(`ls /db/public/${table}/.indexes`);
    assertExitCode('ls .indexes exits 0', r, 0);
  }

  // Read-only enforcement
  try {
    r = await shell.exec('echo test > /db/test.txt');
    assert('write to /db fails (exitCode)', r.exitCode !== 0, true);
  } catch (err) {
    // EROFS thrown directly — this is correct behavior
    assertIncludes('write to /db throws EROFS', err.code || err.message, 'EROFS');
  }

  // --- /home/agent tests ---
  console.log('\n=== /home/agent (read-write workspace) ===');

  r = await shell.exec('echo "hello openeral" > /home/agent/test.txt');
  assertExitCode('write file exits 0', r, 0);

  r = await shell.exec('cat /home/agent/test.txt');
  assertExitCode('cat file exits 0', r, 0);
  assert('file content matches', r.stdout, 'hello openeral\n');

  r = await shell.exec('mkdir -p /home/agent/.claude/projects/test');
  assertExitCode('mkdir -p exits 0', r, 0);

  r = await shell.exec('ls /home/agent/.claude/projects');
  assertExitCode('ls projects exits 0', r, 0);
  assertIncludes('projects contains test', r.stdout, 'test');

  r = await shell.exec('echo \'{"key":"value"}\' > /home/agent/.claude/settings.json');
  assertExitCode('write settings.json exits 0', r, 0);

  r = await shell.exec('cat /home/agent/.claude/settings.json');
  assertExitCode('cat settings.json exits 0', r, 0);
  assertIncludes('settings.json content', r.stdout, '"key":"value"');

  // --- Persistence test ---
  console.log('\n=== Persistence (new shell instance, same workspaceId) ===');

  const shell2 = await createOpeneralShell({
    connectionString,
    workspaceId,
    migrate: false,
  });

  r = await shell2.exec('cat /home/agent/test.txt');
  assertExitCode('persist: cat file exits 0', r, 0);
  assert('persist: file content matches', r.stdout, 'hello openeral\n');

  r = await shell2.exec('cat /home/agent/.claude/settings.json');
  assertExitCode('persist: cat settings.json exits 0', r, 0);
  assertIncludes('persist: settings.json content', r.stdout, '"key":"value"');

  r = await shell2.exec('ls /home/agent/.claude/projects');
  assertExitCode('persist: ls projects exits 0', r, 0);
  assertIncludes('persist: projects contains test', r.stdout, 'test');

  // --- Shell features ---
  console.log('\n=== Shell features (pipes, redirections, variables) ===');

  r = await shell.exec('echo "line1\nline2\nline3" | wc -l');
  assertExitCode('pipe + wc exits 0', r, 0);

  r = await shell.exec('echo $HOME');
  assertExitCode('echo $HOME exits 0', r, 0);
  assert('HOME is /home/agent', r.stdout, '/home/agent\n');

  r = await shell.exec('x=42 && echo $x');
  assertExitCode('variable assignment exits 0', r, 0);
  assert('variable value', r.stdout, '42\n');

  // --- pg custom command ---
  console.log('\n=== pg custom command (direct SQL) ===');

  r = await shell.exec('pg SELECT 1 as test');
  assertExitCode('pg query exits 0', r, 0);
  assertIncludes('pg query returns result', r.stdout, '"test"');

  // --- Summary ---
  console.log(`\n=== Results: ${passed} passed, ${failed} failed ===\n`);

  // Cleanup: remove test workspace
  const { createPool } = await import('./dist/db/pool.js');
  const pool = createPool(connectionString);
  await pool.query('DELETE FROM _openeral.workspace_files WHERE workspace_id = $1', [workspaceId]);
  await pool.query('DELETE FROM _openeral.workspace_config WHERE id = $1', [workspaceId]);
  await pool.end();

  process.exit(failed > 0 ? 1 : 0);
}

run().catch((err) => {
  console.error('Integration test failed:', err);
  process.exit(1);
});
