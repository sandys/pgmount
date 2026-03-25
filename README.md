# OpenEral

OpenEral exists to make Claude Code run inside OpenShell with a persistent home directory.

The product goal is simple:

- launch Claude Code inside an OpenShell sandbox
- mount `/home/agent` as a PostgreSQL-backed writable filesystem
- keep Claude's `~/.claude` state across reconnects and restarts
- optionally expose the same database read-only at `/db`

Everything in this repo supports that flow.

## Supported Outcome

After setup, the command that matters is:

```bash
openshell sandbox create \
  --name "$OPENERAL_SANDBOX_NAME" \
  --from "$OPENERAL_SANDBOX_IMAGE" \
  --provider "$OPENERAL_DB_PROVIDER" \
  --provider claude \
  --auto-providers \
  --no-tty -- env HOME=/home/agent claude
```

When that works correctly:

- Claude Code starts inside the sandbox
- `/home/agent` is mounted by `openeral`
- Claude writes `~/.claude` into `/home/agent`
- those files persist in PostgreSQL
- `/db` is available as a read-only view of the same database

## Prerequisites

This repo assumes these are already true before the final launch command:

1. Upstream `openshell` CLI is installed on the host.
2. An OpenShell gateway is already running with the custom openeral cluster image.
3. A generic OpenShell provider already exists for the live PostgreSQL database.
4. The published openeral sandbox image is available by image reference.
5. The host has `ANTHROPIC_API_KEY` available.

The database and its OpenShell provider are infrastructure prerequisites. They are not created by the final user-facing launch command.

## Launch Claude Code

```bash
export OPENERAL_SANDBOX_IMAGE='<sandbox image ref>'
export OPENERAL_DB_PROVIDER=openeral-db
export OPENERAL_SANDBOX_NAME=openeral-demo

set -a
. ./.env
set +a

openshell sandbox create \
  --name "$OPENERAL_SANDBOX_NAME" \
  --from "$OPENERAL_SANDBOX_IMAGE" \
  --provider "$OPENERAL_DB_PROVIDER" \
  --provider claude \
  --auto-providers \
  --no-tty -- env HOME=/home/agent claude
```

This is the primary supported flow:

- no wrapper scripts
- no `sandbox upload`
- no follow-up `sandbox connect` just to start Claude
- no manual mount steps

OpenShell auto-creates the `claude` provider from host `ANTHROPIC_API_KEY`. The preexisting database provider supplies `DATABASE_URL`, which the sandbox supervisor maps to `OPENERAL_DATABASE_URL` for `openeral`.

## What OpenEral Provides

Inside the sandbox, OpenEral provides two mounts:

- `/home/agent`
  - read-write
  - backed by `_openeral.workspace_files`
  - intended to be Claude Code's durable `HOME`
- `/db`
  - read-only
  - backed by PostgreSQL tables and schemas
  - intended for Claude to inspect database data without separate client tooling

The only `HOME` that should matter for Claude Code is:

```bash
HOME=/home/agent
```

If Claude runs with `HOME=/sandbox`, you are not using the persistent workspace correctly.

## Persistence Model

Persistence is keyed to the OpenShell sandbox object:

- reconnect to the same sandbox: same `/home/agent`
- delete and recreate the sandbox: new `/home/agent`

Files you should expect Claude to persist include:

- `~/.claude.json`
- `~/.claude/settings.json`
- `~/.claude/projects/...`
- Claude transcripts, plans, and local state

Those files are stored as rows in PostgreSQL under `_openeral.workspace_files`.

## Verify Success

If you want to verify the runtime explicitly, connect to the sandbox and check:

```bash
grep -E ' /db | /home/agent ' /proc/mounts
stat -c '%u:%g %a %n' /home/agent
```

Expected properties:

- `/db` and `/home/agent` are both present in `/proc/mounts`
- `/home/agent` is writable by the sandbox user
- Claude runs successfully with `HOME=/home/agent`

If you want to verify persistence in PostgreSQL directly:

```sql
SELECT path, uid, gid, size
FROM _openeral.workspace_files
WHERE workspace_id = '<sandbox-id>'
ORDER BY path;
```

You should see Claude-owned rows such as `/.claude.json` and `/.claude/projects/...`.

## Troubleshooting

- Claude starts but state is not preserved:
  - you are probably not using `HOME=/home/agent`
- `/db` or `/home/agent` is missing:
  - this is an OpenShell or mount bootstrap failure, not a Claude problem
- Claude fails with auth or billing errors:
  - the OpenEral mount path is separate from Anthropic account validity
- The database is mounted but unreadable:
  - the OpenShell database provider or PostgreSQL permissions are wrong

## Repo Scope

This repo produces the pieces that make the above workflow work:

- the `openeral` FUSE binary
- the writable workspace filesystem in PostgreSQL
- the published OpenShell sandbox image
- the custom OpenShell cluster image that enables `/dev/fuse`

If you are looking for the operational sandbox runbook, see [sandboxes/openeral/README.md](/home/sss/Code/pgmount/sandboxes/openeral/README.md).
