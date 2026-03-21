# pgmount OpenShell Sandbox

A container image for running AI agents with PostgreSQL database access at `/db` (read-only) and an optional persistent workspace at `/home/agent` (read-write, backed by PostgreSQL).

## Quick Start

```bash
# Build the sandbox image
openshell sandbox build pgmount

# Database access only
openshell sandbox create --from pgmount \
  -e PGMOUNT_DATABASE_URL="postgres://readonly:pass@db.example.com/myapp" \
  -- pgmount-start.sh openclaw-start

# Database access + persistent workspace
openshell sandbox create --from pgmount \
  -e PGMOUNT_DATABASE_URL="postgres://user:pass@db.example.com/myapp" \
  -e PGMOUNT_WORKSPACE_ID="agent-42" \
  -e PGMOUNT_WORKSPACE_CONFIG='{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions"]}' \
  -- pgmount-start.sh openclaw-start
```

## Environment Variables

### Database mount (`/db`)

| Variable | Default | Description |
|----------|---------|-------------|
| `PGMOUNT_DATABASE_URL` | *(required)* | PostgreSQL connection string |
| `PGMOUNT_SCHEMAS` | all | Comma-separated schema filter (e.g. `public,analytics`) |
| `PGMOUNT_PAGE_SIZE` | 1000 | Rows per page directory |
| `PGMOUNT_CACHE_TTL` | 30 | Metadata cache TTL in seconds |
| `PGMOUNT_STATEMENT_TIMEOUT` | 30 | SQL query timeout in seconds |
| `PGMOUNT_TIMEOUT` | 15 | Seconds to wait for mount readiness at startup |

### Workspace mount (`/home/agent`)

| Variable | Default | Description |
|----------|---------|-------------|
| `PGMOUNT_WORKSPACE_ID` | *(optional)* | Workspace ID — enables workspace mount |
| `PGMOUNT_WORKSPACE_MOUNT` | `/home/agent` | Mount point for the workspace |
| `PGMOUNT_WORKSPACE_NAME` | *(workspace ID)* | Display name |
| `PGMOUNT_WORKSPACE_CONFIG` | `{}` | JSON config for auto_dirs/seed_files |

When `PGMOUNT_WORKSPACE_ID` is set, the entrypoint:
1. Creates the workspace if it doesn't exist
2. Mounts it at `PGMOUNT_WORKSPACE_MOUNT`
3. Sets `HOME` to the mount point
4. Launches the agent command

## Claude Code Setup

To run Claude Code with persistent state in the sandbox:

```bash
openshell sandbox create --from pgmount \
  -e PGMOUNT_DATABASE_URL="postgres://user:pass@db/myapp" \
  -e PGMOUNT_WORKSPACE_ID="claude-agent-1" \
  -e PGMOUNT_WORKSPACE_CONFIG='{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions",".claude/tasks",".claude/todos"]}' \
  -e ANTHROPIC_API_KEY="sk-ant-..." \
  -- pgmount-start.sh claude
```

Claude Code's `~/.claude/` directory (memory, plans, tasks, sessions, settings) persists across container restarts because `HOME=/home/agent` points to the pgmount workspace.

To verify persistence:

```bash
# After the agent runs, check PostgreSQL directly
psql -c "SELECT path, size FROM _pgmount.workspace_files WHERE workspace_id='claude-agent-1' ORDER BY path;"
```

## Credential Handling

| Tier | Approach | Security |
|------|----------|----------|
| Simple | `-e PGMOUNT_DATABASE_URL=...` on sandbox create | Visible in process list |
| Recommended | Docker secret at `/run/secrets/pgmount_database_url` | Not in env or process list |
| Production | Read-only PG role + PgBouncer + secrets injection | Minimal privilege |

The entrypoint checks `/run/secrets/pgmount_database_url` first, then falls back to the environment variable.

### Production Database Role

```sql
CREATE ROLE agent_readonly LOGIN PASSWORD 'secure-password';
GRANT CONNECT ON DATABASE myapp TO agent_readonly;
GRANT USAGE ON SCHEMA public TO agent_readonly;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO agent_readonly;

-- pgmount needs write access to its internal schema
GRANT ALL ON SCHEMA _pgmount TO agent_readonly;
GRANT ALL ON ALL TABLES IN SCHEMA _pgmount TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA _pgmount GRANT ALL ON TABLES TO agent_readonly;
```

## Security Model

- **`/db`** — read-only FUSE mount (`MountOption::RO`). No data mutation possible.
- **`/home/agent`** — read-write workspace. Stores only agent state, cannot access database tables.
- **Landlock policy** — filesystem access restricted via `policy.yaml`.
- **FUSE** — requires `SYS_ADMIN` capability and `/dev/fuse` device access.

## Docker Requirements

```yaml
devices:
  - /dev/fuse
cap_add:
  - SYS_ADMIN
security_opt:
  - apparmor:unconfined
```

## Troubleshooting

**"fusermount: mount failed"** — Ensure `SYS_ADMIN` capability and `/dev/fuse` device. Check `user_allow_other` in `/etc/fuse.conf`.

**Connection timeout** — Verify `PGMOUNT_DATABASE_URL` and network access. Increase `PGMOUNT_TIMEOUT` for slow databases.

**Empty `/db/`** — Check `PGMOUNT_SCHEMAS` filter. Verify database user has `USAGE` on target schemas and `SELECT` on tables.

**"Migration failed"** — Database user needs `CREATE` privilege for the `_pgmount` schema. See Production Database Role above.
