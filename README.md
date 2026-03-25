# OpenEral

*Eral (ஏரல்) — Tamil for the lobster family. Like lobsters carry their homes on their backs, OpenEral gives AI agents a persistent home they carry across containers.*

Mount PostgreSQL databases as virtual filesystems. Browse schemas, tables, rows, and columns as directories and files using standard shell commands. Give AI agents a persistent `~/.claude/` directory backed by PostgreSQL that survives container restarts.

```
$ openeral mount -c "host=localhost dbname=myapp" /mnt/db

$ ls /mnt/db/public/users/page_1/
1  2  3

$ cat /mnt/db/public/users/page_1/1/row.json
{ "id": 1, "name": "Alice", "email": "alice@example.com" }

$ cat /mnt/db/public/users/.filter/active/true/1/name
Alice
```

openeral also provides **writable workspaces** — a read-write FUSE filesystem backed by PostgreSQL, designed for AI agents (Claude Code, Codex, etc.) that need persistent `~/.claude/` state across container restarts.

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

### Quick start in OpenShell (supported)

```bash
# Infrastructure assumptions:
# - a gateway is already running with the custom cluster image
# - a generic provider named "$OPENERAL_DB_PROVIDER" already exists on that gateway
#   and points at the live PostgreSQL database
# - the host has ANTHROPIC_API_KEY available
#
# Use the exact sandbox image ref you were given.
export OPENERAL_SANDBOX_IMAGE='<sandbox image ref>'
export OPENERAL_DB_PROVIDER=openeral-db
export OPENERAL_SANDBOX_NAME=openeral-demo
set -a
. ./.env
set +a

# One command: create the sandbox, auto-create the Claude provider from host env,
# mount /db and /home/agent, and run Claude with persistent HOME.
openshell sandbox create \
  --name "$OPENERAL_SANDBOX_NAME" \
  --from "$OPENERAL_SANDBOX_IMAGE" \
  --provider "$OPENERAL_DB_PROVIDER" \
  --provider claude \
  --auto-providers \
  --no-tty -- env HOME=/home/agent claude
```

See [sandboxes/openeral/README.md](sandboxes/openeral/README.md) for the supported sandbox flow.

### Build from source

**Requirements:** Rust 1.85+, FUSE 3 (`libfuse3-dev`), PostgreSQL client libraries (`libpq-dev`)

```bash
cargo build --release
sudo cp target/release/openeral /usr/local/bin/
```

## Usage

### Mount a database (read-only)

```bash
# Connection string
openeral mount -c "host=localhost user=postgres dbname=myapp" /mnt/db

# PostgreSQL URI
openeral mount -c "postgres://user:pass@localhost/myapp" /mnt/db

# Environment variable
export OPENERAL_DATABASE_URL="host=localhost dbname=myapp"
openeral mount /mnt/db
```

### Mount options

```
openeral mount [OPTIONS] <MOUNT_POINT>

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
openeral unmount /mnt/db
# or: fusermount -u /mnt/db
```

## Workspaces

Workspaces provide a **read-write** FUSE filesystem backed by PostgreSQL. Files written to the mount point are transparently stored in the `_openeral.workspace_files` table and persist across unmount/remount cycles.

**Primary use case:** AI agents running in sandboxed containers that need persistent `HOME` directories — config, memory, plans, session transcripts, and other state that would otherwise be lost on restart.

### Create and mount

```bash
# Create a workspace with pre-configured directories
openeral workspace create agent-1 \
  --display-name "My Agent" \
  --config '{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions"]}'

# Mount as a read-write filesystem
openeral workspace mount agent-1 /home/agent

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
openeral workspace mount agent-1 /home/agent
cat /home/agent/.claude/test.txt   # → hello

# Verify in PostgreSQL directly
psql -c "SELECT path, size FROM _openeral.workspace_files WHERE workspace_id='agent-1';"
```

### Manage workspaces

```bash
openeral workspace list                          # list all workspaces
openeral workspace seed agent-1 --from ./data/   # seed from local directory
openeral workspace delete agent-1                # delete workspace + all files
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

openeral workspaces are designed to work with [Claude Code](https://claude.ai/claude-code). When an agent runs with `HOME` pointing to a openeral workspace, Claude Code's entire `~/.claude/` directory (memory, plans, tasks, sessions, settings) persists in PostgreSQL.

### Standalone setup

```bash
# Set your API key
export ANTHROPIC_API_KEY="sk-ant-..."

