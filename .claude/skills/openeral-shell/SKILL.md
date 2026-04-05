---
name: openeral-shell
description: Run an AI agent with persistent PostgreSQL-backed /home/agent and read-only /db via just-bash
---

# OpenEral Shell

This skill is for running an AI agent with a persistent home directory and
database access. No FUSE, no kernel modules, no privileged containers.

## How It Works

The agent's bash tool runs through just-bash (TypeScript bash interpreter) with
two PostgreSQL-backed virtual mounts:

- `/home/agent` — read-write, persists to `_openeral.workspace_files`
- `/db` — read-only, generated from live database queries

```typescript
import { createOpeneralShell, createToolHandler } from 'openeral-js'

const shell = await createOpeneralShell({
  connectionString: process.env.DATABASE_URL,
  workspaceId: 'session-123',
  python: true,
})

const handleBash = createToolHandler(shell)
// or with safety enforcement:
const handleBash = createToolHandler(shell, { enforceSafety: true })
```

## What Must Be True

- The agent runs with `HOME=/home/agent`
- `/home/agent` is writable and persists to PostgreSQL
- `/db` is available for read-only database exploration
- All file operations go through the single bash tool

## Primary Commands

```bash
# Database exploration
ls /db/public
cat /db/public/users/.info/columns.json
cat /db/public/users/.info/count
cat /db/public/users/page_1/1/row.json
pg SELECT count(*) FROM public.users

# Workspace
echo "notes" > /home/agent/work/notes.txt
mkdir -p /home/agent/.claude
ls /home/agent
```

## Persistence

Keyed to `workspaceId`:

- Same ID across shell instances = same `/home/agent`
- Different ID = fresh workspace

Expected persistent paths:

- `/home/agent/.claude.json`
- `/home/agent/.claude/settings.json`
- `/home/agent/.claude/projects/...`

## Legacy OpenShell Path

The OpenShell sandbox flow (FUSE-based) is still supported via `sandboxes/openeral/`.
That path requires custom cluster/gateway/sandbox images and `/dev/fuse`.

For new integrations, prefer `openeral-js`.

## Failure Interpretation

- Agent starts but state disappears: wrong `workspaceId` or not using `/home/agent`
- `/db` commands fail: database connection or migration issue
- Write to `/db` fails: expected — `/db` is read-only (EROFS)
- Command blocked: safety layer caught a destructive command
