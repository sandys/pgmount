---
name: openeral-dev
description: Use when working on the openeral project — building, testing, debugging, adding features, or understanding the codebase. openeral mounts PostgreSQL databases as FUSE virtual filesystems.
disable-model-invocation: false
user-invocable: true
allowed-tools: Read, Grep, Glob, Bash
argument-hint: [task description]
---

# OpenEral Development

OpenEral (openeral) is a Rust project that mounts PostgreSQL databases as FUSE virtual filesystems. It has two filesystem implementations:

1. **PgmountFilesystem** (read-only) — browse schemas, tables, rows, and columns as directories and files
2. **WorkspaceFilesystem** (read-write) — persistent agent state stored in PostgreSQL, designed for Claude Code's `~/.claude/` directory

## Project Structure

```
openeral/
├── Cargo.toml                          # Workspace root
├── Dockerfile.dev                      # Rust 1.85 + FUSE3 + libpq
├── docker-compose.yml                  # dev (Rust) + postgres (PG 16) services
├── crates/
│   ├── openeral/                        # Binary crate (CLI entry point)
│   │   └── src/main.rs                 # Tokio main, tracing init, calls cli::run()
│   └── openeral-core/                   # Library crate (all logic)
│       ├── migrations/                 # Refinery SQL migrations
│       │   ├── V1__create_openeral_schema.sql  # _openeral schema + schema_version
│       │   ├── V2__create_mount_log.sql       # Mount session audit log
│       │   ├── V3__create_cache_hints.sql     # Persistent cache hints
│       │   └── V4__create_workspace_tables.sql # workspace_config + workspace_files
│       ├── src/
│       │   ├── lib.rs                  # Module declarations
│       │   ├── error.rs                # FsError enum → fuser::Errno mapping
│       │   ├── cli/                    # Clap v4 commands
│       │   │   ├── mod.rs              # Cli struct, Commands enum, run()
│       │   │   ├── mount.rs            # Mount subcommand (pool + migrations + fuser::mount2)
│       │   │   ├── workspace.rs        # Workspace subcommands (create/mount/seed/list/delete)
│       │   │   ├── unmount.rs          # fusermount -u wrapper
│       │   │   ├── list.rs             # Reads /proc/mounts for openeral entries
│       │   │   └── version.rs          # Prints CARGO_PKG_VERSION
│       │   ├── config/                 # Connection resolution
│       │   │   ├── mod.rs
│       │   │   ├── types.rs            # MountConfig, WorkspaceMountConfig
│       │   │   └── connection.rs       # CLI arg > env var > ~/.openeral/config.yml
│       │   ├── db/                     # PostgreSQL layer
│       │   │   ├── mod.rs
│       │   │   ├── migrate.rs          # run_migrations() + log_mount_session() via refinery
│       │   │   ├── pool.rs             # deadpool-postgres pool (max 16, statement timeout)
│       │   │   ├── types.rs            # SchemaInfo, TableInfo, ColumnInfo, WorkspaceFile, etc.
│       │   │   └── queries/
│       │   │       ├── mod.rs          # Public quote_ident(), get_client()
│       │   │       ├── introspection.rs # list_schemas/tables/columns, get_primary_key
│       │   │       ├── rows.rs         # query_rows, list_rows, get_row_data, get_all_rows_as_text
│       │   │       ├── indexes.rs      # list_indexes from pg_class/pg_index
│       │   │       ├── stats.rs        # Row count estimate + exact
│       │   │       └── workspace.rs    # Workspace CRUD, file ops, seeding, rename
│       │   ├── fs/                     # FUSE filesystem
│       │   │   ├── mod.rs              # PgmountFilesystem (read-only, impl fuser::Filesystem)
│       │   │   ├── workspace.rs        # WorkspaceFilesystem (read-write, impl fuser::Filesystem)
│       │   │   ├── workspace_inode.rs  # Path-based inode table for workspaces
│       │   │   ├── inode.rs            # InodeTable + NodeIdentity enum (read-only mount)
│       │   │   ├── attr.rs             # FileAttr helpers (dir_attr, file_attr)
│       │   │   ├── cache.rs            # MetadataCache with TTL
│       │   │   └── nodes/              # One file per virtual node type
│       │   │       ├── mod.rs          # Dispatch: node_lookup/readdir/read/getattr
│       │   │       ├── root.rs         # / → lists schemas
│       │   │       ├── schema.rs       # /public/ → lists tables
│       │   │       ├── table.rs        # /public/users/ → special dirs + page_N/ dirs
│       │   │       ├── page.rs         # /public/users/page_1/ → rows for that page
│       │   │       ├── row.rs          # /public/users/page_1/1/ → columns + format files
│       │   │       ├── column.rs       # column value as text file + parse_pk_display
│       │   │       ├── row_file.rs     # row.json / row.csv / row.yaml (delegates to format/)
│       │   │       ├── info.rs         # .info/ → columns.json, schema.sql, count, primary_key
│       │   │       ├── export.rs       # .export/ → data.json/, data.csv/, data.yaml/ (paginated)
│       │   │       ├── indexes.rs      # .indexes/ → index metadata files
│       │   │       ├── filter.rs       # .filter/<col>/<val>/ → filtered rows
│       │   │       └── order.rs        # .order/<col>/asc|desc/ → sorted rows
│       │   ├── format/                 # Serializers (single source of truth)
│       │   │   ├── mod.rs
│       │   │   ├── json.rs            # format_row / format_rows (smart type inference)
│       │   │   ├── csv.rs             # CSV with headers
│       │   │   └── yaml.rs            # YAML via serde_yml
│       │   └── mount/
│       │       ├── mod.rs
│       │       └── registry.rs         # MountRegistry (DashMap tracking)
│       └── tests/
│           ├── integration.rs          # Rust integration tests (read-only)
│           └── workspace_integration.rs # Workspace DB operation tests
├── sandboxes/
│   └── openeral/                        # OpenShell sandbox for AI agents
│       ├── Dockerfile                  # Multi-stage: build openeral + extend openclaw base
│       ├── policy.yaml                 # Landlock filesystem + capability policy
│       ├── skills/openeral-navigate/SKILL.md  # Agent skill for /db navigation
│       └── README.md                   # Published-image OpenShell flow and sandbox behavior
└── tests/
    └── test_fuse_mount.sh              # 119-assertion FUSE mount test suite
```

