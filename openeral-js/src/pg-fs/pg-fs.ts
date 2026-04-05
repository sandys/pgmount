import pg from 'pg';
import {
  listSchemas,
  listTables,
  listColumns,
  getPrimaryKey,
  getRowCountEstimate,
  getExactRowCount,
  listIndexes,
  listRows,
  queryRows,
  getAllRowsAsText,
  getRowData,
  getColumnValue,
  quoteIdent,
} from '../db/queries.js';
import type {
  SchemaInfo,
  TableInfo,
  ColumnInfo,
  PrimaryKeyInfo,
} from '../db/types.js';
import { parsePath, isDirectory, parsePkDisplay } from './path-parser.js';
import type { PgNode } from './path-parser.js';

// ---------------------------------------------------------------------------
// Minimal stat shape (matches the IFileSystem contract from just-bash)
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
// Error helpers
// ---------------------------------------------------------------------------

function fsError(code: string, message: string): Error {
  const err = new Error(message);
  (err as NodeJS.ErrnoException).code = code;
  return err;
}

function enoent(path: string): Error {
  return fsError('ENOENT', `ENOENT: no such file or directory, '${path}'`);
}

function erofs(op: string): Error {
  return fsError('EROFS', `EROFS: read-only file system, ${op}`);
}

function enotsup(op: string): Error {
  return fsError('ENOTSUP', `ENOTSUP: operation not supported, ${op}`);
}

function enotdir(path: string): Error {
  return fsError('ENOTDIR', `ENOTDIR: not a directory, '${path}'`);
}

// ---------------------------------------------------------------------------
// MetadataCache — simple Map with TTL (ported from Rust cache.rs)
// ---------------------------------------------------------------------------

interface CacheEntry<T> {
  data: T;
  insertedAt: number; // Date.now() millis
}

class MetadataCache {
  private ttlMs: number;
  private schemas = new Map<string, CacheEntry<SchemaInfo[]>>();
  private tables = new Map<string, CacheEntry<TableInfo[]>>();
  private columns = new Map<string, CacheEntry<ColumnInfo[]>>();
  private primaryKeys = new Map<string, CacheEntry<PrimaryKeyInfo>>();
  private rowCounts = new Map<string, CacheEntry<number>>();

  constructor(ttlMs: number) {
    this.ttlMs = ttlMs;
  }

  private isValid<T>(entry: CacheEntry<T> | undefined): entry is CacheEntry<T> {
    if (!entry) return false;
    return Date.now() - entry.insertedAt < this.ttlMs;
  }

  private key(schema: string, table?: string): string {
    return table ? `${schema}.${table}` : schema;
  }

  getSchemas(): SchemaInfo[] | null {
    const e = this.schemas.get('');
    return this.isValid(e) ? e.data : null;
  }
  setSchemas(data: SchemaInfo[]): void {
    this.schemas.set('', { data, insertedAt: Date.now() });
  }

  getTables(schema: string): TableInfo[] | null {
    const e = this.tables.get(schema);
    return this.isValid(e) ? e.data : null;
  }
  setTables(schema: string, data: TableInfo[]): void {
    this.tables.set(schema, { data, insertedAt: Date.now() });
  }

  getColumns(schema: string, table: string): ColumnInfo[] | null {
    const e = this.columns.get(this.key(schema, table));
    return this.isValid(e) ? e.data : null;
  }
  setColumns(schema: string, table: string, data: ColumnInfo[]): void {
    this.columns.set(this.key(schema, table), { data, insertedAt: Date.now() });
  }

  getPrimaryKey(schema: string, table: string): PrimaryKeyInfo | null {
    const e = this.primaryKeys.get(this.key(schema, table));
    return this.isValid(e) ? e.data : null;
  }
  setPrimaryKey(schema: string, table: string, data: PrimaryKeyInfo): void {
    this.primaryKeys.set(this.key(schema, table), { data, insertedAt: Date.now() });
  }

  getRowCount(schema: string, table: string): number | null {
    const e = this.rowCounts.get(this.key(schema, table));
    return this.isValid(e) ? e.data : null;
  }
  setRowCount(schema: string, table: string, data: number): void {
    this.rowCounts.set(this.key(schema, table), { data, insertedAt: Date.now() });
  }

  invalidateAll(): void {
    this.schemas.clear();
    this.tables.clear();
    this.columns.clear();
    this.primaryKeys.clear();
    this.rowCounts.clear();
  }
}

