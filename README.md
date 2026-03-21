# pgmount

Mount PostgreSQL databases as virtual filesystems using FUSE.

Browse schemas, tables, rows, and columns as directories and files. Filter, sort, and export data using standard shell commands.

```
$ pgmount mount -c "host=localhost dbname=myapp" /mnt/db

$ ls /mnt/db/public/users/
.export  .filter  .indexes  .info  .order  page_1

$ ls /mnt/db/public/users/page_1/
1  2  3

$ cat /mnt/db/public/users/page_1/1/name
Alice

$ cat /mnt/db/public/users/page_1/1/row.json
{
  "id": 1,
  "name": "Alice",
  "email": "alice@example.com",
  "age": 30,
  "active": true
}

$ ls /mnt/db/public/users/.filter/active/true/
1  3

$ cat /mnt/db/public/users/.export/data.csv/page_1.csv
id,name,email,age,active
1,Alice,alice@example.com,30,true
2,Bob,bob@example.com,25,false
3,Charlie,charlie@example.com,35,true
```

## Features

- **Browse database structure** as a directory tree: schemas / tables / rows / columns
- **Paginated row listing** — rows grouped into `page_N/` directories (configurable page size, default 1000)
- **Read column values** as plain text files
- **Row serialization** in JSON, CSV, and YAML (`row.json`, `row.csv`, `row.yaml`)
- **Paginated bulk export** via `.export/data.json/page_N.json`, `.export/data.csv/page_N.csv`, etc.
- **Filter rows** with `.filter/<column>/<value>/` directories
- **Sort rows** with `.order/<column>/asc/` or `.order/<column>/desc/`
- **Inspect metadata** via `.info/columns.json`, `.info/schema.sql`, `.info/count`, `.info/primary_key`
- **View indexes** via `.indexes/<index_name>` files
- **Composite primary keys** displayed as `col1=val1,col2=val2` directories
- **Percent-encoded PK values** — special characters (`/`, `,`, `=`, `%`) in PK values are safely encoded
- **NULL handling** — NULL values read as `NULL`
- **Tables/columns with special characters** (spaces, quotes) handled correctly
- **Metadata caching** with configurable TTL
- **Connection pooling** via deadpool-postgres (16 connections)
- **Statement timeout** — configurable per-query timeout prevents hung filesystems (default 30s)
- **Multiple schemas** — all non-system schemas mounted, or filter with `--schemas`
- **Automatic migrations** — creates `_pgmount` internal schema on first mount for audit logging and cache hints (via refinery)
- **OpenShell sandbox** — pre-built sandbox image for running AI agents with database access at `/db`

## Filesystem Layout

```
/mnt/db/
  <schema>/                              # e.g. public/
    <table>/                             # e.g. users/
      .info/
        columns.json                     # column metadata as JSON array
        schema.sql                       # approximate CREATE TABLE DDL
        count                            # exact row count
        primary_key                      # PK column name(s)
      .export/
        data.json/                       # export directory (paginated)
          page_1.json                    # rows 1-1000 as JSON array
          page_2.json                    # rows 1001-2000
          ...
        data.csv/
          page_1.csv
          ...
        data.yaml/
          page_1.yaml
          ...
      .filter/
        <column>/                        # e.g. active/
          <value>/                       # e.g. true/
            <pk>/...                     # matching row directories
      .order/
        <column>/                        # e.g. name/
          asc/                           # rows sorted ascending
            <pk>/...
          desc/                          # rows sorted descending
            <pk>/...
      .indexes/
        <index_name>                     # index metadata file
      page_1/                            # rows 1-1000
        <pk_value>/                      # e.g. 1/ or col1=val1,col2=val2/
          <column_name>                  # column value as text file
          row.json                       # full row as JSON
          row.csv                        # full row as CSV
          row.yaml                       # full row as YAML
      page_2/                            # rows 1001-2000
        ...
```

## Installation

### Requirements

- Rust 1.85+
- FUSE 3 (`libfuse3-dev` on Debian/Ubuntu, `fuse3` on Fedora/Arch)
- PostgreSQL client libraries (`libpq-dev`)

### Build from source

