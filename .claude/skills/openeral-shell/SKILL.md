---
name: openeral-shell
description: Run Claude Code in the published openeral OpenShell sandbox with a persistent PostgreSQL-backed HOME at /home/agent
---

# OpenEral Shell

This sandbox is for one thing: running Claude Code with persistent state.

Your first assumption should be:

- Claude should run with `HOME=/home/agent`
- `/home/agent` is the durable workspace
- `/db` is available for read-only database access when needed

Do not optimize for anything else first.

## Fresh Machine Setup

If you only have upstream `openshell`, image refs, and a live database:

1. start the gateway with the openeral cluster image
2. create a generic provider from `DATABASE_URL`
3. launch the sandbox with that provider plus `claude --auto-providers`

The runtime dependency is still a 3-image set:

- custom `cluster`
- custom `gateway`
- custom `sandbox`

Only `cluster` and `sandbox` are user-facing. The matching `gateway` image is resolved internally from the cluster image and must not be mixed with upstream OpenShell images.

The CLI itself is still the stock upstream `openshell` release. This repo changes the images behind that CLI flow, not the user-facing command surface.

The supported Claude launch still remains:

```bash
openshell sandbox create \
  --gateway "$OPENSHELL_GATEWAY_NAME" \
  --name "$OPENERAL_SANDBOX_NAME" \
  --from "$OPENERAL_SANDBOX_IMAGE" \
  --provider "$OPENERAL_DB_PROVIDER" \
  --provider claude \
  --auto-providers \
  --no-tty -- env HOME=/home/agent claude
```

## What Must Be True

Inside a healthy sandbox:

- `/home/agent` exists and is writable
- `/db` exists and is mounted read-only
- Claude is launched with `HOME=/home/agent`

Check that first:

```bash
grep -E ' /db | /home/agent ' /proc/mounts
stat -c '%u:%g %a %n' /home/agent
```

## Primary Command

If you need to start Claude manually inside the sandbox, use:

```bash
HOME=/home/agent claude
```

If you need a non-interactive check:

```bash
HOME=/home/agent claude -p 'Reply with READY and nothing else.'
```

## Persistence Rule

Everything Claude needs to keep must live under `/home/agent`.

Expected persistent paths include:

- `/home/agent/.claude.json`
- `/home/agent/.claude/settings.json`
- `/home/agent/.claude/projects/...`

Do not rely on `/sandbox` for durable state.

## `/db` Usage

`/db` is support infrastructure for Claude tasks, not the primary goal.

Use it when Claude needs database context:

```bash
ls /db
ls /db/public
cat /db/public/users/.info/columns.json
cat /db/public/users/.filter/id/42/42/row.json
```

Prefer targeted lookups through `.filter/` instead of browsing large page trees.

## Failure Interpretation

- `/home/agent` missing:
  - infrastructure failure
- `/db` missing:
  - infrastructure failure
- Claude starts but forgets prior state:
  - `HOME` was wrong
- Claude auth or billing failure:
  - credential issue, not a mount issue

Do not try ad hoc mount workarounds from inside the sandbox.