// ---------------------------------------------------------------------------
// Path utilities
// ---------------------------------------------------------------------------

function normalizePath(p: string): string {
  let out = p.startsWith('/') ? p : `/${p}`;
  if (out.length > 1 && out.endsWith('/')) {
    out = out.slice(0, -1);
  }
  return out;
}

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

// ---------------------------------------------------------------------------
// PgFs — read-only IFileSystem backed by PostgreSQL queries
// ---------------------------------------------------------------------------

export class PgFs {
  private pool: pg.Pool;
  private pageSize: number;
  private cache: MetadataCache;

  constructor(pool: pg.Pool, opts?: { pageSize?: number; cacheTtlMs?: number }) {
    this.pool = pool;
    this.pageSize = opts?.pageSize ?? 1000;
    this.cache = new MetadataCache(opts?.cacheTtlMs ?? 30_000);
  }

  // -----------------------------------------------------------------------
  // Cached data accessors
  // -----------------------------------------------------------------------

  private async cachedSchemas(): Promise<SchemaInfo[]> {
    const cached = this.cache.getSchemas();
    if (cached) return cached;
    const data = await listSchemas(this.pool);
    this.cache.setSchemas(data);
    return data;
  }

  private async cachedTables(schema: string): Promise<TableInfo[]> {
    const cached = this.cache.getTables(schema);
    if (cached) return cached;
    const data = await listTables(this.pool, schema);
    this.cache.setTables(schema, data);
    return data;
  }

  private async cachedColumns(schema: string, table: string): Promise<ColumnInfo[]> {
    const cached = this.cache.getColumns(schema, table);
    if (cached) return cached;
    const data = await listColumns(this.pool, schema, table);
    this.cache.setColumns(schema, table, data);
    return data;
  }

  private async cachedPrimaryKey(schema: string, table: string): Promise<PrimaryKeyInfo> {
    const cached = this.cache.getPrimaryKey(schema, table);
    if (cached) return cached;
    const data = await getPrimaryKey(this.pool, schema, table);
    this.cache.setPrimaryKey(schema, table, data);
    return data;
  }

  private async cachedRowCount(schema: string, table: string): Promise<number> {
    const cached = this.cache.getRowCount(schema, table);
    if (cached !== null) return cached;
    const data = await getRowCountEstimate(this.pool, schema, table);
    this.cache.setRowCount(schema, table, data);
    return data;
  }

  /** Number of pages for a table given its estimated row count. */
  private pageCount(rowCount: number): number {
    if (rowCount <= 0) return 1;
    return Math.ceil(rowCount / this.pageSize);
  }

  // -----------------------------------------------------------------------
  // readdir
  // -----------------------------------------------------------------------

  async readdir(path: string): Promise<string[]> {
    const norm = normalizePath(path);
    const node = parsePath(norm);
    if (!node) throw enoent(path);
    if (!isDirectory(node)) throw enotdir(path);

    return this.readdirNode(node, path);
  }

