---
name: openeral-dev
description: Develop openeral-js — isolated home + optional PostgreSQL persistence + database access for AI agents
disable-model-invocation: false
user-invocable: true
allowed-tools: Read, Grep, Glob, Bash
argument-hint: [task description]
---

# OpenEral Development

OpenEral gives AI agents an isolated home directory with optional PostgreSQL-backed persistence and database access. Works with stock OpenShell or standalone via `npx openeral`.

## Key Files

```
openeral-js/src/
  cli.ts                      # npx openeral entry point (persistence optional)
  sync.ts                     # PostgreSQL ↔ real filesystem sync
  pg-fs/pg-fs.ts              # PgFs: read-only IFileSystem → SQL queries
  pg-fs/path-parser.ts        # parsePath() → PgNode discriminated union
  workspace-fs/workspace-fs.ts # WorkspaceFs: read-write → workspace_files table
  db/queries.ts               # All SQL (introspection, rows, stats, indexes)
  db/workspace-queries.ts     # Workspace CRUD
  db/migrations.ts            # V1-V4 schema migrations
  safety.ts                   # Command analysis via just-bash parse() AST
  shell.ts                    # createOpeneralShell(), createToolHandler()

sandboxes/openeral/
  Dockerfile                  # Stock OpenShell base + Node.js + openeral-js
  openeral-bash.mjs           # Daemon/client bridge for custom agents
  setup.sh                    # Sandbox entry point
  policy.yaml                 # Network policy
```

## Build & Verify

```bash
cd openeral-js
pnpm install && pnpm build
pnpm check                                      # typecheck + 24 lints + unit tests
DATABASE_URL='...' node test-integration.mjs     # 34 tests against live PostgreSQL
DATABASE_URL='...' ANTHROPIC_API_KEY='...' node test-e2e-claude.mjs  # 45+ tests, 3 sessions
```

## Structural Lints (lint.mjs — 24 rules)

Key rules: imports resolve, exports match, just-bash >=2.x, PgFs throws EROFS, no write-back buffering, no FUSE in Dockerfile, no hardcoded credentials, sync persists deletions, sync preserves modes, exclude uses exact matching, syncToFs prunes stale files, syncToFs prunes before creating, pruneLocal handles type conflicts, README includes build steps, migrations use advisory lock, skill checks node_modules, no fork-specific policy fields (secret_injection/egress_via), Socket.dev endpoint has TLS terminate.

## Conventions

- PgFs is read-only — all write methods throw EROFS
- WorkspaceFs receives complete content per writeFile() — no buffering
- Path parsing replaces FUSE inodes: `parsePath()` → PgNode
- SQL uses `quoteIdent()` + `$N` params + `::text` casts
- `pg` command: complex SQL must be double-quoted
- Command safety: AST walk + regex fallback
- Persistence is optional — CLI works without DATABASE_URL (local-only mode)
- Never hardcode credentials — always read from environment at runtime

## Migrations

Auto-run in `createOpeneralShell()` and CLI. Schema: `_openeral` with tables `workspace_config`, `workspace_files`, `schema_version`, `mount_log`, `cache_hints`. Must be idempotent.
