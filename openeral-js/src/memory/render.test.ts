import { describe, expect, it } from 'vitest';
import { renderMemoryIndex, renderTopicFile, slugifyQuery } from './render.js';
import type { RankedMemoryChunk } from './types.js';

function makeChunk(overrides: Partial<RankedMemoryChunk>): RankedMemoryChunk {
  return {
    absPath: '/repo/README.md',
    relPath: 'README.md',
    kind: 'doc',
    content: [
      '# Build',
      'pnpm install',
      'pnpm check',
      '- Never skip integration tests',
    ].join('\n'),
    mtimeMs: Date.now(),
    chunkId: 'README.md#0',
    title: 'Build',
    excerpt: 'pnpm install pnpm check',
    tokenSet: new Set(['build', 'pnpm', 'install', 'check', 'never']),
    score: 42,
    reasons: ['exact-query'],
    ...overrides,
  };
}

describe('memory rendering', () => {
  it('renders frontmatter and deterministic sections', () => {
    const doc = renderTopicFile({
      filename: 'build-and-test.md',
      name: 'Build and test',
      description: 'Repeated build commands',
      type: 'workflow',
    }, [makeChunk()]);

    expect(doc).toBeDefined();
    expect(doc!.content).toContain('name: "Build and test"');
    expect(doc!.content).toContain('## Key Facts');
    expect(doc!.content).toContain('## Commands');
    expect(doc!.content).toContain('## Pitfalls');
    expect(doc!.content).toContain('`README.md`');
  });

  it('renders a focus fallback when no ranked material exists', () => {
    const doc = renderTopicFile({
      filename: 'focus-openshell.md',
      name: 'Focus: openshell',
      description: 'Focus document',
      type: 'focus',
    }, [], { query: 'openshell proxy' });

    expect(doc).toBeDefined();
    expect(doc!.content).toContain('Query: `openshell proxy`');
    expect(doc!.content).toContain('No strong matches found yet');
  });

  it('renders a compact MEMORY.md index', () => {
    const index = renderMemoryIndex([
      { name: 'project-overview.md', description: 'Goals and architecture' },
      { name: 'build-and-test.md', description: 'Commands' },
    ]);

    expect(index).toContain('[project-overview.md](project-overview.md)');
    expect(index).toContain('[build-and-test.md](build-and-test.md)');
  });

  it('slugifies focus queries into filenames', () => {
    expect(slugifyQuery('OpenShell proxy / memory refresh')).toBe('openshell-proxy-memory-refresh');
  });
});