  private async readdirNode(node: PgNode, rawPath: string): Promise<string[]> {
    switch (node.type) {
      case 'root': {
        const schemas = await this.cachedSchemas();
        return schemas.map((s) => s.name);
      }

      case 'schema': {
        const tables = await this.cachedTables(node.schema);
        return tables.map((t) => t.name);
      }

      case 'table': {
        const rowCount = await this.cachedRowCount(node.schema, node.table);
        const pages = this.pageCount(rowCount);
        const entries = ['.info', '.export', '.filter', '.order', '.indexes'];
        for (let i = 1; i <= pages; i++) {
          entries.push(`page_${i}`);
        }
        return entries;
      }

      case 'specialDir': {
        switch (node.kind) {
          case 'info':
            return ['columns.json', 'schema.sql', 'count', 'primary_key'];
          case 'export':
            return ['data.json', 'data.csv', 'data.yaml'];
          case 'filter': {
            const cols = await this.cachedColumns(node.schema, node.table);
            return cols.map((c) => c.name);
          }
          case 'order': {
            const cols = await this.cachedColumns(node.schema, node.table);
            return cols.map((c) => c.name);
          }
          case 'indexes': {
            const indexes = await listIndexes(this.pool, node.schema, node.table);
            return indexes.map((idx) => idx.name);
          }
        }
        break;
      }

      case 'exportDir': {
        const rowCount = await this.cachedRowCount(node.schema, node.table);
        const pages = this.pageCount(rowCount);
        const entries: string[] = [];
        for (let i = 1; i <= pages; i++) {
          entries.push(`page_${i}.${node.format}`);
        }
        return entries;
      }

      case 'filterColumn': {
        const { rows } = await this.pool.query(
          `SELECT DISTINCT ${quoteIdent(node.column)}::text AS val FROM ${quoteIdent(node.schema)}.${quoteIdent(node.table)} WHERE ${quoteIdent(node.column)} IS NOT NULL ORDER BY val LIMIT 10000`,
        );
        return rows.map((r) => r.val as string);
      }

      case 'filterValue': {
        const pk = await this.cachedPrimaryKey(node.schema, node.table);
        if (pk.columnNames.length === 0) return [];
        const rows = await queryRows(
          this.pool,
          node.schema,
          node.table,
          pk.columnNames,
          this.pageSize,
          0,
          `${quoteIdent(node.column)}::text = $1`,
          undefined,
          [node.value],
        );
        return rows.map((r) => r.displayName);
      }

      case 'orderColumn':
        return ['asc', 'desc'];

      case 'orderDirection': {
        const pk = await this.cachedPrimaryKey(node.schema, node.table);
        if (pk.columnNames.length === 0) return [];
        const rows = await queryRows(
          this.pool,
          node.schema,
          node.table,
          pk.columnNames,
          this.pageSize,
          0,
          undefined,
          `${quoteIdent(node.column)} ${node.dir.toUpperCase()}`,
        );
        return rows.map((r) => r.displayName);
      }

      case 'pageDir': {
        const pk = await this.cachedPrimaryKey(node.schema, node.table);
        if (pk.columnNames.length === 0) return [];
        const offset = (node.page - 1) * this.pageSize;
        const rows = await listRows(
          this.pool,
          node.schema,
          node.table,
          pk.columnNames,
          this.pageSize,
          offset,
        );
        return rows.map((r) => r.displayName);
      }

      case 'row': {
        const cols = await this.cachedColumns(node.schema, node.table);
        return [...cols.map((c) => c.name), 'row.json', 'row.csv', 'row.yaml'];
      }

      default:
        throw enoent(rawPath);
    }

    // Unreachable for well-formed switch, but satisfy the compiler.
    throw enoent(rawPath);
  }

  // -----------------------------------------------------------------------
  // readFile
  // -----------------------------------------------------------------------

  async readFile(
    path: string,
    _options?: { encoding?: BufferEncoding },
  ): Promise<string> {
    const norm = normalizePath(path);
    const node = parsePath(norm);
    if (!node) throw enoent(path);
    if (isDirectory(node)) throw fsError('EISDIR', `EISDIR: illegal operation on a directory, read '${path}'`);

    return this.readFileNode(node, path);
  }

  private async readFileNode(node: PgNode, rawPath: string): Promise<string> {
    switch (node.type) {
      case 'infoFile':
        return this.readInfoFile(node.schema, node.table, node.filename);

      case 'column': {
        const pk = await this.cachedPrimaryKey(node.schema, node.table);
        const pkValues = parsePkDisplay(node.pkDisplay, pk.columnNames);
        const value = await getColumnValue(
          this.pool,
          node.schema,
          node.table,
          node.column,
          pk.columnNames,
          pkValues,
        );
        return value ?? 'NULL';
      }

      case 'rowFile': {
        const pk = await this.cachedPrimaryKey(node.schema, node.table);
        const pkValues = parsePkDisplay(node.pkDisplay, pk.columnNames);
        const data = await getRowData(
          this.pool,
          node.schema,
          node.table,
          pk.columnNames,
          pkValues,
        );
        return this.formatRowData(data, node.format);
      }

      case 'exportPageFile': {
        const offset = (node.page - 1) * this.pageSize;
        const { colNames, rows } = await getAllRowsAsText(
          this.pool,
          node.schema,
          node.table,
          this.pageSize,
          offset,
        );
        return this.formatExportPage(colNames, rows, node.format);
      }

      case 'indexFile': {
        const indexes = await listIndexes(this.pool, node.schema, node.table);
        const idx = indexes.find((i) => i.name === node.indexName);
        if (!idx) throw enoent(rawPath);
        return JSON.stringify(idx, null, 2) + '\n';
      }

      default:
        throw enoent(rawPath);
    }
  }

  // -----------------------------------------------------------------------
  // .info file content generators
  // -----------------------------------------------------------------------

