# OpenEral

OpenEral exists to make Claude Code run inside OpenShell with a persistent home directory.

The product goal is simple:

- launch Claude Code inside an OpenShell sandbox
- mount `/home/agent` as a PostgreSQL-backed writable filesystem
- keep Claude's `~/.claude` state across reconnects and restarts
- optionally expose the same database read-only at `/db`

Everything in this repo supports that flow.

OpenEral also extends the OpenShell outbound proxy path so allowed package-manager
traffic can be chained through an upstream package proxy. FUSE remains optional at
the product level, but the current published sandbox still includes the FUSE path.
It also supports boundary-level secret injection on inspected REST endpoints, so
workloads can carry placeholder tokens instead of real credentials.

One important constraint:

- users run the stock upstream `openshell` CLI
- this repo ships custom `cluster`, `gateway`, and `sandbox` images
- CI smoke validation also uses the upstream released `openshell` CLI, not a vendored locally built CLI

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

### Image Contract

OpenEral FUSE support depends on three custom runtime images:

- `cluster`
  - boots k3s
  - bundles the patched `openshell-sandbox` supervisor
  - installs the FUSE device-plugin manifests
- `gateway`
  - creates sandbox pod specs
  - requests the FUSE device resource
- `sandbox`
  - contains `openeral`, `fuse3`, and the `/etc/fstab` mount declarations
  - ships the repo-owned `/etc/openshell/policy.yaml` used by the supervisor
    inside the sandbox

In normal use you only set two refs:

- `OPENSHELL_CLUSTER_IMAGE`
- `OPENERAL_SANDBOX_IMAGE`

The matching `gateway` image is internal and is resolved from the cluster image. It is version-locked to that cluster image and should not be mixed with upstream OpenShell images.

With stock upstream `openshell 0.0.12`, also set `IMAGE_REPO_BASE` to the openeral repo base when using canonical openeral image refs. Otherwise the CLI still defaults the internal gateway pull to upstream OpenShell.

Unsupported combinations:

- openeral `cluster` + upstream `gateway`
- upstream `cluster` + openeral `gateway`
- upstream `cluster` + openeral `sandbox`

The repo still vendors OpenShell source because the custom `cluster` and `gateway` images are built from it. That vendored source is for image builds, not for the user-facing CLI.

### Sandbox Policy Contract

The openeral sandbox image does not rely on whatever policy file happens to ship
in the upstream base image.

It explicitly copies this repo's `sandboxes/openeral/policy.yaml` into:

```text
/etc/openshell/policy.yaml
```

That file is the authoritative runtime policy for the published openeral
sandbox image.

Why this matters:

- the upstream base image already allows the normal Claude egress path
- openeral now also relies on supervisor-managed placeholder provider env
- the sandbox policy therefore has to authorize boundary secret injection for
  the Anthropic path used by Claude Code

Today the important rules are:

- `claude_code`
  - allows Claude traffic to `api.anthropic.com`
  - uses `protocol: rest`
  - uses `tls: terminate`
  - injects `ANTHROPIC_API_KEY` on the `x-api-key` header
- `anthropic_secret_test`
  - allows `curl` to call `GET /v1/models`
  - uses the same placeholder-to-secret rewrite path

If you change secret-injection behavior for the sandbox, update
`sandboxes/openeral/policy.yaml` and rebuild the sandbox image.

### Optional Package Proxy

Package installs can be routed through OpenShell's existing sandbox proxy and
policy engine instead of going direct to the public registries.

- policy allow/deny still comes from normal OpenShell `network_policies`
- if an allowed request matches the package-proxy route, the sandbox proxy chains
  it through the configured upstream package proxy
- if the upstream package proxy is unavailable, package-manager traffic fails
  closed for that request

Current implementation details:

- supported package-manager families: npm/pnpm/yarn, pip/uv, cargo
- package-proxy routing is enforced inside the built-in OpenShell sandbox proxy,
  not by a separate sidecar
- non-package traffic still follows the normal OpenShell allow/deny path

### Boundary Secret Injection

OpenEral also extends the same built-in OpenShell proxy/policy path with
endpoint-scoped secret injection.

- workloads see placeholder values like `openshell:resolve:env:OPENAI_API_KEY`
- real secrets stay in provider env and are only redeemed inside the sandbox proxy
- secret injection is controlled per endpoint through normal OpenShell
  `network_policies`
- v1 supports header and query-string rewriting on inspected REST traffic

Required policy shape:

- `protocol: rest`
- `tls: terminate`
- `secret_injection:` rules on the endpoint

Routing behavior for an endpoint that has both `secret_injection` and
`egress_via: package_proxy`:

- requests without placeholders use the endpoint's normal route
- requests with authorized placeholders are rewritten and sent direct to origin
- requests with unauthorized or leaked placeholders are denied

Important migration note:

- plain `HTTP_PROXY` forward-proxy requests no longer get automatic placeholder
  rewriting
- placeholder-based auth must use the CONNECT + `protocol: rest` +
  `tls: terminate` path

Current openeral sandbox detail:

- the published sandbox policy authorizes Claude's Anthropic path by rewriting
  the placeholder `ANTHROPIC_API_KEY` into the `x-api-key` header on the
  `claude_code` policy entry
- this is what makes the stock command
  `HOME=/home/agent claude -p 'Reply with READY and nothing else.'`
  work inside the sandbox while the child process still only sees the
  placeholder env value

Example policy shape:

```yaml
network_policies:
  openai_api:
    name: openai_api
    endpoints:
      - host: api.openai.com
        port: 443
        protocol: rest
        tls: terminate
        egress_via: package_proxy
        egress_profile: socket
        rules:
          - allow:
              method: POST
              path: /v1/**
        secret_injection:
          - env_var: OPENAI_API_KEY
            match_headers: [Authorization]
            match_query: true
    binaries:
      - path: /usr/bin/curl
```

Cluster-scoped control knobs:

- `OPENERAL_PACKAGE_PROXY_ENABLED`
- `OPENERAL_PACKAGE_PROXY_PROFILE`
- `OPENERAL_PACKAGE_PROXY_UPSTREAM_URL`
- optional `OPENERAL_PACKAGE_PROXY_CA_SECRET_NAME`
- optional `OPENERAL_PACKAGE_PROXY_AUTH_SECRET_NAME`

For a real Socket Firewall Enterprise upstream, the sandbox also needs the
package-proxy CA mounted into the pod so child processes trust Socket's MITM
certificate. The gateway already supports that via
`OPENERAL_PACKAGE_PROXY_CA_SECRET_NAME`, with a Kubernetes secret containing
`ca.crt`.

What was validated:

- `npm view is-number version` succeeds from inside the sandbox and is observed
  on the upstream proxy
- `curl https://registry.npmjs.org/...` from the same sandbox is still denied by
  normal OpenShell policy when the policy only allows npm/node
- stopping the upstream proxy makes `npm view ...` fail with a proxy error
  instead of silently falling back to direct egress

Operational note:

- on this machine, OpenShell-side package-proxy chaining was validated end to end
  with a generic upstream proxy
- the actual `socketdev/socket-firewall --service` container still exits with
  `Socket Firewall is not enabled for your account; please contact Socket sales`
  unless the Socket account has the Enterprise Firewall entitlement

### Local Development

If you are developing locally, build and publish all three images to a local registry first.

Start a local registry:

```bash
docker run -d --restart=always -p 5000:5000 --name openshell-local-registry registry:2
```

Build and push the cluster image from the vendored OpenShell source:

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

Build and push the sandbox image from this repo:

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

If you change only sandbox policy or sandbox packaging:

- rebuild and push the sandbox image
- keep the cluster/gateway image pair version-locked
- if your local cluster image was baked with a specific gateway tag, make sure
  that gateway tag exists in the registry even when the gateway code itself did
  not change

## CI Contract

The publish workflow builds the openeral `cluster`, `gateway`, and `sandbox` images from this repo, then validates them with the upstream released `openshell` CLI.

That is intentional:

- image builds come from the vendored OpenShell fork plus this repo
- runtime control is exercised through the stock upstream `openshell` CLI
- CI should not compile a vendored `openshell` CLI binary just to run smoke tests

## Database Migrations

`openeral` embeds its database migrations with `refinery`.

That matters during upgrades:

- the first upgraded `openeral` mount against a database auto-applies any pending `_openeral` schema migrations
- if migrations fail, the mount fails instead of serving `/db` or `/home/agent` against a half-prepared schema

In the normal OpenShell flow, auto-run is the expected path. If you have direct access to the `openeral` binary and want to prepare a database ahead of time, the explicit admin command is:

```bash
openeral migrate --connection "$DATABASE_URL"
```

If `--connection` is omitted, `openeral migrate` uses `OPENERAL_DATABASE_URL`.

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

## Launch Claude Code

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

This is the primary supported flow:

- no wrapper scripts
- no `sandbox upload`
- no follow-up `sandbox connect` just to start Claude
- no manual mount steps

OpenShell auto-creates the `claude` provider from host `ANTHROPIC_API_KEY`. The preexisting database provider supplies `DATABASE_URL`, which the sandbox supervisor maps to `OPENERAL_DATABASE_URL` for `openeral`.

The cluster image resolves the matching gateway image automatically. In normal use you only set:

- `OPENSHELL_CLUSTER_IMAGE`
- `OPENERAL_SANDBOX_IMAGE`

Behind the scenes, the gateway image is still required and must come from the same openeral image set as the cluster image. With upstream `openshell 0.0.12`, `IMAGE_REPO_BASE` is the required hint that points that internal gateway pull at the openeral repo base instead of upstream.

For repeatable deployments, prefer the same immutable tag for both refs, for example a release tag or `sha-<commit>`. `latest` is intended for atomic quickstarts, not for long-lived pinned environments.
For repeatable deployments, prefer the provided immutable refs, for example a release tag or `sha-<commit>`. Do not assume the canonical `latest` tags exist or are the intended deployment channel.

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

## Live Validation Status

The current tree was live-validated on April 3, 2026 with the stock upstream
`openshell` CLI and the openeral images.

What was proven in the live run:

1. inside the sandbox, `ANTHROPIC_API_KEY` was visible only as:
   - `openshell:resolve:env:ANTHROPIC_API_KEY`
2. `HOME=/home/agent claude -p 'Reply with READY and nothing else.'` succeeded
3. `/home/agent` was mounted by `openeral` and `.claude*` files were persisted
   into `_openeral.workspace_files`
4. a separate sandbox run using:

```bash
curl -fsS https://api.anthropic.com/v1/models \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H 'anthropic-version: 2023-06-01'
```

   also succeeded while `$ANTHROPIC_API_KEY` was still the placeholder value in
   the child environment

This is the current reference proof that:

- the positive Claude path works
- boundary secret injection works for the Anthropic path
- Claude state persists into PostgreSQL-backed `/home/agent`

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
