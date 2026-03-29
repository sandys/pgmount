---
name: openeral-dev
description: Develop and verify the openeral OpenShell flow whose goal is a working Claude Code with persistent /home/agent
disable-model-invocation: false
user-invocable: true
allowed-tools: Read, Grep, Glob, Bash
argument-hint: [task description]
---

# OpenEral Development

The product goal is not “generic database browsing.”

The product goal is:

- OpenShell starts a sandbox
- `openeral` mounts `/home/agent` and `/db`
- Claude Code runs with `HOME=/home/agent`
- `.claude` persists to PostgreSQL

When developing this repo, optimize for that end-to-end flow first.

The supported runtime is a coupled 3-image set:

- `cluster`
  - owns the patched supervisor and bundled manifests
- `gateway`
  - owns sandbox pod spec generation and FUSE device requests
- `sandbox`
  - owns `openeral`, `fuse3`, and `/etc/fstab`

Do not assume any mixed upstream/openeral image combination is valid.

The supported CLI is still the upstream released `openshell` binary. The vendored OpenShell tree is kept to build the custom `cluster` and `gateway` images, not to provide a separate CLI path.

When validating the fresh-machine path with upstream `openshell 0.0.12`, include `IMAGE_REPO_BASE=<openeral repo base>` alongside `OPENSHELL_CLUSTER_IMAGE`. Without that override, the CLI still points the internal gateway pull at upstream OpenShell.

## Files That Matter Most

- `crates/openeral-core/src/fs/workspace.rs`
  - writable workspace persistence
- `crates/openeral-core/src/db/queries/workspace.rs`
  - database storage for workspace files
- `sandboxes/openeral/Dockerfile`
  - published sandbox image
- `vendor/openshell/crates/openshell-sandbox/src/fuse.rs`
  - supervisor-side FUSE startup from `/etc/fstab`
- `vendor/openshell/crates/openshell-server/src/sandbox/mod.rs`
  - sandbox pod generation and resource requests
- `vendor/openshell/crates/openshell-sandbox/src/proxy.rs`
  - package-proxy routing inside the built-in OpenShell sandbox proxy
- `vendor/openshell/crates/openshell-sandbox/src/child_env.rs`
  - child process proxy and CA trust environment
- `vendor/openshell/deploy/helm/openshell/templates/statefulset.yaml`
  - gateway deployment wiring

## Primary Verification Loop

The most important validation is an end-to-end OpenShell run where:

1. a fresh host starts a gateway with the custom cluster image
2. a generic OpenShell provider is created for the live PostgreSQL database
3. the sandbox uses the published openeral image
4. the sandbox starts with `/db` and `/home/agent`
5. `HOME=/home/agent claude -p 'Reply with READY and nothing else.'` succeeds
6. PostgreSQL contains the resulting `/.claude*` rows in `_openeral.workspace_files`

If a change affects the OpenShell path, rerun the full flow from scratch.

In CI and release smoke, use the upstream OpenShell installer/release path for the CLI. Do not add vendored `openshell-cli` builds just to run smoke.

If a change affects package-proxy routing, validate both:

1. positive path:
   - allowed package-manager traffic reaches the upstream proxy
2. negative path:
   - non-allowed binaries are still denied by normal OpenShell policy
   - stopping the upstream proxy makes package-manager traffic fail closed

For real Socket validation, remember the upstream service itself is entitlement
gated. Even with a valid API token, `socketdev/socket-firewall --service` will
exit until the account has Firewall Enterprise enabled. Socket also requires its
CA to be mounted into the sandbox trust path via
`OPENERAL_PACKAGE_PROXY_CA_SECRET_NAME`.

When the Socket account is not entitled yet, use a generic HTTP proxy to
validate the OpenShell side of the feature. That is enough to prove:

- the sandbox proxy routes allowed package-manager traffic upstream
- non-package binaries are still governed by normal OpenShell policy
- the package-manager path fails closed when the upstream proxy is unavailable

The most useful live check pair is:

1. `npm view is-number version`
   - should succeed
   - should appear in the upstream proxy logs
2. `curl -I https://registry.npmjs.org/is-number`
   - should still be denied if the policy only allows npm/node

## Migration Contract

`openeral` owns its PostgreSQL schema migrations.

- binary or image upgrades are expected to auto-run pending migrations on first mount
- mounts must fail closed if migrations cannot be applied
- `openeral migrate` is the explicit admin/preflight command when you have direct binary access

Do not introduce a separate external migration tool for the supported product flow.

## Local Rust Validation

Use the Docker dev environment:

```bash
docker compose up -d
docker compose exec dev cargo build
docker compose exec dev cargo test -p openeral-core
docker compose exec -e PGPASSWORD=pgmount dev bash tests/test_fuse_mount.sh
```

These tests matter mainly because they protect the Claude persistence path.

## Development Heuristics

- prefer fixes that preserve `HOME=/home/agent` semantics
- treat workspace ownership bugs as high severity
- treat `/dev/fuse` or `/etc/fstab` regressions as product blockers
- keep the supported user flow to stock `openshell` commands, not wrapper scripts
- keep CI aligned with that same path: upstream `openshell` CLI driving openeral images
- treat cluster, gateway, and sandbox as one version-locked release set

## Failure Triage

- Claude auth failure:
  - credential or billing problem
- `/home/agent` missing or not writable:
  - workspace mount failure
- `/db` missing:
  - database mount failure
- Claude runs but state disappears:
  - wrong `HOME` or workspace persistence bug

Always debug from the end-user product flow backward.
