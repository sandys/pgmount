/**
 * Bidirectional sync between PostgreSQL workspace_files and the real filesystem.
 *
 * syncToFs:    PostgreSQL → real filesystem (startup)
 * syncFromFs:  real filesystem → PostgreSQL (shutdown / on-change)
 * watchAndSync: continuous background sync via fs.watch
 */

import { mkdirSync, writeFileSync, readFileSync, readdirSync, statSync, chmodSync, existsSync, watch } from 'node:fs';
import { join, dirname } from 'node:path';
import type pg from 'pg';

function nowNs(): bigint {
  return BigInt(Date.now()) * 1_000_000n;
}

/**
 * Dump all workspace_files rows to a real directory.
 * Creates directories and writes file content, preserving stored modes.
 */
export async function syncToFs(
  pool: pg.Pool,
  workspaceId: string,
  targetDir: string,
): Promise<number> {
  const { rows } = await pool.query(
    `SELECT path, is_dir, content, mode FROM _openeral.workspace_files
     WHERE workspace_id = $1 ORDER BY path`,
    [workspaceId],
  );

  let count = 0;

  // Create directories first (sorted by path ensures parents before children)
  for (const row of rows) {
    if (!row.is_dir) continue;
    const fullPath = join(targetDir, row.path);
    mkdirSync(fullPath, { recursive: true });
    // Apply stored mode (strip file-type bits, keep permission bits)
    try { chmodSync(fullPath, row.mode & 0o7777); } catch {}
    count++;
  }

  // Then write files
  for (const row of rows) {
    if (row.is_dir) continue;
    const fullPath = join(targetDir, row.path);
    mkdirSync(dirname(fullPath), { recursive: true });
    const content = row.content ?? Buffer.alloc(0);
    writeFileSync(fullPath, content);
    // Apply stored mode (strip file-type bits, keep permission bits)
    try { chmodSync(fullPath, row.mode & 0o7777); } catch {}
    count++;
  }

  return count;
}

/**
 * Scan a real directory and upsert all files into workspace_files.
 * Deletes DB rows for files that no longer exist on disk.
 */
export async function syncFromFs(
  pool: pg.Pool,
  workspaceId: string,
  sourceDir: string,
  opts?: { exclude?: RegExp },
): Promise<number> {
  const exclude = opts?.exclude ?? /node_modules|\.git/;
  const seenPaths = new Set<string>(['/']);
  let count = 0;

  async function walkDir(dirPath: string, dbParent: string): Promise<void> {
    let entries: string[];
    try {
      entries = readdirSync(dirPath);
    } catch {
      return;
    }

    for (const name of entries) {
      if (exclude.test(name)) continue;

      const fullPath = join(dirPath, name);
      const dbPath = dbParent === '/' ? `/${name}` : `${dbParent}/${name}`;

      let st;
      try {
        st = statSync(fullPath);
      } catch {
        continue;
      }

      const now = nowNs();
      seenPaths.add(dbPath);

      if (st.isDirectory()) {
        await pool.query(
          `INSERT INTO _openeral.workspace_files
           (workspace_id, path, parent_path, name, is_dir, content, mode, size, mtime_ns, ctime_ns, atime_ns, nlink, uid, gid)
           VALUES ($1, $2, $3, $4, true, NULL, $5, 0, $6, $6, $6, 2, 1000, 1000)
           ON CONFLICT (workspace_id, path) DO UPDATE SET mode = $5, mtime_ns = $6`,
          [workspaceId, dbPath, dbParent, name, st.mode, now.toString()],
        );
        count++;
        await walkDir(fullPath, dbPath);
      } else if (st.isFile()) {
        const content = readFileSync(fullPath);
        await pool.query(
          `INSERT INTO _openeral.workspace_files
           (workspace_id, path, parent_path, name, is_dir, content, mode, size, mtime_ns, ctime_ns, atime_ns, nlink, uid, gid)
           VALUES ($1, $2, $3, $4, false, $5, $6, $7, $8, $8, $8, 1, 1000, 1000)
           ON CONFLICT (workspace_id, path) DO UPDATE SET content = $5, mode = $6, size = $7, mtime_ns = $8`,
          [workspaceId, dbPath, dbParent, name, content, st.mode, st.size, now.toString()],
        );
        count++;
      }
    }
  }

  // Ensure root exists
  const now = nowNs();
  await pool.query(
    `INSERT INTO _openeral.workspace_files
     (workspace_id, path, parent_path, name, is_dir, content, mode, size, mtime_ns, ctime_ns, atime_ns, nlink, uid, gid)
     VALUES ($1, '/', '', '', true, NULL, $2, 0, $3, $3, $3, 2, 1000, 1000)
     ON CONFLICT (workspace_id, path) DO NOTHING`,
    [workspaceId, 0o40755, now.toString()],
  );

  await walkDir(sourceDir, '/');

  // Delete DB rows for files that no longer exist on disk
  const { rows: dbRows } = await pool.query(
    `SELECT path FROM _openeral.workspace_files WHERE workspace_id = $1 AND path != '/'`,
    [workspaceId],
  );
  for (const row of dbRows) {
    if (!seenPaths.has(row.path)) {
      await pool.query(
        `DELETE FROM _openeral.workspace_files WHERE workspace_id = $1 AND path = $2`,
        [workspaceId, row.path],
      );
    }
  }

  return count;
}

/**
 * Watch a directory for changes and sync to PostgreSQL.
 * Returns a stop function.
 */
export function watchAndSync(
  pool: pg.Pool,
  workspaceId: string,
  dir: string,
  opts?: { debounceMs?: number; exclude?: RegExp },
): () => void {
  const debounceMs = opts?.debounceMs ?? 2000;
  const exclude = opts?.exclude ?? /node_modules|\.git/;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let syncing = false;

  const ac = new AbortController();

  try {
    const watcher = watch(dir, { recursive: true, signal: ac.signal });

    watcher.on('change', (_event, filename) => {
      if (typeof filename === 'string' && exclude.test(filename)) return;

      // Debounce: wait for changes to settle before syncing
      if (timer) clearTimeout(timer);
      timer = setTimeout(async () => {
        if (syncing) return;
        syncing = true;
        try {
          await syncFromFs(pool, workspaceId, dir, { exclude });
        } catch (err: any) {
          process.stderr.write(`openeral: sync error: ${err.message}\n`);
        } finally {
          syncing = false;
        }
      }, debounceMs);
    });

    watcher.on('error', () => {}); // ignore watcher errors
  } catch {
    // fs.watch may not support recursive on all platforms
  }

  return () => {
    ac.abort();
    if (timer) clearTimeout(timer);
  };
}
