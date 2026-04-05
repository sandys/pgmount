---
name: openeral-shell
description: Run Claude Code with persistent /home/agent and /db in a stock OpenShell sandbox via just-bash + PostgreSQL
---

# OpenEral Shell

Run Claude Code in an OpenShell sandbox with persistent `/home/agent` and read-only `/db`. Uses stock OpenShell — no custom cluster or gateway images.

## Launch

```bash
openshell gateway start

openshell provider create \
  --name db --type generic --credential DATABASE_URL

openshell sandbox create \
  --from ghcr.io/<owner>/openeral/sandbox:latest \
  --provider db --provider claude --auto-providers \
  -- /opt/openeral/setup.sh
```

`setup.sh` runs migrations, seeds the workspace, starts the openeral-bash daemon, then launches Claude Code with `HOME=/home/agent`.

## What Happens Inside

1. OpenShell provider injects `DATABASE_URL` and `ANTHROPIC_API_KEY` (as placeholder)
2. `setup.sh` runs `_openeral` schema migrations
3. openeral-bash daemon starts on Unix socket
4. Claude Code launches — every `bash -c` routes through the daemon
5. Daemon dispatches to just-bash with PgFs (`/db`) and WorkspaceFs (`/home/agent`)
6. Writes to `/home/agent` persist to PostgreSQL immediately

## Using /db

```bash
ls /db/public
cat /db/public/users/.info/columns.json
cat /db/public/users/.info/count
cat /db/public/users/page_1/1/row.json
ls /db/public/users/.filter/status/active/
pg "SELECT count(*) FROM public.users"
```

## Persistence

Keyed to `OPENSHELL_SANDBOX_ID`:
- Reconnect to same sandbox = same `/home/agent`
- Delete and recreate = fresh workspace

Expected persistent paths: `.claude.json`, `.claude/settings.json`, `.claude/projects/...`

## Build

```bash
docker build -f sandboxes/openeral/Dockerfile -t openeral/sandbox:latest .
```

## Troubleshooting

- Claude auth failure: check `ANTHROPIC_API_KEY` provider and policy.yaml secret_injection
- `/db` empty or errors: check `DATABASE_URL` provider and run `pg "SELECT 1"`
- State lost between sessions: verify same sandbox ID / workspaceId
- Write to `/db` fails: expected — read-only (EROFS)
