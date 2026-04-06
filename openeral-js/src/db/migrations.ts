import type { DbPool } from './pool.js';

/**
 * Run all database migrations (V1-V4) in order.
 *
 * Uses an advisory lock to serialize concurrent callers — two shells
 * starting at the same time on a fresh database won't race on CREATE SCHEMA.
 *
 * Uses IF NOT EXISTS / CREATE TABLE IF NOT EXISTS for idempotency.
 * Safe to call multiple times -- already-existing objects are skipped.
 */
export async function runMigrations(pool: DbPool): Promise<void> {
  const client = await pool.connect();
  try {
    // Acquire an advisory lock (key 0x4F50454E = 'OPEN' in hex) to serialize
    // concurrent migration attempts. Without this, two shells hitting a fresh
    // database race on CREATE SCHEMA and one fails with duplicate key.
    await client.query(`SELECT pg_advisory_lock(1330795854)`);

    try {
      // V1: Create _openeral schema and schema_version table
      await client.query(`
        CREATE SCHEMA IF NOT EXISTS _openeral;

        CREATE TABLE IF NOT EXISTS _openeral.schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMPTZ DEFAULT NOW()
        );
      `);

      // V2: Create mount_log table
      await client.query(`
        CREATE TABLE IF NOT EXISTS _openeral.mount_log (
            id SERIAL PRIMARY KEY,
            mounted_at TIMESTAMPTZ DEFAULT NOW(),
            mount_point TEXT NOT NULL,
            schemas_filter TEXT[],
            page_size INTEGER,
            openeral_version TEXT
        );
      `);

      // V3: Create cache_hints table
      await client.query(`
        CREATE TABLE IF NOT EXISTS _openeral.cache_hints (
            id SERIAL PRIMARY KEY,
            schema_name TEXT NOT NULL,
            table_name TEXT NOT NULL,
            hint_type TEXT NOT NULL,
            hint_value TEXT,
            created_at TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE (schema_name, table_name, hint_type)
        );
      `);

      // V4: Create workspace_config, workspace_files, and index
      await client.query(`
        CREATE TABLE IF NOT EXISTS _openeral.workspace_config (
            id TEXT PRIMARY KEY,
            display_name TEXT,
            config JSONB NOT NULL DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        );

        CREATE TABLE IF NOT EXISTS _openeral.workspace_files (
            workspace_id TEXT NOT NULL REFERENCES _openeral.workspace_config(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            parent_path TEXT NOT NULL,
            name TEXT NOT NULL,
            is_dir BOOLEAN NOT NULL DEFAULT false,
            content BYTEA,
            mode INTEGER NOT NULL DEFAULT 33188,
            size BIGINT NOT NULL DEFAULT 0,
            mtime_ns BIGINT NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW()) * 1e9)::BIGINT,
            ctime_ns BIGINT NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW()) * 1e9)::BIGINT,
            atime_ns BIGINT NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW()) * 1e9)::BIGINT,
            nlink INTEGER NOT NULL DEFAULT 1,
            uid INTEGER NOT NULL DEFAULT 1000,
            gid INTEGER NOT NULL DEFAULT 1000,
            PRIMARY KEY (workspace_id, path)
        );

        CREATE INDEX IF NOT EXISTS idx_ws_files_parent
            ON _openeral.workspace_files (workspace_id, parent_path);
      `);
    } finally {
      await client.query(`SELECT pg_advisory_unlock(1330795854)`);
    }
  } finally {
    client.release();
  }
}
