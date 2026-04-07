import { describe, expect, it } from 'vitest';
import { deriveProjectRoots, slugifyClaudeProjectPath } from './resolve.js';

describe('memory project resolution', () => {
  it('slugifies Claude project paths like the native ~/.claude layout', () => {
    expect(slugifyClaudeProjectPath('/home/sss/Code/pgmount')).toBe('-home-sss-Code-pgmount');
  });

  it('uses git common dir for the shared memory key while keeping worktree content root', () => {
    const roots = deriveProjectRoots({
      cwd: '/home/sss/Code/pgmount/.worktrees/just-bash',
      gitShowTopLevel: '/home/sss/Code/pgmount/.worktrees/just-bash',
      gitCommonDir: '/home/sss/Code/pgmount/.git',
    });

    expect(roots.contentRoot).toBe('/home/sss/Code/pgmount/.worktrees/just-bash');
    expect(roots.memoryKeyRoot).toBe('/home/sss/Code/pgmount');
  });

  it('uses the explicit project root for scanning when provided', () => {
    const roots = deriveProjectRoots({
      cwd: '/tmp',
      explicitProjectRoot: '/tmp/project',
      gitShowTopLevel: '/tmp/project',
      gitCommonDir: '/tmp/project/.git',
    });

    expect(roots.contentRoot).toBe('/tmp/project');
    expect(roots.memoryKeyRoot).toBe('/tmp/project');
  });

  it('falls back to cwd outside git', () => {
    const roots = deriveProjectRoots({ cwd: '/tmp/plain-dir' });
    expect(roots.contentRoot).toBe('/tmp/plain-dir');
    expect(roots.memoryKeyRoot).toBe('/tmp/plain-dir');
  });
});
