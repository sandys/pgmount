// ---------------------------------------------------------------------------
// PgFs path parser — discriminated union for every node in the read-only
// PostgreSQL virtual filesystem.
// ---------------------------------------------------------------------------

export type PgNode =
  | { type: 'root' }
  | { type: 'schema'; schema: string }
  | { type: 'table'; schema: string; table: string }
  | { type: 'specialDir'; schema: string; table: string; kind: 'info' | 'export' | 'filter' | 'order' | 'indexes' }
  | { type: 'infoFile'; schema: string; table: string; filename: string }
  | { type: 'exportDir'; schema: string; table: string; format: string }
  | { type: 'exportPageFile'; schema: string; table: string; format: string; page: number }
  | { type: 'filterColumn'; schema: string; table: string; column: string }
  | { type: 'filterValue'; schema: string; table: string; column: string; value: string }
  | { type: 'orderColumn'; schema: string; table: string; column: string }
  | { type: 'orderDirection'; schema: string; table: string; column: string; dir: string }
  | { type: 'indexFile'; schema: string; table: string; indexName: string }
  | { type: 'pageDir'; schema: string; table: string; page: number }
  | { type: 'row'; schema: string; table: string; pkDisplay: string }
  | { type: 'column'; schema: string; table: string; pkDisplay: string; column: string }
  | { type: 'rowFile'; schema: string; table: string; pkDisplay: string; format: string }

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const SPECIAL_DIRS = new Set(['.info', '.export', '.filter', '.order', '.indexes']);
const INFO_FILES = new Set(['columns.json', 'schema.sql', 'count', 'primary_key']);
const EXPORT_FORMATS: Record<string, string> = {
  'data.json': 'json',
  'data.csv': 'csv',
  'data.yaml': 'yaml',
};

const PAGE_DIR_RE = /^page_(\d+)$/;
const EXPORT_PAGE_RE = /^page_(\d+)\.(json|csv|yaml)$/;

const ROW_FILE_FORMATS: Record<string, string> = {
  'row.json': 'json',
  'row.csv': 'csv',
  'row.yaml': 'yaml',
};

// ---------------------------------------------------------------------------
// parsePath
// ---------------------------------------------------------------------------

export function parsePath(path: string): PgNode | null {
  const segments = path.split('/').filter((s) => s !== '');

  if (segments.length === 0) {
    return { type: 'root' };
  }

  if (segments.length === 1) {
    return { type: 'schema', schema: segments[0] };
  }

  const schema = segments[0];
  const table = segments[1];

  if (segments.length === 2) {
    return { type: 'table', schema, table };
  }

  const third = segments[2];

  // --- Special directories (.info, .export, .filter, .order, .indexes) ---

  if (SPECIAL_DIRS.has(third)) {
    const kind = third.slice(1) as 'info' | 'export' | 'filter' | 'order' | 'indexes';

    if (segments.length === 3) {
      return { type: 'specialDir', schema, table, kind };
    }

    const fourth = segments[3];

    switch (kind) {
      case 'info':
        if (segments.length === 4 && INFO_FILES.has(fourth)) {
          return { type: 'infoFile', schema, table, filename: fourth };
        }
        return null;

      case 'export': {
        const format = EXPORT_FORMATS[fourth];
        if (!format) return null;
        if (segments.length === 4) {
          return { type: 'exportDir', schema, table, format };
        }
        if (segments.length === 5) {
          const pageMatch = segments[4].match(EXPORT_PAGE_RE);
          if (pageMatch && pageMatch[2] === format) {
            return { type: 'exportPageFile', schema, table, format, page: Number(pageMatch[1]) };
          }
        }
        return null;
      }

      case 'filter':
        if (segments.length === 4) {
          return { type: 'filterColumn', schema, table, column: fourth };
        }
        if (segments.length === 5) {
          return { type: 'filterValue', schema, table, column: fourth, value: segments[4] };
        }
        return null;

      case 'order':
        if (segments.length === 4) {
          return { type: 'orderColumn', schema, table, column: fourth };
        }
        if (segments.length === 5 && (segments[4] === 'asc' || segments[4] === 'desc')) {
          return { type: 'orderDirection', schema, table, column: fourth, dir: segments[4] };
        }
        return null;

      case 'indexes':
        if (segments.length === 4) {
          return { type: 'indexFile', schema, table, indexName: fourth };
        }
        return null;
    }
  }

  // --- Page directories (page_N) ---

  const pageMatch = third.match(PAGE_DIR_RE);
  if (pageMatch) {
    const page = Number(pageMatch[1]);

    if (segments.length === 3) {
      return { type: 'pageDir', schema, table, page };
    }

    const pkDisplay = segments[3];

    if (segments.length === 4) {
      return { type: 'row', schema, table, pkDisplay };
    }

    if (segments.length === 5) {
      const fifth = segments[4];
      const rowFormat = ROW_FILE_FORMATS[fifth];
      if (rowFormat) {
        return { type: 'rowFile', schema, table, pkDisplay, format: rowFormat };
      }
      return { type: 'column', schema, table, pkDisplay, column: fifth };
    }
  }

  return null;
}

// ---------------------------------------------------------------------------
// isDirectory — true for node types that represent directories
// ---------------------------------------------------------------------------

export function isDirectory(node: PgNode): boolean {
  switch (node.type) {
    case 'root':
    case 'schema':
    case 'table':
    case 'specialDir':
    case 'exportDir':
    case 'filterColumn':
    case 'filterValue':
    case 'orderColumn':
    case 'orderDirection':
    case 'pageDir':
    case 'row':
      return true;

    case 'infoFile':
    case 'exportPageFile':
    case 'indexFile':
    case 'column':
    case 'rowFile':
      return false;
  }
}

// ---------------------------------------------------------------------------
// parsePkDisplay — decode primary key display names
//
// Single-PK: the directory name is the percent-encoded value.
// Multi-PK:  "col1=val1,col2=val2" where each val is percent-encoded.
// ---------------------------------------------------------------------------

export function parsePkDisplay(display: string, pkColumns: string[]): string[] {
  if (pkColumns.length === 1) {
    return [decodeURIComponent(display)];
  }

  // Multi-PK: split on "," then extract values from "col=val" pairs
  const pairs = display.split(',');
  const values: string[] = [];

  for (const pair of pairs) {
    const eqIdx = pair.indexOf('=');
    if (eqIdx === -1) {
      // Malformed — return the raw decoded pair as a best-effort value
      values.push(decodeURIComponent(pair));
    } else {
      values.push(decodeURIComponent(pair.slice(eqIdx + 1)));
    }
  }

  return values;
}
