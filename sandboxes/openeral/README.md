# OpenEral Sandbox

This sandbox exists for one purpose: run Claude Code in OpenShell with a persistent PostgreSQL-backed home directory at `/home/agent`.

`/db` is mounted too, but it is secondary. The primary success criterion is that Claude writes `~/.claude` into `/home/agent` and that those files survive reconnects.

The user-facing CLI remains the stock upstream `openshell` release. This repo customizes the runtime images, not the CLI command users invoke.

## Fresh Machine Flow

Assume a fresh machine with:

- upstream `openshell`
- a live PostgreSQL database
- the openeral cluster image reference
- the openeral sandbox image reference
- host `ANTHROPIC_API_KEY`

From there, the supported flow is:

1. start the gateway with the openeral cluster image
2. create one generic provider for the live database
3. launch Claude from the sandbox image

## Image Contract

OpenEral FUSE support uses three custom runtime images:

- `cluster`
  - boots k3s
  - bundles the patched `openshell-sandbox` supervisor
  - installs the FUSE device-plugin manifests
- `gateway`
  - creates sandbox pod specs
  - requests the FUSE device resource
- `sandbox`
  - contains `openeral`, `fuse3`, and `/etc/fstab`

Only two refs are user-facing:

- `OPENSHELL_CLUSTER_IMAGE`
- `OPENERAL_SANDBOX_IMAGE`

The `gateway` image is internal. It is resolved from the cluster image and must come from the same openeral image set.

With stock upstream `openshell 0.0.12`, also set `IMAGE_REPO_BASE` to the openeral repo base when using canonical openeral image refs. Otherwise the CLI still defaults the internal gateway pull to upstream OpenShell.

Unsupported combinations:

- openeral `cluster` + upstream `gateway`
- upstream `cluster` + openeral `gateway`
- upstream `cluster` + openeral `sandbox`

The vendored OpenShell source in this repo exists to build the custom `cluster` and `gateway` images. It is not the supported source of the user-facing CLI.

## Optional Package Proxy

OpenEral can also route package-manager traffic through the built-in OpenShell
sandbox proxy to an upstream package proxy.

- OpenShell policy still decides whether a given binary may reach a given host
- once allowed, package-manager traffic can be chained through an upstream proxy
- if that upstream proxy is down, package-manager requests fail closed

For a real Socket Firewall Enterprise deployment, the sandbox must trust the
Socket proxy CA. The supported control-plane knob is
`OPENERAL_PACKAGE_PROXY_CA_SECRET_NAME`, pointing at a Kubernetes secret with a
`ca.crt` entry mounted into the sandbox pod.

Cluster-scoped control knobs:

- `OPENERAL_PACKAGE_PROXY_ENABLED`
- `OPENERAL_PACKAGE_PROXY_PROFILE`
- `OPENERAL_PACKAGE_PROXY_UPSTREAM_URL`
- optional `OPENERAL_PACKAGE_PROXY_CA_SECRET_NAME`
- optional `OPENERAL_PACKAGE_PROXY_AUTH_SECRET_NAME`

The validated behavior is:

- allowed `npm` traffic is chained through the upstream proxy
- non-allowed binaries are still denied by normal OpenShell policy
- if the upstream package proxy is down, package-manager requests fail closed

Observed runtime caveat:

- generic upstream proxy routing is validated end to end
- actual Socket service mode still requires the account entitlement behind
  `socketdev/socket-firewall --service`; without it, the service exits before the
  OpenShell sandbox can use it

## Quick Start (One Command)

Set three environment variables, then run a single command:

```bash
export DATABASE_URL='host=<host> user=<user> password=<password> dbname=<dbname>'
export ANTHROPIC_API_KEY='<your-anthropic-api-key>'
export STRINGCOST_API_KEY='<your-stringcost-api-key>'   # optional, enables cost tracking

openeral launch --image <sandbox-image-ref>
```