```bash
cargo build --release
sudo cp target/release/pgmount /usr/local/bin/
```

## Usage

### Mount a database

```bash
# Using a connection string
pgmount mount -c "host=localhost user=postgres dbname=myapp" /mnt/db

# Using a PostgreSQL URI
pgmount mount -c "postgres://user:pass@localhost/myapp" /mnt/db

# Using environment variable
export PGMOUNT_DATABASE_URL="host=localhost dbname=myapp"
pgmount mount /mnt/db
```

### Options

```
pgmount mount [OPTIONS] <MOUNT_POINT>

Arguments:
  <MOUNT_POINT>    Path where the filesystem will be mounted

Options:
  -c, --connection <CONNECTION>         PostgreSQL connection string
  -s, --schemas <SCHEMAS>               Only show these schemas (comma-separated)
      --cache-ttl <SECONDS>             Metadata cache TTL [default: 30]
      --page-size <N>                   Max rows per page directory [default: 1000]
      --statement-timeout <SECONDS>     SQL statement timeout [default: 30]
      --read-only <BOOL>                Mount read-only [default: true]
      --skip-migrations                 Skip automatic database migrations
  -f, --foreground                      Run in foreground
```

### Unmount

```bash
pgmount unmount /mnt/db
# or
fusermount -u /mnt/db
```

### List active mounts

```bash
pgmount list
```

### Browsing examples

Rows are organized into paginated `page_N/` directories under each table. Use `.filter/` for targeted access to specific rows.

```bash
# List schemas
ls /mnt/db/

# List tables in a schema
ls /mnt/db/public/

# List pages in a table
ls /mnt/db/public/users/
# output: .export  .filter  .indexes  .info  .order  page_1  page_2

# List rows in page 1
ls /mnt/db/public/users/page_1/

# Read a column value
cat /mnt/db/public/users/page_1/42/email

# Get full row as JSON
cat /mnt/db/public/users/page_1/42/row.json
```

**Accessing a specific row directly** — use `.filter/` instead of browsing pages:

```bash
# Find user with id=42 (targeted DB query, no pagination needed)
cat /mnt/db/public/users/.filter/id/42/42/row.json

# Find all active users
ls /mnt/db/public/users/.filter/active/true/

# Find across all pages with shell globbing
cat /mnt/db/public/users/page_*/42/row.json 2>/dev/null
```

**Exporting data:**

```bash
# Export page 1 as CSV
cat /mnt/db/public/users/.export/data.csv/page_1.csv > users_page1.csv

# Export all pages (concatenate)
cat /mnt/db/public/users/.export/data.json/page_*.json

# Export as a single stream with jq
cat /mnt/db/public/users/.export/data.json/page_*.json | jq -s 'add'
```

**Metadata and indexes:**

```bash
# View table metadata
cat /mnt/db/public/users/.info/count
cat /mnt/db/public/users/.info/primary_key
cat /mnt/db/public/users/.info/schema.sql
cat /mnt/db/public/users/.info/columns.json

# Sort rows by column
ls /mnt/db/public/users/.order/name/asc/

# Read sorted row data
cat /mnt/db/public/users/.order/name/asc/1/row.json

# View indexes
ls /mnt/db/public/users/.indexes/
cat /mnt/db/public/users/.indexes/users_pkey
```

## Migrations