  private async readInfoFile(schema: string, table: string, filename: string): Promise<string> {
    switch (filename) {
      case 'columns.json': {
        const cols = await this.cachedColumns(schema, table);
        return JSON.stringify(cols, null, 2) + '\n';
      }

      case 'schema.sql': {
        const cols = await this.cachedColumns(schema, table);
        const pk = await this.cachedPrimaryKey(schema, table);
        return this.generateCreateTable(schema, table, cols, pk);
      }

      case 'count': {
        const count = await getExactRowCount(this.pool, schema, table);
        return String(count) + '\n';
      }

      case 'primary_key': {
        const pk = await this.cachedPrimaryKey(schema, table);
        return pk.columnNames.join('\n') + '\n';
      }

      default:
        throw enoent(filename);
    }
  }

  private generateCreateTable(
    schema: string,
    table: string,
    cols: ColumnInfo[],
    pk: PrimaryKeyInfo,
  ): string {
    const lines: string[] = [];
    lines.push(`CREATE TABLE ${quoteIdent(schema)}.${quoteIdent(table)} (`);

    const colDefs = cols.map((c) => {
      let def = `  ${quoteIdent(c.name)} ${c.dataType.toUpperCase()}`;
      if (!c.isNullable) def += ' NOT NULL';
      if (c.columnDefault !== null) def += ` DEFAULT ${c.columnDefault}`;
      return def;
    });

    if (pk.columnNames.length > 0) {
      const pkCols = pk.columnNames.map((c) => quoteIdent(c)).join(', ');
      colDefs.push(`  PRIMARY KEY (${pkCols})`);
    }

    lines.push(colDefs.join(',\n'));
    lines.push(');\n');
    return lines.join('\n');
  }

  // -----------------------------------------------------------------------
  // Format helpers
  // -----------------------------------------------------------------------

  private formatRowData(data: [string, string | null][], format: string): string {
    switch (format) {
      case 'json': {
        const obj: Record<string, string | null> = {};
        for (const [k, v] of data) {
          obj[k] = v;
        }
        return JSON.stringify(obj, null, 2) + '\n';
      }

      case 'csv': {
        const header = data.map(([k]) => this.csvEscape(k)).join(',');
        const values = data.map(([, v]) => this.csvEscape(v ?? 'NULL')).join(',');
        return header + '\n' + values + '\n';
      }

      case 'yaml': {
        const lines = data.map(([k, v]) => `${k}: ${v ?? 'null'}`);
        return lines.join('\n') + '\n';
      }

      default:
        throw new Error(`Unknown format: ${format}`);
    }
  }

  private formatExportPage(
    colNames: string[],
    rows: [string, string | null][][],
    format: string,
  ): string {
    switch (format) {
      case 'json': {
        const objects = rows.map((row) => {
          const obj: Record<string, string | null> = {};
          for (const [k, v] of row) {
            obj[k] = v;
          }
          return obj;
        });
        return JSON.stringify(objects, null, 2) + '\n';
      }

      case 'csv': {
        const header = colNames.map((c) => this.csvEscape(c)).join(',');
        const dataLines = rows.map((row) =>
          row.map(([, v]) => this.csvEscape(v ?? 'NULL')).join(','),
        );
        return [header, ...dataLines].join('\n') + '\n';
      }

      case 'yaml': {
        const docs = rows.map((row) => {
          const lines = row.map(([k, v]) => `  ${k}: ${v ?? 'null'}`);
          return '- \n' + lines.join('\n');
        });
        return docs.join('\n') + '\n';
      }

      default:
        throw new Error(`Unknown format: ${format}`);
    }
  }

  private csvEscape(value: string): string {
    if (value.includes(',') || value.includes('"') || value.includes('\n')) {
      return `"${value.replace(/"/g, '""')}"`;
    }
    return value;
  }

  // -----------------------------------------------------------------------
  // stat
  // -----------------------------------------------------------------------

  async stat(path: string): Promise<FsStat> {
    const norm = normalizePath(path);
    const node = parsePath(norm);
    if (!node) throw enoent(path);

    // Validate the node actually exists by checking parent data when feasible
    await this.validateNodeExists(node, path);

    if (isDirectory(node)) {
      return {
        isFile: false,
        isDirectory: true,
        isSymbolicLink: false,
        mode: 0o40755,
        size: 0,
        mtime: new Date(),
      };
    }

    return {
      isFile: true,
      isDirectory: false,
      isSymbolicLink: false,
      mode: 0o100444,
      size: 0,
      mtime: new Date(),
    };
  }