## Key Architecture

### Pagination
Rows are grouped into `page_N/` directories (configurable via `--page-size`, default 1000). This bounds memory and directory listing size. Export files are similarly paginated (`data.json/page_1.json`). Use `.filter/` for targeted access to specific rows without browsing pages.

**One-liner alternatives to direct row access:**
```bash
# Instead of: cat /mnt/db/public/users/42/name
# Use filter for targeted lookup:
cat /mnt/db/public/users/.filter/id/42/42/name
# Or glob across pages:
cat /mnt/db/public/users/page_*/42/name 2>/dev/null
```

### Async Bridge
fuser callbacks are sync (OS threads). Database calls use tokio-postgres (async). Each FUSE callback calls `handle.block_on(async_fn)` to bridge them.

### Inode Allocation
`NodeIdentity` enum describes every virtual node. `InodeTable` (DashMap) maps identity ↔ inode. Same identity = same inode within a session. Root = inode 1.

### Node Dispatch
`fs/nodes/mod.rs` has four dispatch functions: `node_getattr`, `node_lookup`, `node_readdir`, `node_read`. Each matches on `NodeIdentity` and delegates to the appropriate node module.

### Shared Query Function
`db/queries/rows.rs::query_rows()` is the single row-fetching function used by table listing, filter, and order nodes. It accepts optional WHERE and ORDER BY clauses to avoid code duplication. The `get_client()` helper in `db/queries/mod.rs` centralizes connection acquisition.

### File Content Strategy
`getattr` reports estimated size 4096. On `open`, content is generated and cached in an `OpenFileHandle` map. `read` slices from this cache. If `open` fails (e.g., nonexistent row), ENOENT is returned.

### SQL Type Handling
All values cast to `::text` in SQL queries. This avoids Rust type-mapping issues with NUMERIC, MONEY, custom domains, etc.

### PK Encoding
Primary key values are percent-encoded in directory names using the `percent-encoding` crate. Characters `/`, `,`, `=`, `%` are encoded. Integer PKs appear as-is. Decoded on read via `parse_pk_display()`.

### Statement Timeout
Configured via `--statement-timeout` (default 30s). Set at the PostgreSQL connection level via `-c statement_timeout=Ns` in connection options. Prevents runaway queries from hanging the FUSE filesystem.

### WorkspaceFilesystem (read-write)
Separate FUSE implementation in `fs/workspace.rs`. Stores opaque files by path in `_openeral.workspace_files`. Uses `WorkspaceInodeTable` (path ↔ inode via DashMap) — simpler than `NodeIdentity`. Write-back buffering: `open()` loads from DB, `write()` mutates buffer, `flush()`/`release()` writes back in one query.

