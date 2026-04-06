export interface SchemaInfo {
  name: string;
}

export interface TableInfo {
  name: string;
  tableType: string; // "BASE TABLE" or "VIEW"
}

export interface ColumnInfo {
  name: string;
  dataType: string;
  isNullable: boolean;
  columnDefault: string | null;
  ordinalPosition: number;
}

export interface PrimaryKeyInfo {
  columnNames: string[];
}

export interface IndexInfo {
  name: string;
  isUnique: boolean;
  isPrimary: boolean;
  definition: string;
  columns: string[];
}

export interface RowIdentifier {
  pkValues: [string, string][]; // [columnName, valueAsString]
  displayName: string; // For directory name: "pk_value" or "pk1=v1,pk2=v2"
}

export interface WorkspaceFile {
  workspaceId: string;
  path: string;
  parentPath: string;
  name: string;
  isDir: boolean;
  content: Buffer | null;
  mode: number;
  size: number;
  mtimeNs: bigint;
  ctimeNs: bigint;
  atimeNs: bigint;
  nlink: number;
  uid: number;
  gid: number;
}
