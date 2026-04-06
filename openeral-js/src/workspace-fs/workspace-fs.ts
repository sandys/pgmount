import pg from 'pg';
import {
  getFile,
  listChildren,
  createFile,
  updateFileContent,
  updateFileAttrs,
  deleteFile,
  deleteDirectory,
  deleteTree,
  renameFile,
  renameTree,
} from '../db/workspace-queries.js';
import type { WorkspaceFile } from '../db/types.js';

// ---------------------------------------------------------------------------
// Minimal IFileSystem-compatible stat type (no dependency on just-bash)
// ---------------------------------------------------------------------------

interface FsStat {
  isFile: boolean;
  isDirectory: boolean;
  isSymbolicLink: boolean;
  mode: number;
  size: number;
  mtime: Date;
}

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

function fsError(code: string, message: string): Error {
  const err = new Error(message);
  (err as NodeJS.ErrnoException).code = code;
  return err;
}

// ---------------------------------------------------------------------------
// Path utilities
// ---------------------------------------------------------------------------

/** Ensure the path starts with `/`, strip trailing `/` (except root). */
function normalizePath(p: string): string {
  let out = p.startsWith('/') ? p : `/${p}`;
  if (out.length > 1 && out.endsWith('/')) {
    out = out.slice(0, -1);
  }
  return out;
}

/** Split a normalized path into `[parentPath, name]`. */
function splitPath(path: string): [string, string] {
  if (path === '/') return ['', ''];
  const idx = path.lastIndexOf('/');
  if (idx === 0) return ['/', path.slice(1)];
  return [path.slice(0, idx), path.slice(idx + 1)];
}

/** Resolve `.` and `..` segments from a normalized absolute path. */
function resolveSegments(p: string): string {
  const parts = p.split('/');
  const stack: string[] = [];
  for (const seg of parts) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') {
      stack.pop();
    } else {
      stack.push(seg);
    }
  }
  return '/' + stack.join('/');
}

/** Current time as a bigint of nanoseconds since epoch. */
function nowNs(): bigint {
  return BigInt(Date.now()) * 1_000_000n;
}

/** Convert nanosecond epoch to Date. */
function nsToDate(ns: bigint): Date {
  return new Date(Number(ns / 1_000_000n));
}

// ---------------------------------------------------------------------------
// WorkspaceFs
// ---------------------------------------------------------------------------

export class WorkspaceFs {
  private pool: pg.Pool;
  private workspaceId: string;

  constructor(pool: pg.Pool, workspaceId: string) {
    this.pool = pool;
    this.workspaceId = workspaceId;
  }

  // -----------------------------------------------------------------------
  // Internal helpers
  // -----------------------------------------------------------------------

  private fileToStat(f: WorkspaceFile): FsStat {
    return {
      isFile: !f.isDir,
      isDirectory: f.isDir,
      isSymbolicLink: false,
      mode: f.mode,
      size: f.size,
      mtime: nsToDate(f.mtimeNs),
    };
  }

  /** Ensure every ancestor directory of `path` exists, creating as needed. */
  private async ensureParents(path: string): Promise<void> {
    const segments = path.split('/').filter(Boolean);
    // Walk from the root down to (but not including) the leaf.
    let current = '';
    for (let i = 0; i < segments.length - 1; i++) {
      current += '/' + segments[i];
      try {
        await getFile(this.pool, this.workspaceId, current);
      } catch {
        const [parentPath, name] = splitPath(current);
        const now = nowNs();
        const dir: WorkspaceFile = {
          workspaceId: this.workspaceId,
          path: current,
          parentPath,
          name,
          isDir: true,
          content: null,
          mode: 0o40755,
          size: 0,
          mtimeNs: now,
          ctimeNs: now,
          atimeNs: now,
          nlink: 2,
          uid: 1000,
          gid: 1000,
        };
        try {
          await createFile(this.pool, dir);
        } catch {
          // may race with another caller — ignore EEXIST
        }
      }
    }
  }

  // -----------------------------------------------------------------------
  // IFileSystem methods
  // -----------------------------------------------------------------------