### Database Migrations
Managed by `refinery` (embed_migrations! macro). SQL files live in `crates/openeral-core/migrations/` (V1–V4). Migrations run automatically in `cli/mount.rs` after connection test, before FUSE mount — creating the `_openeral` schema with `mount_log`, `cache_hints`, `workspace_config`, and `workspace_files` tables. Skip with `--skip-migrations`.

## Development Workflow

**ALL builds and tests run inside Docker containers:**

```bash
# Start environment
docker compose up -d

# Build
docker compose exec dev cargo build

# Run Rust tests (38 tests, uses dedicated rust_test schema)
docker compose exec dev cargo test -p openeral-core

# Run FUSE mount integration tests (119 assertions)
docker compose exec -e PGPASSWORD=pgmount dev bash tests/test_fuse_mount.sh

# Lint
docker compose exec dev cargo clippy

# Manual testing: mount and browse
docker compose exec dev mkdir -p /mnt/db
docker compose exec -d dev /workspace/target/debug/openeral mount \
  -c "host=postgres user=pgmount password=pgmount dbname=testdb" /mnt/db
docker compose exec dev ls /mnt/db/public/users/
docker compose exec dev ls /mnt/db/public/users/page_1/
docker compose exec dev cat /mnt/db/public/users/page_1/1/row.json
docker compose exec dev cat /mnt/db/public/users/.filter/id/1/1/row.json
docker compose exec dev fusermount -u /mnt/db
```

**PostgreSQL test credentials:** `pgmount:pgmount@postgres/testdb`

## Adding a New Node Type

To add a new virtual directory/file type (e.g., `.sample/`):

1. Add variant(s) to `NodeIdentity` enum in `fs/inode.rs`
2. Create `fs/nodes/sample.rs` with `lookup`, `readdir`, and/or `read` functions
3. Add `pub mod sample;` to `fs/nodes/mod.rs`
4. Wire into the dispatch functions in `fs/nodes/mod.rs`:
   - `node_lookup` — handle the parent identity that contains this node
   - `node_readdir` — list children
   - `node_read` — return file content (for leaf files)
   - `node_getattr` — return dir or file attrs
   - `is_directory` — add to the match if it's a directory
5. If it's a special dir under tables, add to `SPECIAL_DIRS` in `fs/nodes/table.rs`
6. Add tests to `tests/test_fuse_mount.sh` and `tests/integration.rs`

## Adding a New SQL Query

1. Add the function to the appropriate file in `db/queries/`
2. Use `super::get_client(pool).await?` for a connection
3. Use parameterized queries (`$1`, `$2`, etc.)
4. Use `super::quote_ident()` for dynamic identifiers
5. Cast results to `::text` when returning user-facing string data
6. For row-listing queries, use `query_rows()` with extra_where/extra_order params

## NodeIdentity Enum (Complete)

```
Root
Schema { name }
Table { schema, table }
SpecialDir { schema, table, kind: Info|Export|Filter|Order|Indexes|... }
PageDir { schema, table, page }          # page_N/ under table
Row { schema, table, pk_display }
Column { schema, table, pk_display, column }
RowFile { schema, table, pk_display, format: json|csv|yaml }
FilterDir { schema, table, stage: Root|Column|Value }
OrderDir { schema, table, stage: Root|Column|Direction }
LimitDir { schema, table, kind: First|Last, n }
ByIndexDir { schema, table, stage: Root|Column|Value }
InfoFile { schema, table, filename }
ExportDir { schema, table, format }      # data.json/ directory
ExportFile { schema, table, format }     # (legacy, kept for compat)
ExportPageFile { schema, table, format, page }  # page_N.json file
IndexDir { schema, table }
IndexFile { schema, table, index_name }
ViewsDir { schema }
View { schema, view_name }
```

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| fuser | 0.17 | FUSE filesystem trait |
| tokio | 1 | Async runtime |
| tokio-postgres | 0.7 | PostgreSQL async driver |
| deadpool-postgres | 0.14 | Connection pooling |
| clap | 4 | CLI argument parsing |
| dashmap | 6 | Lock-free concurrent maps |
| serde_json | 1 | JSON serialization |
| csv | 1 | CSV serialization |
| serde_yml | 0.0.12 | YAML serialization |
| refinery | 0.8 | Embedded SQL migrations |
| thiserror | 2 | Error type derivation |
| tracing | 0.1 | Structured logging |
| chrono | 0.4 | Date/time types |
| percent-encoding | 2 | PK value encoding for safe directory names |
| libc | 0.2 | System call constants |
