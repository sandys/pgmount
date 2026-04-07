import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { basename, extname, join, relative } from 'node:path';
import type { MemoryChunk, MemorySourceFile, MemorySourceKind, ProjectContext } from './types.js';

const EXCLUDED_DIRS = new Set([
  '.git',
  '.agents',
  '.codex',
  '.gemini',
  'node_modules',
  'dist',
  'build',
  'coverage',
  '.venv',
  'target',
  '.next',
]);

const TEXT_EXTENSIONS = new Set([
  '.md', '.txt', '.rst',
  '.json', '.jsonc', '.yaml', '.yml', '.toml', '.ini', '.cfg', '.conf', '.env', '.properties',
  '.sh', '.bash', '.zsh', '.fish',
  '.ts', '.tsx', '.js', '.jsx', '.mjs', '.cjs',
  '.py', '.rs', '.go', '.java', '.kt', '.swift', '.rb', '.php',
  '.sql', '.graphql', '.proto',
  '.c', '.cc', '.cpp', '.h', '.hpp',
  '.css', '.html', '.xml',
]);

const TEXT_BASENAMES = new Set([
  'Dockerfile',
  'Makefile',
  'justfile',
  'Procfile',
  'README',
  'README.md',
  'CLAUDE.md',
  'MEMORY.md',
]);

const STOPWORDS = new Set([
  'a', 'an', 'and', 'are', 'as', 'at', 'be', 'by', 'for', 'from', 'how', 'if',
  'in', 'into', 'is', 'it', 'of', 'on', 'or', 'that', 'the', 'this', 'to', 'with',
]);

const MAX_FILE_BYTES = 128 * 1024;
const MAX_FILES = 1500;

function looksLikeText(buffer: Buffer): boolean {
  if (buffer.length === 0) return true;
  for (const byte of buffer) {
    if (byte === 0) return false;
  }
  return true;
}

function shouldIncludeFile(path: string, size: number): boolean {
  if (size > MAX_FILE_BYTES) return false;
  return TEXT_EXTENSIONS.has(extname(path).toLowerCase()) || TEXT_BASENAMES.has(basename(path));
}

function classifySource(absPath: string, ctx: ProjectContext): MemorySourceKind {
  if (absPath.startsWith(ctx.memoryDir)) return 'memory';
  if (absPath.endsWith('/CLAUDE.md') || absPath.includes('/.claude/rules/')) return 'instruction';

  const ext = extname(absPath).toLowerCase();
  if (ext === '.md' || ext === '.txt' || ext === '.rst') return 'doc';
  if (['.json', '.jsonc', '.yaml', '.yml', '.toml', '.ini', '.cfg', '.conf', '.env', '.properties'].includes(ext)) {
    return 'config';
  }
  return 'code';
}

function relPathFor(absPath: string, ctx: ProjectContext): string {
  if (absPath.startsWith(ctx.memoryDir)) {
    return `memory/${relative(ctx.memoryDir, absPath)}`;
  }
  const rel = relative(ctx.contentRoot, absPath);
  return rel && !rel.startsWith('..') ? rel : basename(absPath);
}

function fileExcerpt(text: string): string {
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && line !== '---' && !line.startsWith('```') && !line.startsWith('#'));
  return lines.slice(0, 4).join(' ').replace(/\s+/g, ' ').slice(0, 220);
}

export function tokenizeText(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^a-z0-9_./-]+/)
    .map((part) => part.trim())
    .filter((part) => part.length >= 2 && !STOPWORDS.has(part));
}

function chunkMarkdown(file: MemorySourceFile): MemoryChunk[] {
  const lines = file.content.split(/\r?\n/);
  const chunks: MemoryChunk[] = [];
  let currentTitle = basename(file.absPath);
  let currentLines: string[] = [];
  let chunkIndex = 0;

  const flush = () => {
    const text = currentLines.join('\n').trim();
    if (!text) return;
    chunks.push({
      ...file,
      chunkId: `${file.relPath}#${chunkIndex++}`,
      title: currentTitle,
      excerpt: fileExcerpt(text),
      tokenSet: new Set(tokenizeText(`${file.relPath}\n${currentTitle}\n${text}`)),
    });
    currentLines = [];
  };

  for (const line of lines) {
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    if (heading) {
      flush();
      currentTitle = heading[1].trim();
      currentLines.push(line);
    } else {
      currentLines.push(line);
    }
  }
  flush();

  if (chunks.length > 0) return chunks;
  return [{
    ...file,
    chunkId: `${file.relPath}#0`,
    title: basename(file.absPath),
    excerpt: fileExcerpt(file.content),
    tokenSet: new Set(tokenizeText(`${file.relPath}\n${file.content}`)),
  }];
}

