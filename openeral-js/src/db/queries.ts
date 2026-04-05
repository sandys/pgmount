import type { DbPool } from './pool.js';
import type {
  SchemaInfo,
  TableInfo,
  ColumnInfo,
  PrimaryKeyInfo,
  IndexInfo,
  RowIdentifier,
} from './types.js';

/**
 * Wraps an identifier in double quotes, escaping any internal double quotes
 * by doubling them.
 */
export function quoteIdent(s: string): string {
  return `"${s.replace(/"/g, '""')}"`;
}

/**
 * Percent-encode characters that are unsafe in PK display names:
 * / , = % and control characters (0x00-0x1F, 0x7F).
 */
export function encodePkValue(value: string): string {
  return value.replace(/[/,=%\x00-\x1f\x7f]/g, (ch) => {
    const code = ch.charCodeAt(0);
    return `%${code.toString(16).toUpperCase().padStart(2, '0')}`;
  });
}

export async function listSchemas(pool: DbPool): Promise<SchemaInfo[]> {
  const { rows } = await pool.query(
    `SELECT schema_name FROM information_schema.schemata \
     WHERE schema_name NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
     ORDER BY schema_name`,
  );
  return rows.map((r) => ({ name: r.schema_name as string }));
}

export async function listTables(
  pool: DbPool,
  schema: string,
): Promise<TableInfo[]> {
  const { rows } = await pool.query(
    `SELECT table_name, table_type FROM information_schema.tables \
     WHERE table_schema = $1 \
     ORDER BY table_name`,
    [schema],
  );
  return rows.map((r) => ({
    name: r.table_name as string,
    tableType: r.table_type as string,
  }));
}

export async function listColumns(
  pool: DbPool,
  schema: string,
  table: string,
): Promise<ColumnInfo[]> {
  const { rows } = await pool.query(
    `SELECT column_name, data_type, is_nullable, column_default, ordinal_position \
     FROM information_schema.columns \
     WHERE table_schema = $1 AND table_name = $2 \
     ORDER BY ordinal_position`,
    [schema, table],
  );
  return rows.map((r) => ({
    name: r.column_name as string,
    dataType: r.data_type as string,
    isNullable: (r.is_nullable as string) === 'YES',
    columnDefault: (r.column_default as string | null) ?? null,
    ordinalPosition: Number(r.ordinal_position),
  }));
}

export async function getPrimaryKey(
  pool: DbPool,
  schema: string,
  table: string,
): Promise<PrimaryKeyInfo> {
  const { rows } = await pool.query(
    `SELECT kcu.column_name \
     FROM information_schema.table_constraints tc \
     JOIN information_schema.key_column_usage kcu \
       ON tc.constraint_name = kcu.constraint_name \
      AND tc.table_schema = kcu.table_schema \
     WHERE tc.constraint_type = 'PRIMARY KEY' \
       AND tc.table_schema = $1 \
       AND tc.table_name = $2 \
     ORDER BY kcu.ordinal_position`,
    [schema, table],
  );
  return { columnNames: rows.map((r) => r.column_name as string) };
}

/**
 * Returns an estimated row count from pg_class.reltuples.
 * Fast but may be stale if ANALYZE hasn't been run recently.
 */
export async function getRowCountEstimate(
  pool: DbPool,
  schema: string,
  table: string,
): Promise<number> {
  const { rows } = await pool.query(
    `SELECT COALESCE(reltuples::bigint, 0) as count \
     FROM pg_class c \
     JOIN pg_namespace n ON c.relnamespace = n.oid \
     WHERE n.nspname = $1 AND c.relname = $2`,
    [schema, table],
  );
  if (rows.length === 0) {
    throw new Error(`Table not found: ${schema}.${table}`);
  }
  return Number(rows[0].count);
}

/**
 * Returns the exact row count via COUNT(*).
 * Performs a full table scan and may be slow on large tables.
 */
export async function getExactRowCount(
  pool: DbPool,
  schema: string,
  table: string,
): Promise<number> {
  const query = `SELECT COUNT(*) as count FROM ${quoteIdent(schema)}.${quoteIdent(table)}`;
  const { rows } = await pool.query(query);
  if (rows.length === 0) {
    throw new Error('COUNT(*) returned no rows');
  }
  return Number(rows[0].count);
}

export async function listIndexes(
  pool: DbPool,
  schema: string,
  table: string,
): Promise<IndexInfo[]> {
  const { rows } = await pool.query(
    `SELECT i.relname as index_name, ix.indisunique, ix.indisprimary, \
            pg_get_indexdef(i.oid) as definition, \
            array_agg(a.attname ORDER BY array_position(ix.indkey, a.attnum)) as columns \
     FROM pg_class t \
     JOIN pg_namespace n ON t.relnamespace = n.oid \
     JOIN pg_index ix ON t.oid = ix.indrelid \
     JOIN pg_class i ON ix.indexrelid = i.oid \
     JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) \
     WHERE n.nspname = $1 AND t.relname = $2 \
     GROUP BY i.relname, ix.indisunique, ix.indisprimary, i.oid \
     ORDER BY i.relname`,
    [schema, table],
  );
  return rows.map((r) => ({
    name: r.index_name as string,
    isUnique: r.indisunique as boolean,
    isPrimary: r.indisprimary as boolean,
    definition: r.definition as string,
    columns: r.columns as string[],
  }));
}

/**
 * Flexible row query with optional WHERE and ORDER BY clauses.
 * `extraWhere` is appended to the WHERE clause (e.g., '"category"::text = $1')
 * `extraOrder` overrides the default PK ordering (e.g., '"name" ASC')
 * `extraParams` are the parameters for the extraWhere clause
 */