  async readFile(
    path: string,
    options?: { encoding?: BufferEncoding },
  ): Promise<string> {
    const norm = normalizePath(path);
    let file: WorkspaceFile;
    try {
      file = await getFile(this.pool, this.workspaceId, norm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, open '${path}'`);
    }
    if (file.isDir) {
      throw fsError('EISDIR', `EISDIR: illegal operation on a directory, read '${path}'`);
    }
    const buf = file.content ?? Buffer.alloc(0);
    const encoding = options?.encoding ?? 'utf8';
    return Buffer.from(buf).toString(encoding);
  }

  async readFileBuffer(path: string): Promise<Uint8Array> {
    const norm = normalizePath(path);
    let file: WorkspaceFile;
    try {
      file = await getFile(this.pool, this.workspaceId, norm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, open '${path}'`);
    }
    if (file.isDir) {
      throw fsError('EISDIR', `EISDIR: illegal operation on a directory, read '${path}'`);
    }
    const buf = file.content ?? Buffer.alloc(0);
    return new Uint8Array(buf);
  }

  async writeFile(
    path: string,
    content: string | Uint8Array,
    options?: { encoding?: BufferEncoding; mode?: number },
  ): Promise<void> {
    const norm = normalizePath(path);
    const buf =
      typeof content === 'string'
        ? Buffer.from(content, options?.encoding ?? 'utf8')
        : Buffer.from(content);

    let existing: WorkspaceFile | null = null;
    try {
      existing = await getFile(this.pool, this.workspaceId, norm);
    } catch {
      // does not exist yet
    }

    if (existing) {
      if (existing.isDir) {
        throw fsError('EISDIR', `EISDIR: illegal operation on a directory, write '${path}'`);
      }
      await updateFileContent(
        this.pool,
        this.workspaceId,
        norm,
        buf,
        nowNs(),
      );
    } else {
      await this.ensureParents(norm);
      const [parentPath, name] = splitPath(norm);
      const now = nowNs();
      const file: WorkspaceFile = {
        workspaceId: this.workspaceId,
        path: norm,
        parentPath,
        name,
        isDir: false,
        content: buf,
        mode: options?.mode ?? 0o100644,
        size: buf.length,
        mtimeNs: now,
        ctimeNs: now,
        atimeNs: now,
        nlink: 1,
        uid: 1000,
        gid: 1000,
      };
      await createFile(this.pool, file);
    }
  }

  async appendFile(
    path: string,
    content: string | Uint8Array,
    options?: { encoding?: BufferEncoding },
  ): Promise<void> {
    const norm = normalizePath(path);
    const appendBuf =
      typeof content === 'string'
        ? Buffer.from(content, options?.encoding ?? 'utf8')
        : Buffer.from(content);

    let existing: WorkspaceFile | null = null;
    try {
      existing = await getFile(this.pool, this.workspaceId, norm);
    } catch {
      // does not exist — create it
    }

    if (existing) {
      if (existing.isDir) {
        throw fsError('EISDIR', `EISDIR: illegal operation on a directory, write '${path}'`);
      }
      const prev = existing.content ? Buffer.from(existing.content) : Buffer.alloc(0);
      const merged = Buffer.concat([prev, appendBuf]);
      await updateFileContent(
        this.pool,
        this.workspaceId,
        norm,
        merged,
        nowNs(),
      );
    } else {
      // New file — just write the appended content.
      await this.writeFile(path, appendBuf);
    }
  }

  async exists(path: string): Promise<boolean> {
    const norm = normalizePath(path);
    try {
      await getFile(this.pool, this.workspaceId, norm);
      return true;
    } catch {
      return false;
    }
  }