On first mount, pgmount automatically creates an internal `_pgmount` schema in the target database with tables for audit logging and cache hints. Migrations are managed by [refinery](https://github.com/rust-db/refinery) and run before the FUSE mount is created.

```
_pgmount.schema_version   — migration tracking
_pgmount.mount_log        — audit log of mount sessions (mount point, schemas, page size, version)
_pgmount.cache_hints      — persistent cache hints (per schema/table)
```

Each mount session is recorded in `mount_log`. To skip migrations (e.g., when the database user lacks CREATE privileges), use `--skip-migrations`.

The database user needs write access to the `_pgmount` schema even though the FUSE mount is read-only:

```sql
GRANT ALL ON SCHEMA _pgmount TO your_readonly_role;
GRANT ALL ON ALL TABLES IN SCHEMA _pgmount TO your_readonly_role;
```

## OpenShell Sandbox

A pre-built sandbox for running AI agents (OpenClaw) with database access is available at `sandboxes/pgmount/`. See [sandboxes/pgmount/README.md](sandboxes/pgmount/README.md) for setup instructions.

```bash
openshell sandbox build pgmount
openshell sandbox create --from pgmount \
  -e PGMOUNT_DATABASE_URL="postgres://readonly:pass@db.example.com/myapp" \
  -- pgmount-start.sh openclaw-start
```

The sandbox mounts the database at `/db`, provides a Landlock security policy, and includes an agent skill (`pgmount-navigate`) that teaches the agent how to browse the filesystem.

## Architecture

```
pgmount/
  crates/
    pgmount/          # CLI binary
    pgmount-core/     # Library
      migrations/      # Refinery SQL migrations (V1, V2, V3)
      src/
        cli/           # Clap command definitions
        config/        # Connection string resolution, YAML config
        db/            # Connection pool, SQL queries, and migrations
          queries/     # Introspection, row access, indexes, stats
        fs/            # FUSE filesystem implementation
          nodes/       # Node types: root, schema, table, page, row, column,
                       #   info, export, indexes, filter, order
        format/        # JSON, CSV, YAML serializers
        mount/         # Mount registry
  sandboxes/
    pgmount/           # OpenShell sandbox definition
```

| Layer | Crate | Purpose |
|-------|-------|---------|
| FUSE | `fuser` 0.17 | Kernel filesystem interface |
| PostgreSQL | `tokio-postgres` + `deadpool-postgres` | Async queries with connection pooling |
| Async | `tokio` | Runtime for database operations |
| CLI | `clap` v4 | Command-line argument parsing |
| Caching | `dashmap` | Lock-free concurrent inode table and metadata cache |
| Serialization | `serde_json`, `csv`, `serde_yml` | Row format output |
| Errors | `thiserror` | Ergonomic error types with errno mapping |
| Migrations | `refinery` | Embedded SQL migrations for `_pgmount` schema |
| Logging | `tracing` | Structured, filterable logging |

### Key design decisions

**Pagination**: Rows are grouped into `page_N/` directories to bound memory usage and directory listing size. Each page contains up to `page_size` rows (default 1000). Export files are similarly paginated. Use `.filter/` for targeted access to specific rows without browsing pages.

**Async bridge**: `fuser` callbacks run on OS threads; database calls are async. Each FUSE callback uses `tokio::runtime::Handle::block_on()` to execute async queries.

**Inode allocation**: Lazy and deterministic within a mount session. A `NodeIdentity` enum describes every virtual node type. A `DashMap` ensures the same identity always maps to the same inode number.

**File content**: `getattr` reports an estimated size (4096). On `open`, the full content is generated and cached in a file-handle map. `read` slices from this cache. If the file doesn't exist (e.g., nonexistent row), `open` returns ENOENT.

**Type handling**: All column values are cast to `::text` in SQL, avoiding Rust type-mapping issues with PostgreSQL types like NUMERIC, MONEY, or custom domains.

**PK encoding**: Primary key values are percent-encoded in directory names so that characters like `/`, `,`, `=` don't break filesystem paths. Integer PKs appear as-is (no special characters to encode).

**Statement timeout**: A configurable timeout (default 30s) prevents runaway queries from hanging the filesystem. Set via `--statement-timeout`.

## Development

All development runs inside Docker containers:

```bash
# Start the dev environment (Rust 1.85 + PostgreSQL 16)
docker compose up -d

# Build inside the container
docker compose exec dev cargo build

# Run Rust unit/integration tests (38 tests)
docker compose exec dev cargo test -p pgmount-core

# Run FUSE mount integration tests (119 assertions)
docker compose exec -e PGPASSWORD=pgmount dev bash tests/test_fuse_mount.sh

# Run clippy
docker compose exec dev cargo clippy

# Stop everything
docker compose down
```

### Docker Compose services

| Service | Description |
|---------|-------------|
| `dev` | Rust 1.85 with FUSE3, mounted with `/dev/fuse` and `SYS_ADMIN` capability |
| `postgres` | PostgreSQL 16 (`pgmount:pgmount@postgres/testdb`) |

## License

MIT
