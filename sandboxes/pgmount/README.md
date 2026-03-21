# pgmount OpenShell Sandbox

An OpenShell sandbox that mounts a PostgreSQL database as a read-only filesystem at `/db`, allowing AI agents to explore relational data using standard file tools.

## Quick Start

```bash
# Build the sandbox image
openshell sandbox build pgmount

# Create and enter with your database
openshell sandbox create --from pgmount \
  --forward 18789 \
  -e PGMOUNT_DATABASE_URL="postgres://readonly:pass@db.example.com/myapp" \
  -- pgmount-start.sh openclaw-start
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PGMOUNT_DATABASE_URL` | *(required)* | PostgreSQL connection string (`postgres://user:pass@host/db`) |
| `PGMOUNT_SCHEMAS` | all | Comma-separated schema filter (e.g. `public,analytics`) |
| `PGMOUNT_PAGE_SIZE` | 1000 | Rows per page directory |
| `PGMOUNT_CACHE_TTL` | 30 | Metadata cache TTL in seconds |
| `PGMOUNT_STATEMENT_TIMEOUT` | 30 | SQL query timeout in seconds |
| `PGMOUNT_TIMEOUT` | 15 | Seconds to wait for mount readiness at startup |

## Credential Handling

| Tier | Approach | Security |
|------|----------|----------|
| Simple | `-e PGMOUNT_DATABASE_URL=...` on sandbox create | Visible in process list |
| Recommended | Docker secret mounted at `/run/secrets/pgmount_database_url` | Not in env or process list |
| Production | Read-only PG role + PgBouncer + secrets injection | Minimal privilege |

The entrypoint checks for a file-based secret at `/run/secrets/pgmount_database_url` first, then falls back to the environment variable.

### Production Database Role

```sql
-- Create a read-only role for the agent
CREATE ROLE agent_readonly LOGIN PASSWORD 'secure-password';
GRANT CONNECT ON DATABASE myapp TO agent_readonly;
GRANT USAGE ON SCHEMA public TO agent_readonly;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO agent_readonly;

-- pgmount needs write access to its internal schema for migrations and audit logging
GRANT ALL ON SCHEMA _pgmount TO agent_readonly;
GRANT ALL ON ALL TABLES IN SCHEMA _pgmount TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA _pgmount GRANT ALL ON TABLES TO agent_readonly;
```

## Security Model

- **Read-only FUSE mount**: The filesystem is mounted with `MountOption::RO`. No data mutation is possible through the mount.
- **Landlock policy**: Filesystem access is restricted via `policy.yaml`. The agent can read `/db` but not write to it.
- **FUSE requirements**: The container needs `SYS_ADMIN` capability and `/dev/fuse` device access. These are declared in `policy.yaml`.
- **Network**: PostgreSQL runs externally. The operator must configure network access (firewall rules, Docker networking) to allow the sandbox to reach the database host.

## Docker Requirements

The container requires FUSE support:

```yaml
devices:
  - /dev/fuse
cap_add:
  - SYS_ADMIN
security_opt:
  - apparmor:unconfined
```

## Troubleshooting

### "FUSE permission denied" or "fusermount: mount failed"
- Ensure the container has `SYS_ADMIN` capability and `/dev/fuse` device access.
- Check that `user_allow_other` is set in `/etc/fuse.conf`.

### "Connection refused" or timeout on startup
- Verify `PGMOUNT_DATABASE_URL` is correct and the database is reachable from the container.
- Check network/firewall rules between the sandbox and PostgreSQL host.
- Increase `PGMOUNT_TIMEOUT` if the database is slow to respond.

### Mount point empty (`ls /db/` shows nothing)
- Check `PGMOUNT_SCHEMAS` — if set, only listed schemas are visible.
- Verify the database user has `USAGE` privilege on the target schemas and `SELECT` on tables.

### "Migration failed" error on startup
- The database user needs `CREATE` privilege to set up the `_pgmount` internal schema.
- If using a read-only role, grant write access to the `_pgmount` schema (see Production Database Role above).
