import type { DbPool } from './pool.js';
import type { WorkspaceFile } from './types.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Normalize a path: ensure it starts with /, remove trailing slash. */
export function normalizePath(path: string): string {
  let p = path.startsWith('/') ? path : `/${path}`;
  if (p.length > 1 && p.endsWith('/')) {
    p = p.slice(0, -1);
  }
  return p;
}

/** Split a path into [parentPath, name]. */
export function splitPath(path: string): [string, string] {
  if (path === '/') {
    return ['', ''];
  }
  const idx = path.lastIndexOf('/');
  if (idx === 0) {
    return ['/', path.slice(1)];
  }
  if (idx === -1) {
    return ['/', path];
  }
  return [path.slice(0, idx), path.slice(idx + 1)];
}

/** Current wall-clock time in nanoseconds since the Unix epoch. */
export function nowNs(): bigint {
  // process.hrtime.bigint() is monotonic, not wall-clock.
  // Use Date.now() (milliseconds) and convert to nanoseconds.
  return BigInt(Date.now()) * 1_000_000n;
}

// ---------------------------------------------------------------------------
// Row mapper
// ---------------------------------------------------------------------------

function rowToWorkspaceFile(r: Record<string, unknown>): WorkspaceFile {
  return {
    workspaceId: r.workspace_id as string,
    path: r.path as string,
    parentPath: r.parent_path as string,
    name: r.name as string,
    isDir: r.is_dir as boolean,
    content: r.content != null ? Buffer.from(r.content as Buffer) : null,
    mode: Number(r.mode),
    size: Number(r.size),
    mtimeNs: BigInt(r.mtime_ns as string),
    ctimeNs: BigInt(r.ctime_ns as string),
    atimeNs: BigInt(r.atime_ns as string),
    nlink: Number(r.nlink),
    uid: Number(r.uid),
    gid: Number(r.gid),
  };
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/** Get a file or directory by path within a workspace. */
export async function getFile(
  pool: DbPool,
  workspaceId: string,
  path: string,
): Promise<WorkspaceFile> {
  const { rows } = await pool.query(
    `SELECT workspace_id, path, parent_path, name, is_dir, content, mode, size, \
     mtime_ns, ctime_ns, atime_ns, nlink, uid, gid \
     FROM _openeral.workspace_files \
     WHERE workspace_id = $1 AND path = $2`,
    [workspaceId, path],
  );
  if (rows.length === 0) {
    throw new Error('File not found');
  }
  return rowToWorkspaceFile(rows[0]);
}

/** List children of a directory. */
export async function listChildren(
  pool: DbPool,
  workspaceId: string,
  parentPath: string,
): Promise<WorkspaceFile[]> {
  const { rows } = await pool.query(
    `SELECT workspace_id, path, parent_path, name, is_dir, content, mode, size, \
     mtime_ns, ctime_ns, atime_ns, nlink, uid, gid \
     FROM _openeral.workspace_files \
     WHERE workspace_id = $1 AND parent_path = $2 \
     ORDER BY name`,
    [workspaceId, parentPath],
  );
  return rows.map(rowToWorkspaceFile);
}

/** Create a new file or directory. */
export async function createFile(
  pool: DbPool,
  file: WorkspaceFile,
): Promise<void> {
  // Check if already exists
  const { rows: existing } = await pool.query(
    `SELECT 1 FROM _openeral.workspace_files WHERE workspace_id = $1 AND path = $2`,
    [file.workspaceId, file.path],
  );
  if (existing.length > 0) {
    throw new Error('File already exists');
  }

  await pool.query(
    `INSERT INTO _openeral.workspace_files \
     (workspace_id, path, parent_path, name, is_dir, content, mode, size, \
      mtime_ns, ctime_ns, atime_ns, nlink, uid, gid) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)`,
    [
      file.workspaceId,
      file.path,
      file.parentPath,
      file.name,
      file.isDir,
      file.content,
      file.mode,
      file.size,
      file.mtimeNs.toString(),
      file.ctimeNs.toString(),
      file.atimeNs.toString(),
      file.nlink,
      file.uid,
      file.gid,
    ],
  );
}

/** Update file content and size (used by flush/release). */
export async function updateFileContent(
  pool: DbPool,
  workspaceId: string,
  path: string,
  content: Buffer,
  mtimeNs: bigint,
): Promise<void> {
  const size = content.length;
  await pool.query(
    `UPDATE _openeral.workspace_files \
     SET content = $3, size = $4, mtime_ns = $5 \
     WHERE workspace_id = $1 AND path = $2`,
    [workspaceId, path, content, size, mtimeNs.toString()],
  );
}

/** Update file metadata (mode, size for truncate, mtime, atime). */
export async function updateFileAttrs(
  pool: DbPool,
  workspaceId: string,
  path: string,
  opts: {
    mode?: number;
    size?: number;
    mtimeNs?: bigint;
    atimeNs?: bigint;
  },
): Promise<WorkspaceFile> {
  const now = nowNs();

  if (opts.mode != null) {
    await pool.query(
      `UPDATE _openeral.workspace_files SET mode = $3, ctime_ns = $4 \
       WHERE workspace_id = $1 AND path = $2`,
      [workspaceId, path, opts.mode, now.toString()],
    );
  }

  if (opts.size != null) {
    // Truncation: if new size < current size, trim content
    await pool.query(
      `UPDATE _openeral.workspace_files \
       SET size = $3, \
           content = CASE WHEN $3 = 0 THEN '\\x'::bytea \
                     ELSE substring(COALESCE(content, '\\x'::bytea) FROM 1 FOR $3::integer) END, \
           mtime_ns = $4, ctime_ns = $4 \
       WHERE workspace_id = $1 AND path = $2`,
      [workspaceId, path, opts.size, now.toString()],
    );
  }

  if (opts.mtimeNs != null) {
    await pool.query(
      `UPDATE _openeral.workspace_files SET mtime_ns = $3 \
       WHERE workspace_id = $1 AND path = $2`,
      [workspaceId, path, opts.mtimeNs.toString()],
    );
  }

  if (opts.atimeNs != null) {
    await pool.query(
      `UPDATE _openeral.workspace_files SET atime_ns = $3 \
       WHERE workspace_id = $1 AND path = $2`,
      [workspaceId, path, opts.atimeNs.toString()],
    );
  }

  return getFile(pool, workspaceId, path);
}

/** Delete a file (not a directory). */
export async function deleteFile(
  pool: DbPool,
  workspaceId: string,
  path: string,
): Promise<void> {
  const result = await pool.query(
    `DELETE FROM _openeral.workspace_files WHERE workspace_id = $1 AND path = $2 AND is_dir = false`,
    [workspaceId, path],
  );
  if (result.rowCount === 0) {
    throw new Error('File not found');
  }
}

/** Delete an empty directory. */
export async function deleteDirectory(
  pool: DbPool,
  workspaceId: string,
  path: string,
): Promise<void> {
  // Check if directory has children
  const { rows: children } = await pool.query(
    `SELECT 1 FROM _openeral.workspace_files \
     WHERE workspace_id = $1 AND parent_path = $2 LIMIT 1`,
    [workspaceId, path],
  );
  if (children.length > 0) {
    throw new Error('Directory not empty');
  }

  const result = await pool.query(
    `DELETE FROM _openeral.workspace_files WHERE workspace_id = $1 AND path = $2 AND is_dir = true`,
    [workspaceId, path],
  );
  if (result.rowCount === 0) {
    throw new Error('Directory not found');
  }
}

/** Delete a directory and all its descendants. */
export async function deleteTree(
  pool: DbPool,
  workspaceId: string,
  path: string,
): Promise<void> {
  const like = `${path}/%`;
  await pool.query(
    `DELETE FROM _openeral.workspace_files WHERE workspace_id = $1 AND path LIKE $2`,
    [workspaceId, like],
  );
  await pool.query(
    `DELETE FROM _openeral.workspace_files WHERE workspace_id = $1 AND path = $2`,
    [workspaceId, path],
  );
}

/** Rename a single file or directory. */
export async function renameFile(
  pool: DbPool,
  workspaceId: string,
  oldPath: string,
  newPath: string,
  newParentPath: string,
  newName: string,
): Promise<void> {
  const now = nowNs();

  // Delete any existing file at newPath
  await pool.query(
    `DELETE FROM _openeral.workspace_files WHERE workspace_id = $1 AND path = $2`,
    [workspaceId, newPath],
  );

  // Rename the file itself
  await pool.query(
    `UPDATE _openeral.workspace_files \
     SET path = $3, parent_path = $4, name = $5, ctime_ns = $6 \
     WHERE workspace_id = $1 AND path = $2`,
    [workspaceId, oldPath, newPath, newParentPath, newName, now.toString()],
  );
}

/** Rename a directory tree (update all paths with the old prefix). */
export async function renameTree(
  pool: DbPool,
  workspaceId: string,
  oldPrefix: string,
  newPrefix: string,
): Promise<void> {
  const oldLike = `${oldPrefix}/%`;

  // Update all descendants: replace old prefix with new prefix in path and parent_path
  await pool.query(
    `UPDATE _openeral.workspace_files \
     SET path = $4 || substring(path FROM length($3) + 1), \
         parent_path = $4 || substring(parent_path FROM length($3) + 1) \
     WHERE workspace_id = $1 AND path LIKE $2`,
    [workspaceId, oldLike, oldPrefix, newPrefix],
  );
}

/**
 * Seed a workspace from its config (create auto_dirs and seed_files).
 * Ensures the root directory exists.
 */
export async function seedFromConfig(
  pool: DbPool,
  workspaceId: string,
  layout: { autoDirs: string[]; seedFiles: Record<string, string> },
): Promise<void> {
  const now = nowNs();
  const uid = 1000;
  const gid = 1000;

  // Ensure root directory exists
  const root: WorkspaceFile = {
    workspaceId,
    path: '/',
    parentPath: '',
    name: '',
    isDir: true,
    content: null,
    mode: 0o40755,
    size: 0,
    mtimeNs: now,
    ctimeNs: now,
    atimeNs: now,
    nlink: 2,
    uid,
    gid,
  };
  try {
    await createFile(pool, root);
  } catch {
    // Ignore if already exists
  }

  // Create auto_dirs
  for (const dirPath of layout.autoDirs) {
    const normalized = normalizePath(dirPath);
    const [parentPath, name] = splitPath(normalized);

    const dir: WorkspaceFile = {
      workspaceId,
      path: normalized,
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
      uid,
      gid,
    };
    try {
      await createFile(pool, dir);
    } catch {
      // Ignore if already exists
    }
  }

  // Create seed_files
  for (const [filePath, contentStr] of Object.entries(layout.seedFiles)) {
    const normalized = normalizePath(filePath);
    const [parentPath, name] = splitPath(normalized);
    const content = Buffer.from(contentStr, 'utf-8');

    const file: WorkspaceFile = {
      workspaceId,
      path: normalized,
      parentPath,
      name,
      isDir: false,
      content,
      mode: 0o100644,
      size: content.length,
      mtimeNs: now,
      ctimeNs: now,
      atimeNs: now,
      nlink: 1,
      uid,
      gid,
    };
    try {
      await createFile(pool, file);
    } catch {
      // Ignore if already exists
    }
  }
}