# Create and mount a workspace
export OPENERAL_DATABASE_URL="postgres://user:pass@localhost/mydb"
openeral workspace create my-agent --config '{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions",".claude/tasks"]}'
openeral workspace mount my-agent /home/agent

# Run Claude Code with HOME on the workspace
HOME=/home/agent claude -p "Create a plan for my project" --model claude-sonnet-4-6
```

### In OpenShell

```bash
# Assumptions:
# - a gateway is already running with the custom cluster image
# - a generic provider named "$OPENERAL_DB_PROVIDER" already exists on that gateway
#   and points at the live PostgreSQL database
# - the host has ANTHROPIC_API_KEY available
export OPENERAL_SANDBOX_IMAGE='<sandbox image ref>'
export OPENERAL_DB_PROVIDER=openeral-db
export OPENERAL_SANDBOX_NAME=openeral-demo
set -a
. ./.env
set +a

# One command: create the sandbox, auto-create the Claude provider from host env,
# mount /db and /home/agent, and run Claude with persistent HOME.
openshell sandbox create \
  --name "$OPENERAL_SANDBOX_NAME" \
  --from "$OPENERAL_SANDBOX_IMAGE" \
  --provider "$OPENERAL_DB_PROVIDER" \
  --provider claude \
  --auto-providers \
  --no-tty -- env HOME=/home/agent claude
```

The custom cluster image deploys the FUSE device plugin and configures the gateway to request `github.com/fuse` for sandbox pods. The sandbox image declares `/db` and `/home/agent` in `/etc/fstab`, and the side-loaded OpenShell supervisor mounts both before launching the child process.

`/db` is read-only. `/home/agent` is read-write and keyed to `OPENSHELL_SANDBOX_ID`, so each sandbox object gets its own persistent workspace. Reconnecting to the same sandbox preserves state; deleting and recreating a sandbox creates a fresh workspace.

For infrastructure setup, the live database and the OpenShell provider that exposes its `DATABASE_URL` are treated as out-of-band prerequisites. The user-facing launch path above stays a single `openshell` command.

### What persists

When Claude Code is launched with `HOME=/home/agent`, everything it writes under `~/.claude/` persists:

| Path | Content |
|------|---------|
| `~/.claude/memory/` | Remembered context across conversations |
| `~/.claude/plans/` | Implementation plans |
| `~/.claude/tasks/` | Task tracking |
| `~/.claude/sessions/` | Conversation transcripts |
| `~/.claude/settings.json` | User preferences |
| `~/.claude.json` | Authentication state |

All stored as rows in `_openeral.workspace_files` — one row per file, content in BYTEA.

## OpenShell Sandbox

A pre-built container image for running AI agents with database access. See [sandboxes/openeral/README.md](sandboxes/openeral/README.md) for full documentation.

```bash
docker build -f sandboxes/openeral/Dockerfile -t openeral-sandbox:dev .
```

For end users, the supported path is the published sandbox image plus the custom cluster image shown above. The sandbox image is built from the repo root because it copies `Cargo.toml`, `Cargo.lock`, `crates/`, and `.claude/skills/` into the build context.

## Migrations

On first mount, openeral creates an internal `_openeral` schema for audit logging, cache hints, and workspace storage. Migrations are managed by [refinery](https://github.com/rust-db/refinery).

The database user needs either:

- `CREATE` on the database so the first mount can create `_openeral`, or
- a pre-created `_openeral` schema plus write access to it

For an existing `_openeral` schema, grant:

```sql
GRANT ALL ON SCHEMA _openeral TO your_role;
GRANT ALL ON ALL TABLES IN SCHEMA _openeral TO your_role;
```

To skip migrations: `--skip-migrations`.

## Development

All builds and tests run inside Docker containers:

```bash
docker compose up -d                                        # start dev + postgres
docker compose exec dev cargo build                         # build
docker compose exec dev cargo test -p openeral-core          # run tests
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
