import { execFileSync } from 'node:child_process';
import type { MemoryChunk, RankedMemoryChunk } from './types.js';
import { tokenizeText } from './collect.js';

function countOccurrences(haystack: string, needle: string): number {
  if (!needle) return 0;
  let count = 0;
  let idx = haystack.indexOf(needle);
  while (idx !== -1) {
    count++;
    idx = haystack.indexOf(needle, idx + needle.length);
  }
  return count;
}

function kindBoost(kind: MemoryChunk['kind']): number {
  switch (kind) {
    case 'memory': return 12;
    case 'instruction': return 10;
    case 'doc': return 8;
    case 'config': return 4;
    case 'code': return 1;
  }
}

export function readDirtyPathSet(contentRoot: string): Set<string> {
  try {
    const output = execFileSync('git', ['-C', contentRoot, 'status', '--porcelain', '--untracked-files=all'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });

    const dirty = new Set<string>();
    for (const line of output.split('\n')) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      const payload = trimmed.slice(3);
      const path = payload.includes(' -> ') ? payload.split(' -> ').at(-1) ?? payload : payload;
      dirty.add(path);
    }
    return dirty;
  } catch {
    return new Set<string>();
  }
}

export function rankMemoryChunks(
  chunks: MemoryChunk[],
  query: string,
  opts?: { now?: Date; dirtyPaths?: Set<string> },
): RankedMemoryChunk[] {
  const normalizedQuery = query.trim().toLowerCase();
  const queryTerms = tokenizeText(query);
  const nowMs = opts?.now?.getTime() ?? Date.now();
  const dirtyPaths = opts?.dirtyPaths ?? new Set<string>();

  const ranked = chunks.map((chunk) => {
    let score = 0;
    const reasons: string[] = [];
    const haystack = `${chunk.relPath}\n${chunk.title}\n${chunk.content}`.toLowerCase();

    if (normalizedQuery && haystack.includes(normalizedQuery)) {
      score += 20;
      reasons.push('exact-query');
    }

    let matchedTerms = 0;
    for (const term of queryTerms) {
      if (chunk.tokenSet.has(term)) {
        matchedTerms++;
        score += 4;
      }
      const occurrences = Math.min(3, countOccurrences(haystack, term));
      if (occurrences > 0) {
        score += occurrences * 1.5;
      }
      if (chunk.relPath.toLowerCase().includes(term)) {
        score += 2;
      }
    }
    if (matchedTerms > 0) {
      reasons.push(`terms:${matchedTerms}`);
    }

    const baseBoost = kindBoost(chunk.kind);
    score += baseBoost;
    reasons.push(`kind:${chunk.kind}`);

    const ageDays = Math.max(0, (nowMs - chunk.mtimeMs) / (24 * 60 * 60 * 1000));
    const recencyBoost = Math.max(0, 4 - ageDays / 14);
    if (recencyBoost > 0) {
      score += recencyBoost;
      reasons.push(`fresh:${recencyBoost.toFixed(1)}`);
    }

    if (dirtyPaths.has(chunk.relPath)) {
      score += 3;
      reasons.push('dirty');
    }

    if (chunk.kind === 'memory' && queryTerms.some((term) => chunk.relPath.toLowerCase().includes(term))) {
      score += 3;
      reasons.push('memory-path-match');
    }

    return {
      ...chunk,
      score,
      reasons,
    };
  });

  return ranked.sort((a, b) => (
    b.score - a.score ||
    b.mtimeMs - a.mtimeMs ||
    a.relPath.localeCompare(b.relPath) ||
    a.chunkId.localeCompare(b.chunkId)
  ));
}

export function selectTopChunks(
  ranked: RankedMemoryChunk[],
  opts?: { limit?: number; maxPerFile?: number },
): RankedMemoryChunk[] {
  const limit = opts?.limit ?? 8;
  const maxPerFile = opts?.maxPerFile ?? 2;
  const selected: RankedMemoryChunk[] = [];
  const perFile = new Map<string, number>();

  for (const chunk of ranked) {
    const count = perFile.get(chunk.absPath) ?? 0;
    if (count >= maxPerFile) continue;
    selected.push(chunk);
    perFile.set(chunk.absPath, count + 1);
    if (selected.length >= limit) break;
  }

  return selected;
}
