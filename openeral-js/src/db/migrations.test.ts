import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const src = readFileSync(join(__dirname, 'migrations.ts'), 'utf8');

describe('migrations.ts structural checks', () => {
  it('uses pg_advisory_lock to serialize concurrent callers', () => {
    expect(src).toContain('pg_advisory_lock');
  });

  it('releases the advisory lock in a finally block', () => {
    expect(src).toContain('pg_advisory_unlock');
    // The unlock must be inside a finally so it runs even on error
    const finallyIdx = src.indexOf('finally');
    const unlockIdx = src.indexOf('pg_advisory_unlock');
    expect(finallyIdx).toBeGreaterThan(-1);
    expect(unlockIdx).toBeGreaterThan(finallyIdx);
  });

  it('acquires a connection from the pool (not pool.query) for lock scope', () => {
    // Advisory locks are session-scoped — must use a single client connection,
    // not pool.query which may use different connections for lock and unlock
    expect(src).toContain('pool.connect()');
    expect(src).toContain('client.query');
    expect(src).toContain('client.release()');
  });
});
