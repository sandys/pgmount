import { describe, expect, it } from 'vitest';
import { rankMemoryChunks, selectTopChunks } from './rank.js';
import type { MemoryChunk } from './types.js';

function makeChunk(overrides: Partial<MemoryChunk>): MemoryChunk {
  const base: MemoryChunk = {
    absPath: '/repo/README.md',
    relPath: 'README.md',
    kind: 'doc',
    content: 'Build with pnpm install and pnpm check.',
    mtimeMs: Date.now(),
    chunkId: 'README.md#0',
    title: 'Build and test',
    excerpt: 'Build with pnpm install and pnpm check.',
    tokenSet: new Set(['build', 'pnpm', 'install', 'check', 'test']),
  };
  return { ...base, ...overrides };
}

describe('memory ranking', () => {
  it('prefers fresher memory/instruction chunks over stale generic docs', () => {
    const now = new Date('2026-04-07T00:00:00Z');
    const recentMemory = makeChunk({
      absPath: '/memory/build.md',
      relPath: 'memory/build.md',
      kind: 'memory',
      mtimeMs: now.getTime() - 2 * 60 * 60 * 1000,
      tokenSet: new Set(['build', 'pnpm', 'install', 'check']),
    });
    const staleDoc = makeChunk({
      absPath: '/repo/docs.txt',
      relPath: 'docs.txt',
      kind: 'doc',
      mtimeMs: now.getTime() - 90 * 24 * 60 * 60 * 1000,
      tokenSet: new Set(['build', 'install']),
    });

    const ranked = rankMemoryChunks([staleDoc, recentMemory], 'build install', { now });
    expect(ranked[0].relPath).toBe('memory/build.md');
  });

  it('boosts dirty files and limits selected chunks per file', () => {
    const a = makeChunk({
      absPath: '/repo/README.md',
      relPath: 'README.md',
      chunkId: 'README.md#0',
      title: 'README',
    });
    const b = makeChunk({
      absPath: '/repo/README.md',
      relPath: 'README.md',
      chunkId: 'README.md#1',
      title: 'README second chunk',
    });
    const c = makeChunk({
      absPath: '/repo/cli.ts',
      relPath: 'src/cli.ts',
      chunkId: 'src/cli.ts#0',
      kind: 'code',
      title: 'CLI',
      tokenSet: new Set(['build', 'install', 'memory']),
    });

    const ranked = rankMemoryChunks([a, b, c], 'memory build', {
      now: new Date('2026-04-07T00:00:00Z'),
      dirtyPaths: new Set(['src/cli.ts']),
    });

    const cliChunk = ranked.find((chunk) => chunk.relPath === 'src/cli.ts');
    expect(cliChunk?.reasons).toContain('dirty');

    const top = selectTopChunks(ranked, { limit: 3, maxPerFile: 1 });
    expect(top).toHaveLength(2);
    expect(top.filter((chunk) => chunk.relPath === 'README.md')).toHaveLength(1);
    expect(top.some((chunk) => chunk.relPath === 'src/cli.ts')).toBe(true);
  });
});