export async function queryRows(
  pool: DbPool,
  schema: string,
  table: string,
  pkColumns: string[],
  limit: number,
  offset: number,
  extraWhere?: string,
  extraOrder?: string,
  extraParams?: unknown[],
): Promise<RowIdentifier[]> {
  if (pkColumns.length === 0) {
    throw new Error('No primary key columns specified');
  }

  const selectCols = pkColumns.map((c) => quoteIdent(c));
  const orderClause = extraOrder ?? selectCols.join(', ');
  const params = extraParams ?? [];

  // extraParams use $1..$N, then LIMIT is $N+1, OFFSET is $N+2
  const paramOffset = params.length;
  const whereClause = extraWhere ? ` WHERE ${extraWhere}` : '';

  const query = `SELECT ${selectCols.join(', ')} FROM ${quoteIdent(schema)}.${quoteIdent(table)}${whereClause} ORDER BY ${orderClause} LIMIT $${paramOffset + 1} OFFSET $${paramOffset + 2}`;

  const allParams = [...params, limit, offset];
  const { rows } = await pool.query(query, allParams);

  return rows.map((row) => {
    const pkValues: [string, string][] = [];
    const displayParts: string[] = [];

    for (const colName of pkColumns) {
      const valueStr = String(row[colName] ?? 'NULL');
      if (pkColumns.length === 1) {
        displayParts.push(encodePkValue(valueStr));
      } else {
        displayParts.push(`${colName}=${encodePkValue(valueStr)}`);
      }
      pkValues.push([colName, valueStr]);
    }

    return {
      pkValues,
      displayName: displayParts.join(','),
    };
  });
}

export async function listRows(
  pool: DbPool,
  schema: string,
  table: string,
  pkColumns: string[],
  limit: number,
  offset: number,
): Promise<RowIdentifier[]> {
  return queryRows(pool, schema, table, pkColumns, limit, offset);
}

/**
 * Fetch all rows from a table as text columns.
 * Returns column names and row data where each cell is [columnName, value|null].
 */
export async function getAllRowsAsText(
  pool: DbPool,
  schema: string,
  table: string,
  limit: number,
  offset: number,
): Promise<{ colNames: string[]; rows: [string, string | null][][] }> {
  // Get column names
  const colResult = await pool.query(
    `SELECT column_name FROM information_schema.columns \
     WHERE table_schema = $1 AND table_name = $2 \
     ORDER BY ordinal_position`,
    [schema, table],
  );
  const colNames: string[] = colResult.rows.map(
    (r) => r.column_name as string,
  );

  // Build SELECT with ::text cast for every column
  const selectExprs = colNames.map((c) => `${quoteIdent(c)}::text`);
  const query = `SELECT ${selectExprs.join(', ')} FROM ${quoteIdent(schema)}.${quoteIdent(table)} ORDER BY 1 LIMIT $1 OFFSET $2`;

  const { rows } = await pool.query(query, [limit, offset]);

  const result: [string, string | null][][] = rows.map((row) =>
    colNames.map((colName) => [colName, (row[colName] as string | null) ?? null]),
  );

  return { colNames, rows: result };
}

/**
 * Get all column values for a single row identified by primary key.
 * All values cast to ::text.
 */
export async function getRowData(
  pool: DbPool,
  schema: string,
  table: string,
  pkColumns: string[],
  pkValues: string[],
): Promise<[string, string | null][]> {
  if (pkColumns.length !== pkValues.length) {
    throw new Error('PK columns and values length mismatch');
  }

  const whereClauses = pkColumns.map(
    (col, i) => `${quoteIdent(col)}::text = $${i + 1}`,
  );

  // Get column names
  const colResult = await pool.query(
    `SELECT column_name FROM information_schema.columns \
     WHERE table_schema = $1 AND table_name = $2 \
     ORDER BY ordinal_position`,
    [schema, table],
  );
  const colNames: string[] = colResult.rows.map(
    (r) => r.column_name as string,
  );

  // Build SELECT with ::text cast for every column
  const selectExprs = colNames.map((c) => `${quoteIdent(c)}::text`);
  const query = `SELECT ${selectExprs.join(', ')} FROM ${quoteIdent(schema)}.${quoteIdent(table)} WHERE ${whereClauses.join(' AND ')}`;

  const { rows } = await pool.query(query, pkValues);

  if (rows.length === 0) {
    throw new Error('Row not found');
  }

  return colNames.map((colName) => [
    colName,
    (rows[0][colName] as string | null) ?? null,
  ]);
}

/**
 * Get a single column value for a row identified by primary key.
 * Value cast to ::text.
 */
export async function getColumnValue(
  pool: DbPool,
  schema: string,
  table: string,
  column: string,
  pkColumns: string[],
  pkValues: string[],
): Promise<string | null> {
  if (pkColumns.length !== pkValues.length) {
    throw new Error('PK columns and values length mismatch');
  }

  const whereClauses = pkColumns.map(
    (col, i) => `${quoteIdent(col)}::text = $${i + 1}`,
  );

  const query = `SELECT ${quoteIdent(column)}::text FROM ${quoteIdent(schema)}.${quoteIdent(table)} WHERE ${whereClauses.join(' AND ')}`;

  const { rows } = await pool.query(query, pkValues);

  if (rows.length === 0) {
    throw new Error('Row not found');
  }

  return (rows[0][column] as string | null) ?? null;
}
