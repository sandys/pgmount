# pgmount

Mount PostgreSQL databases as virtual filesystems. Browse schemas, tables, rows, and columns as directories and files using standard shell commands.

```
$ pgmount mount -c "host=localhost dbname=myapp" /mnt/db

$ ls /mnt/db/public/users/page_1/
1  2  3

$ cat /mnt/db/public/users/page_1/1/row.json
{ "id": 1, "name": "Alice", "email": "alice@example.com" }

$ cat /mnt/db/public/users/.filter/active/true/1/name
Alice
```

pgmount also provides **writable workspaces** — a read-write FUSE filesystem backed by PostgreSQL, designed for AI agents (Claude Code, Codex, etc.) that need persistent `~/.claude/` state across container restarts.

## Features

- **Browse database structure** as a directory tree: schemas / tables / rows / columns
- **Filter rows** with `.filter/<column>/<value>/` — targeted queries, no pagination needed
- **Sort rows** with `.order/<column>/asc/` or `.order/<column>/desc/`
- **Export data** in JSON, CSV, YAML via `.export/data.json/page_N.json`
- **Inspect metadata** via `.info/columns.json`, `.info/schema.sql`, `.info/count`
- **Paginated row listing** — rows grouped into `page_N/` directories (default 1000)
- **Connection pooling**, **statement timeout**, **metadata caching** with configurable TTL
- **Writable workspaces** — persistent agent state stored in PostgreSQL, mountable anywhere
- **OpenShell sandbox** — pre-built container image with database at `/db` and workspace at `/home/agent`

## Installation

**Requirements:** Rust 1.85+, FUSE 3 (`libfuse3-dev`), PostgreSQL client libraries (`libpq-dev`)

```bash
cargo build --release
sudo cp target/release/pgmount /usr/local/bin/
```

## Usage

### Mount a database (read-only)

```bash
# Connection string
pgmount mount -c "host=localhost user=postgres dbname=myapp" /mnt/db

# PostgreSQL URI
pgmount mount -c "postgres://user:pass@localhost/myapp" /mnt/db

# Environment variable
export PGMOUNT_DATABASE_URL="host=localhost dbname=myapp"
pgmount mount /mnt/db
```

### Mount options

```
pgmount mount [OPTIONS] <MOUNT_POINT>

  -c, --connection <STRING>        PostgreSQL connection string
  -s, --schemas <LIST>             Only show these schemas (comma-separated)
      --cache-ttl <SECONDS>        Metadata cache TTL [default: 30]
      --page-size <N>              Max rows per page directory [default: 1000]
      --statement-timeout <SECS>   SQL statement timeout [default: 30]
      --skip-migrations            Skip automatic database migrations
  -f, --foreground                 Run in foreground
```

### Browse and query

```bash
# Discover structure
ls /mnt/db/                                    # schemas
ls /mnt/db/public/                             # tables
cat /mnt/db/public/users/.info/columns.json    # column definitions
cat /mnt/db/public/users/.info/count           # row count

# Read data
cat /mnt/db/public/users/page_1/42/row.json    # full row as JSON
cat /mnt/db/public/users/page_1/42/email       # single column value

# Filter (targeted DB query — fast)
cat /mnt/db/public/users/.filter/id/42/42/row.json
ls /mnt/db/public/users/.filter/active/true/

# Sort
ls /mnt/db/public/users/.order/name/asc/

# Export
cat /mnt/db/public/users/.export/data.csv/page_1.csv
cat /mnt/db/public/users/.export/data.json/page_*.json | jq -s 'add'
```

### Unmount

```bash
pgmount unmount /mnt/db
# or: fusermount -u /mnt/db
```

## Workspaces

Workspaces provide a **read-write** FUSE filesystem backed by PostgreSQL. Files written to the mount point are transparently stored in the `_pgmount.workspace_files` table and persist across unmount/remount cycles.

**Primary use case:** AI agents running in sandboxed containers that need persistent `HOME` directories — config, memory, plans, session transcripts, and other state that would otherwise be lost on restart.

### Create and mount

```bash
# Create a workspace with pre-configured directories
pgmount workspace create agent-1 \
  --display-name "My Agent" \
  --config '{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions"]}'

# Mount as a read-write filesystem
pgmount workspace mount agent-1 /home/agent

# Use it — all I/O transparently goes to PostgreSQL
echo "hello" > /home/agent/.claude/test.txt
cat /home/agent/.claude/test.txt   # → hello
mkdir /home/agent/plans
```

### Persistence

```bash
# Unmount
fusermount -u /home/agent

# Remount — data is still there
pgmount workspace mount agent-1 /home/agent
cat /home/agent/.claude/test.txt   # → hello

# Verify in PostgreSQL directly
psql -c "SELECT path, size FROM _pgmount.workspace_files WHERE workspace_id='agent-1';"
```

### Manage workspaces