function chunkPlainText(file: MemorySourceFile): MemoryChunk[] {
  const lines = file.content.split(/\r?\n/);
  const windowSize = 80;
  const overlap = 20;
  const chunks: MemoryChunk[] = [];

  if (lines.length <= windowSize) {
    return [{
      ...file,
      chunkId: `${file.relPath}#0`,
      title: basename(file.absPath),
      excerpt: fileExcerpt(file.content),
      tokenSet: new Set(tokenizeText(`${file.relPath}\n${file.content}`)),
    }];
  }

  let chunkIndex = 0;
  for (let start = 0; start < lines.length; start += windowSize - overlap) {
    const slice = lines.slice(start, start + windowSize);
    if (slice.length === 0) break;
    const text = slice.join('\n').trim();
    if (!text) continue;
    chunks.push({
      ...file,
      chunkId: `${file.relPath}#${chunkIndex++}`,
      title: `${basename(file.absPath)}:${start + 1}`,
      excerpt: fileExcerpt(text),
      tokenSet: new Set(tokenizeText(`${file.relPath}\n${text}`)),
    });
    if (start + windowSize >= lines.length) break;
  }
  return chunks;
}

export function chunkSourceFile(file: MemorySourceFile): MemoryChunk[] {
  const ext = extname(file.absPath).toLowerCase();
  if (ext === '.md' || ext === '.rst') {
    return chunkMarkdown(file);
  }
  return chunkPlainText(file);
}

function maybeAddFile(absPath: string, ctx: ProjectContext, seen: Set<string>, out: MemorySourceFile[]): void {
  if (seen.has(absPath)) return;

  let st;
  try {
    st = statSync(absPath);
  } catch {
    return;
  }
  if (!st.isFile() || !shouldIncludeFile(absPath, st.size)) return;

  let buffer: Buffer;
  try {
    buffer = readFileSync(absPath);
  } catch {
    return;
  }
  if (!looksLikeText(buffer)) return;

  seen.add(absPath);
  out.push({
    absPath,
    relPath: relPathFor(absPath, ctx),
    kind: classifySource(absPath, ctx),
    content: buffer.toString('utf8'),
    mtimeMs: st.mtimeMs,
  });
}

function walkDir(dirPath: string, ctx: ProjectContext, seen: Set<string>, out: MemorySourceFile[]): void {
  let names: string[];
  try {
    names = readdirSync(dirPath).sort((a, b) => a.localeCompare(b));
  } catch {
    return;
  }

  for (const name of names) {
    if (out.length >= MAX_FILES) return;
    const fullPath = join(dirPath, name);
    let st;
    try {
      st = statSync(fullPath);
    } catch {
      continue;
    }

    if (st.isDirectory()) {
      if (EXCLUDED_DIRS.has(name)) continue;
      walkDir(fullPath, ctx, seen, out);
      continue;
    }

    maybeAddFile(fullPath, ctx, seen, out);
  }
}

export function collectMemorySourceFiles(ctx: ProjectContext): MemorySourceFile[] {
  const seen = new Set<string>();
  const out: MemorySourceFile[] = [];

  if (existsSync(ctx.memoryDir)) {
    for (const name of readdirSync(ctx.memoryDir).sort((a, b) => a.localeCompare(b))) {
      if (!name.endsWith('.md')) continue;
      maybeAddFile(join(ctx.memoryDir, name), ctx, seen, out);
    }
  }

  const explicitFiles = [
    join(ctx.contentRoot, 'CLAUDE.md'),
    join(ctx.contentRoot, '.claude', 'CLAUDE.md'),
  ];
  for (const absPath of explicitFiles) {
    if (existsSync(absPath)) {
      maybeAddFile(absPath, ctx, seen, out);
    }
  }

  walkDir(ctx.contentRoot, ctx, seen, out);
  return out;
}

export function collectMemoryChunks(ctx: ProjectContext): MemoryChunk[] {
  return collectMemorySourceFiles(ctx).flatMap((file) => chunkSourceFile(file));
}