  async stat(path: string): Promise<FsStat> {
    const norm = normalizePath(path);
    let file: WorkspaceFile;
    try {
      file = await getFile(this.pool, this.workspaceId, norm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, stat '${path}'`);
    }
    return this.fileToStat(file);
  }

  async mkdir(
    path: string,
    options?: { recursive?: boolean; mode?: number },
  ): Promise<void> {
    const norm = normalizePath(path);

    if (options?.recursive) {
      const segments = norm.split('/').filter(Boolean);
      let current = '';
      for (const seg of segments) {
        current += '/' + seg;
        let exists = false;
        try {
          const f = await getFile(this.pool, this.workspaceId, current);
          if (!f.isDir) {
            throw fsError('ENOTDIR', `ENOTDIR: not a directory, mkdir '${path}'`);
          }
          exists = true;
        } catch (e) {
          if ((e as NodeJS.ErrnoException).code === 'ENOTDIR') throw e;
          // not found — create it
        }
        if (!exists) {
          const [parentPath, name] = splitPath(current);
          const now = nowNs();
          const dir: WorkspaceFile = {
            workspaceId: this.workspaceId,
            path: current,
            parentPath,
            name,
            isDir: true,
            content: null,
            mode: options?.mode ?? 0o40755,
            size: 0,
            mtimeNs: now,
            ctimeNs: now,
            atimeNs: now,
            nlink: 2,
            uid: 1000,
            gid: 1000,
          };
          try {
            await createFile(this.pool, dir);
          } catch {
            // race — already created, fine
          }
        }
      }
    } else {
      // Non-recursive: parent must exist.
      const [parentPath, name] = splitPath(norm);
      if (parentPath !== '') {
        try {
          const parent = await getFile(this.pool, this.workspaceId, parentPath);
          if (!parent.isDir) {
            throw fsError('ENOTDIR', `ENOTDIR: not a directory, mkdir '${path}'`);
          }
        } catch (e) {
          if ((e as NodeJS.ErrnoException).code === 'ENOTDIR') throw e;
          throw fsError('ENOENT', `ENOENT: no such file or directory, mkdir '${path}'`);
        }
      }

      // Check if already exists.
      try {
        await getFile(this.pool, this.workspaceId, norm);
        throw fsError('EEXIST', `EEXIST: file already exists, mkdir '${path}'`);
      } catch (e) {
        if ((e as NodeJS.ErrnoException).code === 'EEXIST') throw e;
        // not found — good, create it
      }

      const now = nowNs();
      const dir: WorkspaceFile = {
        workspaceId: this.workspaceId,
        path: norm,
        parentPath,
        name,
        isDir: true,
        content: null,
        mode: options?.mode ?? 0o40755,
        size: 0,
        mtimeNs: now,
        ctimeNs: now,
        atimeNs: now,
        nlink: 2,
        uid: 1000,
        gid: 1000,
      };
      await createFile(this.pool, dir);
    }
  }

  async readdir(path: string): Promise<string[]> {
    const norm = normalizePath(path);
    let file: WorkspaceFile;
    try {
      file = await getFile(this.pool, this.workspaceId, norm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, scandir '${path}'`);
    }
    if (!file.isDir) {
      throw fsError('ENOTDIR', `ENOTDIR: not a directory, scandir '${path}'`);
    }
    const children = await listChildren(this.pool, this.workspaceId, norm);
    return children.map((c) => c.name);
  }

  async rm(
    path: string,
    options?: { recursive?: boolean; force?: boolean },
  ): Promise<void> {
    const norm = normalizePath(path);

    let file: WorkspaceFile;
    try {
      file = await getFile(this.pool, this.workspaceId, norm);
    } catch {
      if (options?.force) return;
      throw fsError('ENOENT', `ENOENT: no such file or directory, rm '${path}'`);
    }

    if (file.isDir) {
      if (options?.recursive) {
        // Delete entire subtree then the directory itself.
        await deleteTree(this.pool, this.workspaceId, norm);
        await deleteDirectory(this.pool, this.workspaceId, norm).catch(() => {
          // may already be gone
        });
      } else {
        // Non-recursive: directory must be empty.
        const children = await listChildren(this.pool, this.workspaceId, norm);
        if (children.length > 0) {
          throw fsError('ENOTEMPTY', `ENOTEMPTY: directory not empty, rm '${path}'`);
        }
        await deleteDirectory(this.pool, this.workspaceId, norm);
      }
    } else {
      await deleteFile(this.pool, this.workspaceId, norm);
    }
  }