That's it. `openeral launch` handles gateway startup, provider creation,
StringCost presigning, and sandbox launch automatically.

If `STRINGCOST_API_KEY` is set, all Anthropic traffic is routed through
StringCost for cost tracking. If it's not set, Claude talks directly to
Anthropic. Claude picks its own model — no override needed.

### Options

| Flag | Default | Description |
|---|---|---|
| `--image` | `$OPENERAL_SANDBOX_IMAGE` | Sandbox image reference (required) |
| `--gateway` | `openeral` | Gateway name |
| `--name` | `openeral-sandbox` | Sandbox name |
| `--no-stringcost` | off | Skip StringCost even if key is set |
| `--dry-run` | off | Print commands without executing |

### Preview Mode

See exactly what will run before committing:

```bash
openeral launch --image <sandbox-image-ref> --dry-run
```

## Optional StringCost Integration

OpenEral can route all Anthropic API traffic through
[StringCost](https://github.com/arakoodev/stringcost) for automatic cost
tracking, billing, and usage metering.

No wrapper scripts, no profile hooks, no inference routing config. `openeral
launch` handles everything automatically when `STRINGCOST_API_KEY` is set.
Claude picks its own model — no override.

### How It Works

1. `openeral launch` presigns the Anthropic key via StringCost
2. Passes the presigned URL as `ANTHROPIC_BASE_URL` to the sandbox
3. Claude Code sends API requests to StringCost with a placeholder `x-api-key`
4. The OpenShell proxy rewrites the placeholder header to the real key
5. StringCost receives the real key, proxies to Anthropic with cost tracking

The sandbox image includes a network policy allowing `proxy.stringcost.com:443`.

### Local Development

If you are developing locally, build and publish all three images to a local registry first.

Start a local registry:

```bash
docker run -d --restart=always -p 5000:5000 --name openshell-local-registry registry:2
```

Build and push the cluster image:

```bash
docker build \
  -f vendor/openshell/deploy/docker/Dockerfile.images \
  --target cluster \
  --build-arg OPENERAL_DEFAULT_IMAGE_REPO_BASE=172.17.0.1:5000/openshell/openeral \
  --build-arg OPENERAL_DEFAULT_IMAGE_TAG=dev \
  -t 127.0.0.1:5000/openshell/openeral/cluster:dev \
  vendor/openshell

docker push 127.0.0.1:5000/openshell/openeral/cluster:dev
```

Build and push the matching gateway image:

```bash
docker build \
  -f vendor/openshell/deploy/docker/Dockerfile.images \
  --target gateway \
  -t 127.0.0.1:5000/openshell/openeral/gateway:dev \
  vendor/openshell

docker push 127.0.0.1:5000/openshell/openeral/gateway:dev
```

Build and push the sandbox image:

```bash
docker build \
  -f sandboxes/openeral/Dockerfile \
  -t 127.0.0.1:5000/openshell/openeral/sandbox:dev \
  .

docker push 127.0.0.1:5000/openshell/openeral/sandbox:dev
```

Then use:

- `OPENSHELL_CLUSTER_IMAGE=127.0.0.1:5000/openshell/openeral/cluster:dev`
- `OPENSHELL_REGISTRY_HOST=172.17.0.1:5000`
- `OPENSHELL_REGISTRY_INSECURE=true`
- `OPENERAL_SANDBOX_IMAGE=172.17.0.1:5000/openshell/openeral/sandbox:dev`

The cluster image is pulled by host Docker, so `127.0.0.1:5000` is correct there. The cluster image itself is baked to resolve its sibling gateway image via `172.17.0.1:5000`, and the sandbox image is also pulled from inside the cluster, so use `172.17.0.1:5000` for the sandbox image reference and the registry host.

## Start Gateway

```bash
export OPENSHELL_CLUSTER_IMAGE='<provided-cluster-image-ref>'
export IMAGE_REPO_BASE='<provided-gateway-repo-base>'
export OPENSHELL_REGISTRY_HOST='ghcr.io'
export OPENSHELL_GATEWAY_NAME=openeral

openshell gateway start --name "$OPENSHELL_GATEWAY_NAME"
```

If your cluster image ref is `ghcr.io/<owner>/openeral/cluster:<tag>`, then `IMAGE_REPO_BASE` should be `ghcr.io/<owner>/openeral`.

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

## One-Command Launch

```bash
export OPENERAL_SANDBOX_IMAGE='<provided-sandbox-image-ref>'
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

The cluster image resolves the matching gateway image automatically. The gateway image is still required, but it is not a user-facing input. With upstream `openshell 0.0.12`, `IMAGE_REPO_BASE` is the required hint that points that internal gateway pull at the openeral repo base instead of upstream. For repeatable deployments, prefer the provided immutable refs, for example a release tag or `sha-<commit>`. Do not assume the canonical `latest` tags exist or are the intended deployment channel.

This is the preferred and supported user flow:

- single `openshell` command
- no wrapper scripts
- no `sandbox upload`
- no manual `sandbox connect` just to start Claude

The same rule applies in CI: release smoke installs the upstream released `openshell` CLI and drives the published openeral images through that CLI path.

## Database Migrations

`openeral` carries its own embedded PostgreSQL migrations with `refinery`.

That means image upgrades are self-contained:

- the first upgraded mount auto-applies pending `_openeral` schema changes
- if migrations fail, the sandbox does not get a working `/db` or `/home/agent` mount

That auto-run path is the normal OpenShell behavior. If you are debugging or preparing a database manually and have direct access to the binary, you can run:

```bash
openeral migrate --connection "$DATABASE_URL"
```

Without `--connection`, the command falls back to `OPENERAL_DATABASE_URL`.

## Runtime Contract

When the sandbox is healthy:

- `/home/agent` is mounted read-write by `openeral`
- `/db` is mounted read-only by `openeral`
- Claude runs with `HOME=/home/agent`
- `.claude` files are written into PostgreSQL-backed storage

The sandbox image declares these mounts in `/etc/fstab`:

- `env /db fuse.openeral ro,allow_other,noauto 0 0`
- `env#workspace#${OPENSHELL_SANDBOX_ID} /home/agent fuse.openeral rw,allow_other,noauto 0 0`

OpenShell side-loads `openshell-sandbox`, which reads `/etc/fstab` and launches `mount.fuse3` before the child process starts. The database provider's `DATABASE_URL` is mapped to `OPENERAL_DATABASE_URL` for `openeral`.

## Persistence Rules

- `/home/agent` is the durable Claude home
- reconnect to the same sandbox: same workspace
- delete and recreate the sandbox: new workspace

If Claude is not running with `HOME=/home/agent`, persistence is not configured correctly.

## Quick Checks

Inside the sandbox, these are the checks that matter:

```bash
grep -E ' /db | /home/agent ' /proc/mounts
stat -c '%u:%g %a %n' /home/agent
HOME=/home/agent claude -p 'Reply with READY and nothing else.'
```

If you need to confirm persistence in PostgreSQL:

```sql
SELECT path, uid, gid, size
FROM _openeral.workspace_files
WHERE workspace_id = '<sandbox-id>'
ORDER BY path;
```

You should see Claude state files such as:

- `/.claude.json`
- `/.claude/settings.json`
- `/.claude/projects/...`

## Failure Meaning

- missing `/home/agent`:
  - OpenShell or FUSE bootstrap failed
- missing `/db`:
  - database provider or FUSE bootstrap failed
- Claude auth failure:
  - Anthropic credential issue, not an openeral mount issue
- state not preserved:
  - Claude was not run with `HOME=/home/agent`

## Image Notes

- `openshell sandbox create --from sandboxes/openeral` is not the supported user flow
- this image is meant to be published and referenced by image tag
- the image `ENTRYPOINT` is not the OpenShell control path; the supervisor mounts FUSE from `/etc/fstab`