```bash
pgmount workspace list                          # list all workspaces
pgmount workspace seed agent-1 --from ./data/   # seed from local directory
pgmount workspace delete agent-1                # delete workspace + all files
```

### Workspace config

The `--config` JSON supports:

```json
{
  "auto_dirs": [".claude", ".claude/memory", ".claude/plans"],
  "seed_files": {
    ".claude/settings.json": "{\"model\": \"sonnet\"}",
    ".bashrc": "export PS1='agent> '"
  }
}
```

- `auto_dirs` — directories created automatically on first mount
- `seed_files` — files pre-populated with the given content

## Claude Code Integration

pgmount workspaces are designed to work with [Claude Code](https://claude.ai/claude-code). When an agent runs with `HOME` pointing to a pgmount workspace, Claude Code's entire `~/.claude/` directory (memory, plans, tasks, sessions, settings) persists in PostgreSQL.

### Standalone setup

```bash
# Set your API key
export ANTHROPIC_API_KEY="sk-ant-..."

# Create and mount a workspace
export PGMOUNT_DATABASE_URL="postgres://user:pass@localhost/mydb"
pgmount workspace create my-agent --config '{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions",".claude/tasks"]}'
pgmount workspace mount my-agent /home/agent

# Run Claude Code with HOME on the workspace
HOME=/home/agent claude -p "Create a plan for my project" --model claude-sonnet-4-6
```

### In a sandbox container

```bash
openshell sandbox create --from pgmount \
  -e PGMOUNT_DATABASE_URL="postgres://user:pass@db/myapp" \
  -e PGMOUNT_WORKSPACE_ID="agent-42" \
  -e PGMOUNT_WORKSPACE_CONFIG='{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions"]}' \
  -e ANTHROPIC_API_KEY="sk-ant-..." \
  -- pgmount-start.sh claude
```

The entrypoint automatically creates the workspace (if new), mounts it at `/home/agent`, and sets `HOME=/home/agent` before launching the agent.

### What persists

Everything Claude Code writes under `~/.claude/`:

| Path | Content |
|------|---------|
| `~/.claude/memory/` | Remembered context across conversations |
| `~/.claude/plans/` | Implementation plans |
| `~/.claude/tasks/` | Task tracking |
| `~/.claude/sessions/` | Conversation transcripts |
| `~/.claude/settings.json` | User preferences |
| `~/.claude.json` | Authentication state |

All stored as rows in `_pgmount.workspace_files` — one row per file, content in BYTEA.

## OpenShell Sandbox

A pre-built container image for running AI agents with database access. See [sandboxes/pgmount/README.md](sandboxes/pgmount/README.md) for full documentation.

```bash
openshell sandbox build pgmount
openshell sandbox create --from pgmount \
  -e PGMOUNT_DATABASE_URL="postgres://readonly:pass@db.example.com/myapp" \
  -- pgmount-start.sh openclaw-start
```

The sandbox mounts the database read-only at `/db` and optionally mounts a writable workspace at `/home/agent`. A Landlock security policy restricts filesystem access.

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PGMOUNT_DATABASE_URL` | *(required)* | PostgreSQL connection string |
| `PGMOUNT_SCHEMAS` | all | Comma-separated schema filter |
| `PGMOUNT_PAGE_SIZE` | 1000 | Rows per page directory |
| `PGMOUNT_CACHE_TTL` | 30 | Metadata cache TTL in seconds |
| `PGMOUNT_STATEMENT_TIMEOUT` | 30 | SQL query timeout in seconds |
| `PGMOUNT_WORKSPACE_ID` | *(optional)* | Enables workspace mount |
| `PGMOUNT_WORKSPACE_MOUNT` | `/home/agent` | Workspace mount point |
| `PGMOUNT_WORKSPACE_CONFIG` | `{}` | JSON config for auto_dirs/seed_files |

## Migrations

On first mount, pgmount creates an internal `_pgmount` schema for audit logging, cache hints, and workspace storage. Migrations are managed by [refinery](https://github.com/rust-db/refinery).

The database user needs write access to `_pgmount`:

```sql
GRANT ALL ON SCHEMA _pgmount TO your_role;
GRANT ALL ON ALL TABLES IN SCHEMA _pgmount TO your_role;
```

To skip migrations: `--skip-migrations`.

## Development

All builds and tests run inside Docker containers:

```bash
docker compose up -d                                        # start dev + postgres
docker compose exec dev cargo build                         # build
docker compose exec dev cargo test -p pgmount-core          # run tests
docker compose exec -e PGPASSWORD=pgmount dev bash tests/test_fuse_mount.sh  # FUSE tests
docker compose exec dev cargo clippy                        # lint
docker compose down                                         # stop
```

| Service | Description |
|---------|-------------|
| `dev` | Rust 1.85 with FUSE3, `/dev/fuse`, `SYS_ADMIN` |
| `postgres` | PostgreSQL 16 (`pgmount:pgmount@postgres/testdb`) |

## License

MIT
