# openeral-shell

An OpenShell sandbox that mounts a PostgreSQL database as a browsable filesystem and gives agents a persistent home directory — both backed by PostgreSQL.

## How It Works

openeral-shell starts two FUSE mounts inside the container. `/db/` exposes your PostgreSQL database as a read-only hierarchy of schemas, tables, rows, and columns — navigable with `ls` and `cat`, no SQL required. `/home/agent/` is a read-write workspace where every file persists in PostgreSQL across container restarts.

openeral is a standard Linux FUSE filesystem (`fuse.openeral`). Mounts are declared in `/etc/fstab` and managed by the OpenShell supervisor via `mount.fuse3` — no elevated privileges leak to user code.

## Quick Start (OpenShell)

openeral-shell ships a custom cluster image with FUSE support. Start the gateway with it, then create sandboxes as usual:

```bash
# 1. Start the gateway with FUSE-enabled cluster image
OPENSHELL_CLUSTER_IMAGE=ghcr.io/pgmount/openeral-cluster:latest \
  openshell gateway start

# 2. Create .env
cat > .env <<'EOF'
DATABASE_URL=postgres://user:pass@host/db
ANTHROPIC_API_KEY=sk-ant-...
EOF

# 3. Create the sandbox
openshell sandbox create --from . --upload .env

# 4. Connect
openshell sandbox connect <sandbox-name> -- claude
```

The patched supervisor reads `/etc/fstab` from the sandbox image, discovers `fuse.openeral` entries, creates `/dev/fuse`, and establishes the FUSE mounts before the child process runs. The child sees `/db/` and `/home/agent/` as regular directories with zero capabilities.

## Quick Start (Standard Linux)

openeral works on any Linux system with FUSE support:

```bash
# Direct mount
openeral mount /db

# Or via standard mount command (fuse.openeral type)
mount -t fuse.openeral "host=pg.example.com dbname=mydb" /db -o ro,allow_other

# Or declare in /etc/fstab
echo '"host=pg.example.com dbname=mydb"  /db  fuse.openeral  ro,noauto,allow_other  0  0' >> /etc/fstab
mount /db
```

Workspace mounts use the `#workspace#<id>` separator in the source:

```bash
mount -t fuse.openeral "host=pg dbname=mydb#workspace#default" /home/agent -o rw,allow_other
```

## What's Inside

- **`/db/`** — your database as files. Schemas are directories, tables are directories, rows are directories containing column files. Includes metadata (`.info/`), filtered views (`.filter/`), sorted views (`.order/`), and bulk exports (`.export/`).
- **`/home/agent/`** — persistent workspace. Everything written here survives container restarts. Agent state (`~/.claude/memory/`, `~/.claude/plans/`, etc.) is automatically provisioned.
- **Pre-installed skill** — teaches agents how to navigate `/db/` and use the workspace.
- **Landlock security policy** — `/db/` is read-only, `/home/agent/` is read-write, system directories are locked.

## Database Filesystem

```
/db/
  <schema>/
    <table>/
      .info/
        columns.json         column names, types, nullability
        schema.sql           CREATE TABLE DDL
        count                exact row count
        primary_key          primary key column(s)
      .export/
        data.json/           paginated JSON  (page_1.json, page_2.json, ...)
        data.csv/            paginated CSV
        data.yaml/           paginated YAML
      .filter/<col>/<val>/   rows where column = value (paginated)
      .order/<col>/asc/      rows sorted ascending (paginated)
      .order/<col>/desc/     rows sorted descending (paginated)
      .indexes/<name>        index definitions
      page_1/
        <pk_value>/          row directory (named by primary key)
          <column>           column value as plain text
          row.json           full row as JSON
          row.csv            full row as CSV
          row.yaml           full row as YAML
      page_2/
      ...
```

### Examples

```bash
ls /db/                                          # list schemas
ls /db/public/                                   # list tables
cat /db/public/users/.info/columns.json          # column definitions
cat /db/public/users/.info/count                 # row count

cat /db/public/users/page_1/42/row.json          # row 42 as JSON
cat /db/public/users/page_1/42/email             # single column value

ls /db/public/users/.filter/active/true/         # filtered rows
cat /db/public/orders/.order/created_at/desc/page_1/1001/row.json

cat /db/public/users/.export/data.csv/page_1.csv # bulk export
```

## Mounting

openeral registers as filesystem type `fuse.openeral`. Three ways to mount:

| Method | Command |
|--------|---------|
| Direct CLI | `openeral mount /db` |
| mount command | `mount -t fuse.openeral "<connstr>" /db -o ro` |
| /etc/fstab | `"<connstr>" /db fuse.openeral ro,noauto 0 0` |

The fstab source field encodes the connection string. For workspace mounts, append `#workspace#<id>`:

```
"host=pg dbname=mydb"                    /db          fuse.openeral  ro,noauto,allow_other  0  0
"host=pg dbname=mydb#workspace#default"  /home/agent  fuse.openeral  rw,noauto,allow_other  0  0
```

In OpenShell sandboxes, the supervisor reads these fstab entries and sets up the mounts during its privileged startup phase. The FUSE daemons run as supervisor-managed services. The child process accesses the mounts with zero capabilities.

## Environment Variables

Set in `.env` and uploaded via `--upload .env`:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | *(required)* | PostgreSQL connection string |
| `ANTHROPIC_API_KEY` | *(required for Claude Code)* | Anthropic API key |
| `WORKSPACE_ID` | `default` | Isolate state per agent |
| `WORKSPACE_CONFIG` | *(broad defaults)* | JSON with `auto_dirs` and `seed_files` |
| `STARTUP_TIMEOUT` | `15` | Seconds to wait for mounts |

## Multiple Agents

Each `WORKSPACE_ID` gets its own isolated `/home/agent/`:

```env
WORKSPACE_ID=agent-alice
```

## Security

- **Landlock policy** — `/db/` read-only, `/home/agent/` read-write, system directories locked
- **FUSE isolation** — database and workspace are independent mounts
- **Non-root execution** — child runs as `sandbox` user (UID 1000) with zero capabilities
- **Supervisor-managed FUSE** — `mount.fuse3` establishes mounts during privileged startup; daemons run as background services alongside proxy and SSH
- **fstab inspection** — FUSE mounts are discovered from the image's `/etc/fstab`, not from executable code

## Building the Cluster Image

The custom cluster image is built from the vendored OpenShell source with the FUSE supervisor patch:

```bash
cd vendor/openshell
docker buildx build --target cluster \
  -t ghcr.io/<owner>/openeral-cluster:latest \
  -f deploy/docker/Dockerfile.images .
```

CI pushes to GHCR automatically on changes to `vendor/openshell/`.
