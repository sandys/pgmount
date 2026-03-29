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

1. start the gateway with the openeral cluster image and matching `IMAGE_REPO_BASE`
2. create a generic provider from `DATABASE_URL`
3. launch the sandbox with that provider plus `claude --auto-providers`

The runtime dependency is still a 3-image set:

- custom `cluster`
- custom `gateway`
- custom `sandbox`

Only `cluster` and `sandbox` are user-facing. The matching `gateway` image is resolved internally from the cluster image and must not be mixed with upstream OpenShell images.

With upstream `openshell 0.0.12`, the gateway repo base still needs an explicit host-side hint. If the provided cluster image is `ghcr.io/<owner>/openeral/cluster:<tag>`, set `IMAGE_REPO_BASE=ghcr.io/<owner>/openeral` before `openshell gateway start`.

The CLI itself is still the stock upstream `openshell` release. This repo changes the images behind that CLI flow, not the user-facing command surface.

`openeral` also embeds its own database migrations. In the normal sandbox flow, those migrations auto-run before `/db` or `/home/agent` mounts come up. If you are debugging outside the normal mount path and have direct binary access, `openeral migrate` is the manual preflight/admin command.

If the gateway enables package-proxy routing, that still runs through the built-in
OpenShell sandbox proxy. Package-manager traffic may be chained through an
upstream proxy, but normal OpenShell policy still decides whether the binary is
allowed to reach the target at all.

For a real Socket upstream:

- the Socket service account must actually have Firewall Enterprise enabled
- the Socket proxy CA must be mounted into the sandbox trust path

Without those two pieces, Socket-specific package installs will fail even if the
generic OpenShell proxy path is healthy.

The practical validation pattern inside the sandbox is:

```bash
npm view is-number version
curl -I -sS https://registry.npmjs.org/is-number
```

Expected behavior:

- the `npm` command succeeds if policy allows it and the upstream package proxy
  is healthy
- the `curl` command should still be denied if policy only allows npm/node

If you stop the upstream package proxy and rerun `npm view`, it should fail with
an upstream proxy error. That is the expected fail-closed behavior.

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

- mount fails immediately after an image upgrade:
  - check the embedded `openeral` migration path first

Do not try ad hoc mount workarounds from inside the sandbox.
