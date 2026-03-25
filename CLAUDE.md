# CLAUDE.md

For **using** openeral without developing it, see `sandboxes/openeral/README.md`.

## Build & Test

**ALL builds, tests, and linting MUST run inside the Docker dev container.** Never install Rust or build on the host.

```bash
# Start the environment (if not already running)
docker compose up -d

# Build
docker compose exec dev cargo build

# Run tests
docker compose exec dev cargo test -p openeral-core

# Run FUSE mount integration tests
docker compose exec -e PGPASSWORD=pgmount dev bash tests/test_fuse_mount.sh

# Lint
docker compose exec dev cargo clippy

# Format check
docker compose exec dev cargo fmt -- --check
```

**PostgreSQL is available inside the dev container at:** `host=postgres user=pgmount password=pgmount dbname=testdb`

Do NOT use `cargo build` or `cargo test` directly on the host. The dev container has the correct Rust version, FUSE libraries, and network access to the PostgreSQL container.

## Project Structure

- `crates/openeral/` — binary crate (thin CLI entry point)
- `crates/openeral-core/` — library crate (all logic: FUSE filesystem, DB queries, CLI commands)
- `crates/openeral-core/migrations/` — SQL migrations (V1–V4), managed by refinery
- `sandboxes/openeral/` — current OpenShell sandbox image (upstream base sandbox, supervisor-managed via `/etc/fstab`)
- `vendor/openshell/` — vendored OpenShell fork used to build the custom cluster and gateway images
- `.github/workflows/publish-images.yml` — atomically publishes `openeral/{cluster,gateway,sandbox}`
- `tests/test_fuse_mount.sh` — FUSE mount integration tests (bash)

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
- **Never delete, move, or overwrite user files without explicit permission.** This includes files that appear sensitive, secret-bearing, incorrect, or security-critical.
- **If a file appears risky, stop and ask first.** Report the concern clearly, but do not remove, rewrite, chmod, or hide the file on your own.

## Commit Style

Look at `git log --oneline` for the existing style. Commits are descriptive, imperative mood, with details in the body when needed.
