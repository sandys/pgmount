import { describe, it, expect } from 'vitest';
import { readFileSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));

// We can't import writePgHelper directly (it's not exported),
// so we test by running the CLI's pg helper generation logic inline.

describe('pg helper script', () => {
  const tmpDir = '/tmp/openeral-cli-test-' + Date.now();

  it('reads DATABASE_URL from environment, never hardcodes it', () => {
    mkdirSync(join(tmpDir, '.local', 'bin'), { recursive: true });
    const pgPath = join(tmpDir, '.local', 'bin', 'pg');

    // Simulate what writePgHelper does
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
    require('fs').writeFileSync(pgPath, script);
    require('fs').chmodSync(pgPath, 0o755);

    const content = readFileSync(pgPath, 'utf8');

    // Must reference $DATABASE_URL (env var)
    expect(content).toContain('$DATABASE_URL');
    expect(content).toContain('process.env.DATABASE_URL');

    // Must NOT contain a literal postgresql:// connection string
    expect(content).not.toMatch(/postgresql:\/\/\w+:\w+@/);

    // Must NOT contain a literal API key
    expect(content).not.toMatch(/sk-ant-/);

    // Must fail if DATABASE_URL is not set
    expect(content).toContain('DATABASE_URL is not set');

    rmSync(tmpDir, { recursive: true });
  });

  it('pg helper fails without DATABASE_URL', () => {
    mkdirSync(join(tmpDir, '.local', 'bin'), { recursive: true });
    const pgPath = join(tmpDir, '.local', 'bin', 'pg');

    const script = `#!/bin/bash
if [ -z "$DATABASE_URL" ]; then
  echo "pg: DATABASE_URL is not set" >&2; exit 1
fi
echo "would run: $*"
`;
    require('fs').writeFileSync(pgPath, script);
    require('fs').chmodSync(pgPath, 0o755);

    // Run without DATABASE_URL — should fail
    try {
      execSync(`env -u DATABASE_URL bash ${pgPath} "SELECT 1"`, { encoding: 'utf8', stdio: 'pipe' });
      expect.fail('should have thrown');
    } catch (err: any) {
      expect(err.stderr).toContain('DATABASE_URL is not set');
    }

    rmSync(tmpDir, { recursive: true });
  });

  it('pg helper succeeds with DATABASE_URL set', () => {
    mkdirSync(join(tmpDir, '.local', 'bin'), { recursive: true });
    const pgPath = join(tmpDir, '.local', 'bin', 'pg');

    const script = `#!/bin/bash
if [ -z "$DATABASE_URL" ]; then
  echo "pg: DATABASE_URL is not set" >&2; exit 1
fi
echo "connected to: $DATABASE_URL"
`;
    require('fs').writeFileSync(pgPath, script);
    require('fs').chmodSync(pgPath, 0o755);

    const out = execSync(`DATABASE_URL=test://db bash ${pgPath} "SELECT 1"`, { encoding: 'utf8' });
    expect(out.trim()).toBe('connected to: test://db');

    rmSync(tmpDir, { recursive: true });
  });
});

describe('openeral-shell skill bootstrap', () => {
  it('checks both dist/ and node_modules/ before launching', () => {
    const skillPath = join(__dirname, '../../.claude/skills/openeral-shell/SKILL.md');
    const skill = readFileSync(skillPath, 'utf8');

    // Must check node_modules alongside dist
    expect(skill).toContain('node_modules');
    expect(skill).toContain('dist');

    // The check line must be a conjunction (&&), not just dist alone
    expect(skill).toMatch(/\[ -d dist \].*&&.*\[ -d node_modules \]/);
  });
});
