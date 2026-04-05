# CLAUDE.md

For **using** openeral without developing it, see the README.

## Build & Test

```bash
# openeral-js (primary path — TypeScript + just-bash)
cd openeral-js && pnpm install && pnpm typecheck && pnpm test

# Legacy FUSE path (Rust)
cargo test -p openeral-core
bash tests/test_fuse_mount.sh

# OpenShell end-to-end (requires running cluster)
tests/test_live_secret_injection.sh
```

## Project Structure

- `openeral-js/` — **primary implementation** (TypeScript, just-bash + PostgreSQL)
  - `src/pg-fs/` — PgFs: read-only IFileSystem backed by SQL queries
  - `src/workspace-fs/` — WorkspaceFs: read-write IFileSystem backed by workspace_files
  - `src/db/` — SQL queries (ported from Rust), migrations, pool
  - `src/safety.ts` — command safety analysis via just-bash parse() AST
  - `src/shell.ts` — createOpeneralShell() factory, createToolHandler()
  - `src/index.ts` — public API
- `crates/openeral/` — legacy Rust CLI (FUSE path)
- `crates/openeral-core/` — legacy Rust library (FUSE filesystem, DB queries)
- `crates/openeral-core/migrations/` — SQL migrations (V1-V4), shared by both paths
- `sandboxes/openeral/` — OpenShell sandbox image (uses legacy FUSE path)
- `vendor/openshell/` — vendored OpenShell fork for custom cluster/gateway images
- `tests/` — integration tests

## Two Paths

1. **openeral-js** (preferred) — TypeScript + just-bash. No kernel deps. The
   agent's bash tool routes through just-bash with PostgreSQL-backed virtual
   mounts. Runs in Node.js, serverless, or browser.

2. **crates/ + sandboxes/** (legacy) — Rust + FUSE. Requires `/dev/fuse`, kernel
   modules, privileged containers. Used in the OpenShell sandbox flow where the
   supervisor manages FUSE mounts via `/etc/fstab`.

Both paths share the same SQL queries and `_openeral` database schema.

## openeral-js Conventions

- IFileSystem implementations are path-based (no inodes)
- `parsePath()` returns a `PgNode` discriminated union — the TypeScript equivalent of the Rust `NodeIdentity` enum
- SQL queries use `quoteIdent()` for identifiers and `$N` params for values
- All column values cast to `::text` in SQL
- Read-only filesystem (PgFs) throws EROFS on write methods
- WorkspaceFs receives complete file content per writeFile() call — no write-back buffering
- Command safety uses just-bash's parse() for AST analysis with regex fallback

## Legacy FUSE Conventions

- All FUSE callbacks bridge sync→async via `rt.block_on()`
- Errors map to `FsError` which converts to `fuser::Errno`
- New node types: add to `NodeIdentity` enum, create handler in `fs/nodes/`, wire into dispatch

## Hard Rules

- **Never fix forward from the middle.** When a mistake is found in a build, setup, or integration flow, stop immediately and restart the entire flow from scratch.
- **Do not reintroduce a repo-local compose-centric workflow.**
- **Never delete, move, or overwrite user files without explicit permission.**
- **If a file appears risky, stop and ask first.**

## Commit Style

Look at `git log --oneline` for the existing style. Commits are descriptive, imperative mood, with details in the body when needed.
