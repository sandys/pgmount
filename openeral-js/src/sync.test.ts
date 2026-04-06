import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';

const syncSrc = readFileSync('src/sync.ts', 'utf8');

describe('sync.ts structural checks', () => {
  it('syncFromFs tracks seen paths for deletion', () => {
    expect(syncSrc).toContain('seenPaths');
    expect(syncSrc).toContain('seenPaths.add(');
  });

  it('syncFromFs deletes DB rows not seen on disk', () => {
    expect(syncSrc).toMatch(/DELETE FROM _openeral\.workspace_files/);
    expect(syncSrc).toContain('!seenPaths.has(');
  });

  it('syncFromFs uses st.mode, not hardcoded values', () => {
    // Only check the walkDir function body (exclude the root dir INSERT which
    // is allowed to use a literal 0o40755 since there's no stat() for root)
    const walkDirStart = syncSrc.indexOf('async function walkDir');
    const walkDirEnd = syncSrc.indexOf('// Ensure root exists');
    const walkDirBody = syncSrc.slice(walkDirStart, walkDirEnd);
    expect(walkDirBody).toContain('st.mode');
    // walkDir INSERT statements must use st.mode, not hardcoded modes
    const insertStatements = walkDirBody.match(/INSERT INTO[\s\S]*?ON CONFLICT[\s\S]*?\]/g) || [];
    for (const stmt of insertStatements) {
      expect(stmt).not.toMatch(/0o40755|0o100644/);
    }
  });

  it('syncToFs applies chmod after writing files', () => {
    expect(syncSrc).toContain('chmodSync');
    // chmodSync must appear in the syncToFs function, not just imports
    const syncToFsBody = syncSrc.slice(
      syncSrc.indexOf('export async function syncToFs'),
      syncSrc.indexOf('export async function syncFromFs'),
    );
    expect(syncToFsBody).toContain('chmodSync(');
    expect(syncToFsBody).toContain('row.mode & 0o7777');
  });
});