  async cp(
    src: string,
    dest: string,
    options?: { recursive?: boolean },
  ): Promise<void> {
    const srcNorm = normalizePath(src);
    const destNorm = normalizePath(dest);

    let srcFile: WorkspaceFile;
    try {
      srcFile = await getFile(this.pool, this.workspaceId, srcNorm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, cp '${src}'`);
    }

    if (srcFile.isDir) {
      if (!options?.recursive) {
        throw fsError('EISDIR', `EISDIR: illegal operation on a directory, cp '${src}'`);
      }
      await this.copyDirRecursive(srcNorm, destNorm);
    } else {
      await this.ensureParents(destNorm);
      const [parentPath, name] = splitPath(destNorm);
      const now = nowNs();

      let destExists = false;
      try {
        await getFile(this.pool, this.workspaceId, destNorm);
        destExists = true;
      } catch {
        // not found
      }

      if (destExists) {
        await updateFileContent(
          this.pool,
          this.workspaceId,
          destNorm,
          srcFile.content ? Buffer.from(srcFile.content) : Buffer.alloc(0),
          now,
        );
      } else {
        const newFile: WorkspaceFile = {
          workspaceId: this.workspaceId,
          path: destNorm,
          parentPath,
          name,
          isDir: false,
          content: srcFile.content ? Buffer.from(srcFile.content) : Buffer.alloc(0),
          mode: srcFile.mode,
          size: srcFile.size,
          mtimeNs: now,
          ctimeNs: now,
          atimeNs: now,
          nlink: 1,
          uid: srcFile.uid,
          gid: srcFile.gid,
        };
        await createFile(this.pool, newFile);
      }
    }
  }

  private async copyDirRecursive(srcDir: string, destDir: string): Promise<void> {
    // Ensure dest directory exists.
    await this.mkdir(destDir, { recursive: true });

    const children = await listChildren(this.pool, this.workspaceId, srcDir);
    for (const child of children) {
      const childSrc = child.path;
      const childDest =
        destDir === '/'
          ? `/${child.name}`
          : `${destDir}/${child.name}`;

      if (child.isDir) {
        await this.copyDirRecursive(childSrc, childDest);
      } else {
        const [parentPath, name] = splitPath(childDest);
        const now = nowNs();
        const newFile: WorkspaceFile = {
          workspaceId: this.workspaceId,
          path: childDest,
          parentPath,
          name,
          isDir: false,
          content: child.content ? Buffer.from(child.content) : Buffer.alloc(0),
          mode: child.mode,
          size: child.size,
          mtimeNs: now,
          ctimeNs: now,
          atimeNs: now,
          nlink: 1,
          uid: child.uid,
          gid: child.gid,
        };
        try {
          await createFile(this.pool, newFile);
        } catch {
          // dest file already exists — overwrite
          await updateFileContent(
            this.pool,
            this.workspaceId,
            childDest,
            newFile.content!,
            now,
          );
        }
      }
    }
  }

  async mv(src: string, dest: string): Promise<void> {
    const srcNorm = normalizePath(src);
    const destNorm = normalizePath(dest);

    let srcFile: WorkspaceFile;
    try {
      srcFile = await getFile(this.pool, this.workspaceId, srcNorm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, mv '${src}'`);
    }

    await this.ensureParents(destNorm);
    const [newParentPath, newName] = splitPath(destNorm);

    await renameFile(
      this.pool,
      this.workspaceId,
      srcNorm,
      destNorm,
      newParentPath,
      newName,
    );

    if (srcFile.isDir) {
      await renameTree(this.pool, this.workspaceId, srcNorm, destNorm);
    }
  }

  resolvePath(base: string, path: string): string {
    if (path.startsWith('/')) {
      return resolveSegments(path);
    }
    const combined = base.endsWith('/')
      ? base + path
      : base + '/' + path;
    return resolveSegments(combined);
  }

  getAllPaths(): string[] {
    // Too expensive for a full DB scan — return empty array.
    return [];
  }

  async chmod(path: string, mode: number): Promise<void> {
    const norm = normalizePath(path);
    try {
      await getFile(this.pool, this.workspaceId, norm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, chmod '${path}'`);
    }
    await updateFileAttrs(this.pool, this.workspaceId, norm, { mode });
  }

  async symlink(_target: string, _linkPath: string): Promise<void> {
    throw fsError('ENOTSUP', 'ENOTSUP: symlinks are not supported');
  }

  async link(_existingPath: string, _newPath: string): Promise<void> {
    throw fsError('ENOTSUP', 'ENOTSUP: hard links are not supported');
  }

  async readlink(_path: string): Promise<string> {
    throw fsError('ENOTSUP', 'ENOTSUP: symlinks are not supported');
  }

  async lstat(path: string): Promise<FsStat> {
    // No symlink support — lstat is identical to stat.
    return this.stat(path);
  }

  async realpath(path: string): Promise<string> {
    const resolved = resolveSegments(normalizePath(path));
    // Verify the path exists.
    try {
      await getFile(this.pool, this.workspaceId, resolved);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, realpath '${path}'`);
    }
    return resolved;
  }

  async utimes(path: string, _atime: Date, mtime: Date): Promise<void> {
    const norm = normalizePath(path);
    try {
      await getFile(this.pool, this.workspaceId, norm);
    } catch {
      throw fsError('ENOENT', `ENOENT: no such file or directory, utimes '${path}'`);
    }
    const mtimeNs = BigInt(mtime.getTime()) * 1_000_000n;
    const atimeNs = BigInt(_atime.getTime()) * 1_000_000n;
    await updateFileAttrs(this.pool, this.workspaceId, norm, { mtimeNs, atimeNs });
  }
}
