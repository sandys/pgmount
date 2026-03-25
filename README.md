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

## Fresh Machine Flow

Assume a fresh machine with:

- upstream `openshell` already installed
- a live PostgreSQL database already available
- the openeral cluster image reference
- the openeral sandbox image reference
- host `ANTHROPIC_API_KEY` available

From that starting point, the full flow is:

1. start an OpenShell gateway with the custom openeral cluster image
2. create one generic provider that points at the live PostgreSQL database
3. launch a sandbox from the published openeral sandbox image
4. run Claude with `HOME=/home/agent`

The database itself may exist out of band. The only OpenShell-side setup you need is the gateway and the generic database provider.

### Local Development

If you are developing locally, build and publish both images to a local registry first.

Start a local registry:

```bash
docker run -d --restart=always -p 5000:5000 --name openshell-local-registry registry:2
```

Build and push the cluster image from the vendored OpenShell source:

```bash
docker build \
  -f vendor/openshell/deploy/docker/Dockerfile.images \
  --target cluster \
  -t 127.0.0.1:5000/openshell/openeral-cluster:dev \
  vendor/openshell

docker push 127.0.0.1:5000/openshell/openeral-cluster:dev
```

Build and push the sandbox image from this repo:

```bash
docker build \
  -f sandboxes/openeral/Dockerfile \
  -t 127.0.0.1:5000/openshell/openeral-sandbox:dev \
  .

docker push 127.0.0.1:5000/openshell/openeral-sandbox:dev
```

Then use:

- `OPENSHELL_CLUSTER_IMAGE=127.0.0.1:5000/openshell/openeral-cluster:dev`
- `OPENSHELL_REGISTRY_HOST=172.17.0.1:5000`
- `OPENSHELL_REGISTRY_INSECURE=true`
- `OPENERAL_SANDBOX_IMAGE=172.17.0.1:5000/openshell/openeral-sandbox:dev`

The cluster image is pulled by host Docker, so `127.0.0.1:5000` is correct there. Sandbox images are pulled from inside the cluster container, so use `172.17.0.1:5000` for the sandbox image reference and registry host.

## Start Gateway

```bash
export OPENSHELL_CLUSTER_IMAGE='ghcr.io/sandys/openeral-cluster:latest'
export OPENSHELL_REGISTRY_HOST='ghcr.io'
export OPENSHELL_GATEWAY_NAME=openeral

openshell gateway start --name "$OPENSHELL_GATEWAY_NAME"
```

## Create Database Provider

```bash
export DATABASE_URL='host=<host> port=<port> user=<user> password=<password> dbname=<dbname>'
export OPENERAL_DB_PROVIDER=openeral-db

openshell provider create \
  --gateway "$OPENSHELL_GATEWAY_NAME" \
  --name "$OPENERAL_DB_PROVIDER" \
  --type generic \
  --credential DATABASE_URL
```

## Launch Claude Code

```bash
export OPENERAL_SANDBOX_IMAGE='ghcr.io/sandys/openeral-sandbox:latest'
export OPENERAL_SANDBOX_NAME=openeral-demo

set -a
. ./.env
set +a

openshell sandbox create \
  --gateway "$OPENSHELL_GATEWAY_NAME" \
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
