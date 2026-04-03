# CLAUDE.md

For **using** openeral without developing it, see `sandboxes/openeral/README.md`.

## Build & Test

**Do not use a repo-local docker compose dev stack.** This repo is centered on
the stock upstream `openshell` CLI plus the openeral `cluster`, `gateway`, and
`sandbox` images.

```bash
# Primary end-to-end validation
tests/test_live_secret_injection.sh
```

That harness is the primary verification surface because it exercises the real
OpenShell runtime:

- stock `openshell`
- openeral `cluster` / `gateway` / `sandbox` images
- `/home/agent` persistence through `openeral`
- Anthropic boundary secret injection

If you need lower-level checks in addition to the OpenShell run, prefer direct,
repo-local commands over a compose wrapper:

```bash
cargo test -p openeral-core
bash tests/test_fuse_mount.sh
```

But the OpenShell validation harness is the product-level truth.

## Project Structure

- `crates/openeral/` — binary crate (thin CLI entry point)
- `crates/openeral-core/` — library crate (all logic: FUSE filesystem, DB queries, CLI commands)
- `crates/openeral-core/migrations/` — SQL migrations (V1–V4), managed by refinery
- `sandboxes/openeral/` — current OpenShell sandbox image (upstream base sandbox, supervisor-managed via `/etc/fstab`)
- `vendor/openshell/` — vendored OpenShell fork used to build the custom cluster and gateway images
- `.github/workflows/publish-images.yml` — atomically publishes `openeral/{cluster,gateway,sandbox}`
- `tests/test_fuse_mount.sh` — FUSE mount integration tests (bash)
- `tests/test_live_secret_injection.sh` — OpenShell-first live validation harness for Claude + secret injection

## Two Filesystems

1. **PgmountFilesystem** (`fs/mod.rs`) — read-only mount of database content. Uses `NodeIdentity` enum for inode mapping.
2. **WorkspaceFilesystem** (`fs/workspace.rs`) — read-write mount for agent state. Uses path-based inode table. Files stored in `_openeral.workspace_files`.

## Conventions

- All FUSE callbacks bridge sync→async via `rt.block_on()`
- SQL queries use `quote_ident()` for identifiers and parameterized queries for values
- All column values cast to `::text` in SQL to avoid Rust type-mapping issues
- Errors map to `FsError` which converts to `fuser::Errno`
- New node types: add to `NodeIdentity` enum, create handler in `fs/nodes/`, wire into dispatch functions

## Hard Rules

- **Never fix forward from the middle.** When a mistake is found in a build, setup, or integration flow, stop immediately and restart the entire flow from scratch. Do not patch, work around, or continue from a broken state. This project is being sold — every artifact must be clean and correct from a full rebuild.
- **OpenShell verification must use the supervisor path.** The supported sandbox flow is the custom `openeral/cluster` image plus the published `openeral/sandbox` image. Do not validate OpenShell using `openeral-start.sh` or a container `ENTRYPOINT`; the supervisor overrides the command and mounts FUSE from `/etc/fstab`.
- **Do not reintroduce a repo-local compose-centric workflow.** If a test or instruction can be expressed against the real OpenShell flow, prefer that over maintaining a parallel docker compose path.
- **Never delete, move, or overwrite user files without explicit permission.** This includes files that appear sensitive, secret-bearing, incorrect, or security-critical.
- **If a file appears risky, stop and ask first.** Report the concern clearly, but do not remove, rewrite, chmod, or hide the file on your own.

## Commit Style

Look at `git log --oneline` for the existing style. Commits are descriptive, imperative mood, with details in the body when needed.
