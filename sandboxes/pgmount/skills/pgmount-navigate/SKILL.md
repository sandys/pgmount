---
name: pgmount-navigate
description: Navigate a PostgreSQL database mounted as a filesystem at /db
---

# Navigating PostgreSQL via /db

A PostgreSQL database is mounted as a read-only filesystem at `/db`. You can explore it using standard file tools — no SQL needed.

## Filesystem Layout

```
/db/
  <schema>/
    <table>/
      .info/
        columns.json    # Column names, types, nullability
        schema.sql      # CREATE TABLE statement
        count           # Row count
        primary_key     # Primary key column(s)
      .export/
        data.json/
          page_1.json   # All rows as JSON (paginated)
        data.csv/
          page_1.csv    # All rows as CSV (paginated)
      .filter/
        <column>/<value>/<pk>/row.json   # Filter rows by column value
      .order/
        <column>/asc/   # Rows sorted ascending
        <column>/desc/  # Rows sorted descending
      .indexes/
        <index_name>    # Index definitions
      page_1/
        <pk_value>/
          <column>      # Individual column value as plain text
          row.json      # Full row as JSON
          row.csv       # Full row as CSV
          row.yaml      # Full row as YAML
      page_2/
        ...
```

## Common Operations

### Discover what's available
```bash
ls /db/                           # List schemas
ls /db/public/                    # List tables in a schema
```

### Inspect table structure
```bash
cat /db/public/users/.info/columns.json   # Column definitions
cat /db/public/users/.info/count          # Row count
cat /db/public/users/.info/primary_key    # Primary key
```

### Look up specific rows (preferred for targeted access)
```bash
cat /db/public/users/.filter/id/42/42/row.json        # Find user with id=42
cat /db/public/users/.filter/email/alice@example.com/  # Find by email
```

### Browse rows page by page
```bash
ls /db/public/users/page_1/           # List row directories (by primary key)
cat /db/public/users/page_1/1/row.json   # Read full row
cat /db/public/users/page_1/1/name       # Read single column value
```

### Export data
```bash
cat /db/public/users/.export/data.csv/page_1.csv    # CSV export
cat /db/public/users/.export/data.json/page_1.json  # JSON export
```

### Sort results
```bash
ls /db/public/users/.order/created_at/desc/page_1/   # Newest first
```

### Search within rows
```bash
grep -r "pattern" /db/public/users/page_1/   # Search page content
```

## Important Notes

- **Read-only**: You cannot create, modify, or delete data. Any write attempt returns "Read-only file system".
- **Check count first**: Before scanning a table, read `.info/count`. Tables with millions of rows will have thousands of pages — do not `ls` them all.
- **Use .filter/ for lookups**: If you know what you're looking for, `.filter/` is much faster than browsing pages.
- **Page size**: Each `page_N/` directory contains up to 1000 rows by default (configurable via `PGMOUNT_PAGE_SIZE`).
- **Composite primary keys**: Rows with composite PKs use the format `col1=val1,col2=val2` as directory names.
- **NULL values**: NULL column values appear as empty files.
