import { describe, it, expect } from 'vitest';
import { parsePath, isDirectory, parsePkDisplay } from './path-parser.js';

describe('parsePath', () => {
  it('parses root', () => {
    expect(parsePath('/')).toEqual({ type: 'root' });
  });

  it('parses schema', () => {
    expect(parsePath('/public')).toEqual({ type: 'schema', schema: 'public' });
    expect(parsePath('/test_schema')).toEqual({ type: 'schema', schema: 'test_schema' });
  });

  it('parses table', () => {
    expect(parsePath('/public/users')).toEqual({ type: 'table', schema: 'public', table: 'users' });
  });

  it('parses special dirs', () => {
    expect(parsePath('/public/users/.info')).toEqual(
      { type: 'specialDir', schema: 'public', table: 'users', kind: 'info' },
    );
    expect(parsePath('/public/users/.export')).toEqual(
      { type: 'specialDir', schema: 'public', table: 'users', kind: 'export' },
    );
    expect(parsePath('/public/users/.filter')).toEqual(
      { type: 'specialDir', schema: 'public', table: 'users', kind: 'filter' },
    );
    expect(parsePath('/public/users/.order')).toEqual(
      { type: 'specialDir', schema: 'public', table: 'users', kind: 'order' },
    );
    expect(parsePath('/public/users/.indexes')).toEqual(
      { type: 'specialDir', schema: 'public', table: 'users', kind: 'indexes' },
    );
  });

  it('parses info files', () => {
    expect(parsePath('/public/users/.info/columns.json')).toEqual(
      { type: 'infoFile', schema: 'public', table: 'users', filename: 'columns.json' },
    );
    expect(parsePath('/public/users/.info/schema.sql')).toEqual(
      { type: 'infoFile', schema: 'public', table: 'users', filename: 'schema.sql' },
    );
    expect(parsePath('/public/users/.info/count')).toEqual(
      { type: 'infoFile', schema: 'public', table: 'users', filename: 'count' },
    );
    expect(parsePath('/public/users/.info/primary_key')).toEqual(
      { type: 'infoFile', schema: 'public', table: 'users', filename: 'primary_key' },
    );
  });

  it('parses page dirs', () => {
    expect(parsePath('/public/users/page_1')).toEqual(
      { type: 'pageDir', schema: 'public', table: 'users', page: 1 },
    );
    expect(parsePath('/public/users/page_42')).toEqual(
      { type: 'pageDir', schema: 'public', table: 'users', page: 42 },
    );
  });

  it('parses row', () => {
    expect(parsePath('/public/users/page_1/123')).toEqual(
      { type: 'row', schema: 'public', table: 'users', pkDisplay: '123' },
    );
  });

  it('parses column file', () => {
    expect(parsePath('/public/users/page_1/123/name')).toEqual(
      { type: 'column', schema: 'public', table: 'users', pkDisplay: '123', column: 'name' },
    );
  });

  it('parses row format files', () => {
    expect(parsePath('/public/users/page_1/123/row.json')).toEqual(
      { type: 'rowFile', schema: 'public', table: 'users', pkDisplay: '123', format: 'json' },
    );
    expect(parsePath('/public/users/page_1/123/row.csv')).toEqual(
      { type: 'rowFile', schema: 'public', table: 'users', pkDisplay: '123', format: 'csv' },
    );
    expect(parsePath('/public/users/page_1/123/row.yaml')).toEqual(
      { type: 'rowFile', schema: 'public', table: 'users', pkDisplay: '123', format: 'yaml' },
    );
  });

  it('parses export dirs and files', () => {
    expect(parsePath('/public/users/.export/data.json')).toEqual(
      { type: 'exportDir', schema: 'public', table: 'users', format: 'json' },
    );
    expect(parsePath('/public/users/.export/data.json/page_1.json')).toEqual(
      { type: 'exportPageFile', schema: 'public', table: 'users', format: 'json', page: 1 },
    );
  });

  it('parses filter pipeline', () => {
    expect(parsePath('/public/users/.filter/status')).toEqual(
      { type: 'filterColumn', schema: 'public', table: 'users', column: 'status' },
    );
    expect(parsePath('/public/users/.filter/status/active')).toEqual(
      { type: 'filterValue', schema: 'public', table: 'users', column: 'status', value: 'active' },
    );
  });

  it('parses order pipeline', () => {
    expect(parsePath('/public/users/.order/name')).toEqual(
      { type: 'orderColumn', schema: 'public', table: 'users', column: 'name' },
    );
    expect(parsePath('/public/users/.order/name/asc')).toEqual(
      { type: 'orderDirection', schema: 'public', table: 'users', column: 'name', dir: 'asc' },
    );
    expect(parsePath('/public/users/.order/name/desc')).toEqual(
      { type: 'orderDirection', schema: 'public', table: 'users', column: 'name', dir: 'desc' },
    );
  });

  it('parses index files', () => {
    expect(parsePath('/public/users/.indexes/users_pkey')).toEqual(
      { type: 'indexFile', schema: 'public', table: 'users', indexName: 'users_pkey' },
    );
  });

  it('returns null for unrecognized paths', () => {
    expect(parsePath('/public/users/page_1/123/extra/deep')).toBeNull();
    expect(parsePath('')).not.toBeNull(); // empty = root
  });
});

describe('isDirectory', () => {
  it('root is directory', () => {
    expect(isDirectory({ type: 'root' })).toBe(true);
  });
  it('schema is directory', () => {
    expect(isDirectory({ type: 'schema', schema: 'public' })).toBe(true);
  });
  it('column is file', () => {
    expect(isDirectory({ type: 'column', schema: 'p', table: 't', pkDisplay: '1', column: 'c' })).toBe(false);
  });
  it('infoFile is file', () => {
    expect(isDirectory({ type: 'infoFile', schema: 'p', table: 't', filename: 'count' })).toBe(false);
  });
});

describe('parsePkDisplay', () => {
  it('single PK', () => {
    expect(parsePkDisplay('123', ['id'])).toEqual(['123']);
  });
  it('single PK with percent-encoding', () => {
    expect(parsePkDisplay('hello%2Fworld', ['name'])).toEqual(['hello/world']);
  });
  it('multi PK', () => {
    expect(parsePkDisplay('a=1,b=2', ['a', 'b'])).toEqual(['1', '2']);
  });
});