  /**
   * Best-effort validation that a parsed node actually corresponds to a real
   * database object. We check schemas and tables against the cache/DB. For
   * deeper paths we trust the parser — the data will error on read if absent.
   */
  private async validateNodeExists(node: PgNode, rawPath: string): Promise<void> {
    if (node.type === 'root') return;

    // All non-root nodes have a schema field.
    const schemaName = (node as { schema: string }).schema;
    const schemas = await this.cachedSchemas();
    if (!schemas.some((s) => s.name === schemaName)) {
      throw enoent(rawPath);
    }

    // Schema-only node — no table to check.
    if (node.type === 'schema') return;

    // Everything else references a table.
    const tableName = (node as { table: string }).table;
    const tables = await this.cachedTables(schemaName);
    if (!tables.some((t) => t.name === tableName)) {
      throw enoent(rawPath);
    }
  }

  // -----------------------------------------------------------------------
  // exists
  // -----------------------------------------------------------------------

  async exists(path: string): Promise<boolean> {
    try {
      await this.stat(path);
      return true;
    } catch {
      return false;
    }
  }

  // -----------------------------------------------------------------------
  // Write methods — all throw EROFS
  // -----------------------------------------------------------------------

  async writeFile(
    _path: string,
    _content: string | Uint8Array,
    _options?: { encoding?: BufferEncoding; mode?: number },
  ): Promise<void> {
    throw erofs('writeFile');
  }

  async appendFile(
    _path: string,
    _content: string | Uint8Array,
    _options?: { encoding?: BufferEncoding },
  ): Promise<void> {
    throw erofs('appendFile');
  }

  async mkdir(
    _path: string,
    _options?: { recursive?: boolean; mode?: number },
  ): Promise<void> {
    throw erofs('mkdir');
  }

  async rm(
    _path: string,
    _options?: { recursive?: boolean; force?: boolean },
  ): Promise<void> {
    throw erofs('rm');
  }

  async cp(
    _src: string,
    _dest: string,
    _options?: { recursive?: boolean },
  ): Promise<void> {
    throw erofs('cp');
  }

  async mv(_src: string, _dest: string): Promise<void> {
    throw erofs('mv');
  }

  async chmod(_path: string, _mode: number): Promise<void> {
    throw erofs('chmod');
  }

  async utimes(_path: string, _atime: Date, _mtime: Date): Promise<void> {
    throw erofs('utimes');
  }

  async symlink(_target: string, _linkPath: string): Promise<void> {
    throw erofs('symlink');
  }

  async link(_existingPath: string, _newPath: string): Promise<void> {
    throw erofs('link');
  }

  // -----------------------------------------------------------------------
  // getAllPaths — structural paths only (schemas + tables + special dirs)
  // -----------------------------------------------------------------------

  async getAllPaths(): Promise<string[]> {
    const paths: string[] = ['/'];
    const schemas = await this.cachedSchemas();

    for (const s of schemas) {
      const schemaPath = `/${s.name}`;
      paths.push(schemaPath);

      const tables = await this.cachedTables(s.name);
      for (const t of tables) {
        const tablePath = `${schemaPath}/${t.name}`;
        paths.push(tablePath);
        paths.push(`${tablePath}/.info`);
        paths.push(`${tablePath}/.export`);
        paths.push(`${tablePath}/.filter`);
        paths.push(`${tablePath}/.order`);
        paths.push(`${tablePath}/.indexes`);
      }
    }

    return paths;
  }

  // -----------------------------------------------------------------------
  // Path resolution
  // -----------------------------------------------------------------------

  resolvePath(base: string, path: string): string {
    if (path.startsWith('/')) {
      return resolveSegments(path);
    }
    const combined = base.endsWith('/')
      ? base + path
      : base + '/' + path;
    return resolveSegments(combined);
  }

  // -----------------------------------------------------------------------
  // realpath / lstat / readlink / readFileBuffer
  // -----------------------------------------------------------------------

  async realpath(path: string): Promise<string> {
    const resolved = resolveSegments(normalizePath(path));
    await this.stat(resolved); // throws ENOENT if not found
    return resolved;
  }

  async lstat(path: string): Promise<FsStat> {
    return this.stat(path);
  }

  async readlink(_path: string): Promise<string> {
    throw enotsup('readlink');
  }

  async readFileBuffer(path: string): Promise<Uint8Array> {
    const content = await this.readFile(path);
    return new TextEncoder().encode(content);
  }
}
