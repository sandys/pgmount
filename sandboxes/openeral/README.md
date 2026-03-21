# OpenEral Sandbox

A container image for running AI agents with PostgreSQL database access at `/db` (read-only) and an optional persistent workspace at `/home/agent` (read-write, backed by PostgreSQL).

## Quick Start

```bash
# Build the sandbox image
openshell sandbox build openeral

# Database access only
openshell sandbox create --from openeral \
  -e OPENERAL_DATABASE_URL="postgres://readonly:pass@db.example.com/myapp" \
  -- openeral-start.sh openclaw-start

# Database access + persistent workspace
openshell sandbox create --from openeral \
  -e OPENERAL_DATABASE_URL="postgres://user:pass@db.example.com/myapp" \
  -e OPENERAL_WORKSPACE_ID="agent-42" \
  -e OPENERAL_WORKSPACE_CONFIG='{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions"]}' \
  -- openeral-start.sh openclaw-start
```

## Environment Variables

### Database mount (`/db`)

| Variable | Default | Description |
|----------|---------|-------------|
| `OPENERAL_DATABASE_URL` | *(required)* | PostgreSQL connection string |
| `OPENERAL_SCHEMAS` | all | Comma-separated schema filter (e.g. `public,analytics`) |
| `OPENERAL_PAGE_SIZE` | 1000 | Rows per page directory |
| `OPENERAL_CACHE_TTL` | 30 | Metadata cache TTL in seconds |
| `OPENERAL_STATEMENT_TIMEOUT` | 30 | SQL query timeout in seconds |
| `OPENERAL_TIMEOUT` | 15 | Seconds to wait for mount readiness at startup |

### Workspace mount (`/home/agent`)

| Variable | Default | Description |
|----------|---------|-------------|
| `OPENERAL_WORKSPACE_ID` | *(optional)* | Workspace ID — enables workspace mount |
| `OPENERAL_WORKSPACE_MOUNT` | `/home/agent` | Mount point for the workspace |
| `OPENERAL_WORKSPACE_NAME` | *(workspace ID)* | Display name |
| `OPENERAL_WORKSPACE_CONFIG` | `{}` | JSON config for auto_dirs/seed_files |

When `OPENERAL_WORKSPACE_ID` is set, the entrypoint:
1. Creates the workspace if it doesn't exist
2. Mounts it at `OPENERAL_WORKSPACE_MOUNT`
3. Sets `HOME` to the mount point
4. Launches the agent command

## Claude Code Setup

To run Claude Code with persistent state in the sandbox:

```bash
openshell sandbox create --from openeral \
  -e OPENERAL_DATABASE_URL="postgres://user:pass@db/myapp" \
  -e OPENERAL_WORKSPACE_ID="claude-agent-1" \
  -e OPENERAL_WORKSPACE_CONFIG='{"auto_dirs":[".claude",".claude/memory",".claude/plans",".claude/sessions",".claude/tasks",".claude/todos"]}' \
  -e ANTHROPIC_API_KEY="sk-ant-..." \
  -- openeral-start.sh claude
```

Claude Code's `~/.claude/` directory (memory, plans, tasks, sessions, settings) persists across container restarts because `HOME=/home/agent` points to the openeral workspace.

To verify persistence:

```bash
# After the agent runs, check PostgreSQL directly
psql -c "SELECT path, size FROM _openeral.workspace_files WHERE workspace_id='claude-agent-1' ORDER BY path;"
```

## Credential Handling

| Tier | Approach | Security |
|------|----------|----------|
| Simple | `-e OPENERAL_DATABASE_URL=...` on sandbox create | Visible in process list |
| Recommended | Docker secret at `/run/secrets/openeral_database_url` | Not in env or process list |
| Production | Read-only PG role + PgBouncer + secrets injection | Minimal privilege |

The entrypoint checks `/run/secrets/openeral_database_url` first, then falls back to the environment variable.

### Production Database Role

```sql
CREATE ROLE agent_readonly LOGIN PASSWORD 'secure-password';
GRANT CONNECT ON DATABASE myapp TO agent_readonly;
GRANT USAGE ON SCHEMA public TO agent_readonly;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO agent_readonly;

-- openeral needs write access to its internal schema
GRANT ALL ON SCHEMA _openeral TO agent_readonly;
GRANT ALL ON ALL TABLES IN SCHEMA _openeral TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA _openeral GRANT ALL ON TABLES TO agent_readonly;
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

**Connection timeout** — Verify `OPENERAL_DATABASE_URL` and network access. Increase `OPENERAL_TIMEOUT` for slow databases.

**Empty `/db/`** — Check `OPENERAL_SCHEMAS` filter. Verify database user has `USAGE` on target schemas and `SELECT` on tables.

**"Migration failed"** — Database user needs `CREATE` privilege for the `_openeral` schema. See Production Database Role above.
