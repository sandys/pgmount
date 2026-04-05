---
name: openeral-dev
description: Develop openeral-js — the just-bash + PostgreSQL virtual filesystem for AI agents
disable-model-invocation: false
user-invocable: true
allowed-tools: Read, Grep, Glob, Bash
argument-hint: [task description]
---

# OpenEral Development

The product goal is:

- an AI agent has a single `bash` tool
- that tool runs through just-bash with PostgreSQL-backed virtual mounts
- `/home/agent` persists agent state across sessions
- `/db` gives read-only database access
- no kernel modules, no FUSE, no privileged containers

## Two Paths

1. **openeral-js** (primary) — TypeScript + just-bash. All new work goes here.
2. **crates/ + sandboxes/** (legacy) — Rust + FUSE. Retained for OpenShell sandbox flow.

Both share the same `_openeral` database schema and SQL migrations.

## Files That Matter

### openeral-js (primary)

- `openeral-js/src/pg-fs/pg-fs.ts` — PgFs IFileSystem (read-only database mount)
- `openeral-js/src/pg-fs/path-parser.ts` — path → PgNode discriminated union
- `openeral-js/src/workspace-fs/workspace-fs.ts` — WorkspaceFs IFileSystem (read-write)
- `openeral-js/src/db/queries.ts` — all SQL queries (ported from Rust)
- `openeral-js/src/db/workspace-queries.ts` — workspace CRUD queries
- `openeral-js/src/db/migrations.ts` — V1-V4 migrations
- `openeral-js/src/safety.ts` — command safety via just-bash parse() AST
- `openeral-js/src/shell.ts` — createOpeneralShell() factory

### Legacy FUSE (reference)

- `crates/openeral-core/src/fs/inode.rs` — NodeIdentity enum (blueprint for PgNode)
- `crates/openeral-core/src/fs/nodes/` — content generation per node type
- `crates/openeral-core/src/db/queries/` — original SQL queries (Rust)

## Verification

```bash
# TypeScript path
cd openeral-js && pnpm typecheck && pnpm test

# Path parser tests (no DB needed)
pnpm test -- src/pg-fs/path-parser.test.ts

# Safety analysis tests (no DB needed)
pnpm test -- src/safety.test.ts

# Integration test (requires PostgreSQL)
# const shell = await createOpeneralShell({ connectionString, workspaceId })
# shell.exec('ls /db') → lists schemas
# shell.exec('cat /db/public/users/.info/count') → row count
# shell.exec('echo hello > /home/agent/test.txt && cat /home/agent/test.txt') → "hello"
```

## Development Heuristics

- PgFs is read-only: all write methods throw EROFS
- WorkspaceFs delivers complete content per writeFile() — no buffering
- Path parsing replaces FUSE inodes: `parsePath()` → PgNode, not inode tables
- SQL queries from Rust transfer verbatim (quoteIdent + $N params + ::text casts)
- Command safety follows the pi-coding-agent pattern: AST walk + regex fallback
- The Supabase docs shell is the reference for MountableFs + customCommands + executionLimits + defenseInDepth
- just-bash's Python WASM runtime routes open() through IFileSystem (agent subagents see virtual FS)

## Migration Contract

`openeral-js` auto-runs migrations via `runMigrations()` in `createOpeneralShell()`.
The schema is `_openeral` with tables: `workspace_config`, `workspace_files`,
`schema_version`, `mount_log`, `cache_hints`.

Migrations must be idempotent (IF NOT EXISTS).
