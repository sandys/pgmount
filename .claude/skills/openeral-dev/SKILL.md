---
name: openeral-dev
description: Develop openeral-js — persistent /home/agent + read-only /db for AI agents via just-bash + PostgreSQL
disable-model-invocation: false
user-invocable: true
allowed-tools: Read, Grep, Glob, Bash
argument-hint: [task description]
---

# OpenEral Development

OpenEral gives AI agents persistent `/home/agent` and read-only `/db`, backed by PostgreSQL, via just-bash. Works with stock OpenShell — no custom cluster or gateway.

## Key Files

```
openeral-js/src/
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
  openeral-bash.mjs           # Daemon/client bridge for Claude Code
  setup.sh                    # Entry point: migrate → seed → daemon → claude
  policy.yaml                 # Network policy + boundary secret injection
```

## Build & Verify

```bash
cd openeral-js
pnpm check                                      # typecheck + lint + unit tests
DATABASE_URL='...' node test-integration.mjs     # 34 tests against live PostgreSQL
DATABASE_URL='...' ANTHROPIC_API_KEY='...' node test-e2e-claude.mjs  # 45 tests, 3 sessions
```

## Structural Lints (lint.mjs)

8 rules that prevent known bug classes:
1. All local imports resolve to existing .ts files
2. All named imports match actual exports
3. just-bash version >= 2.x
4. shell.ts auto-creates workspace_config and seeds root
5. PgFs write methods throw EROFS
6. No write-back buffering in WorkspaceFs
7. No FUSE references in sandbox Dockerfile
8. pg custom command defined in shell.ts

## Conventions

- PgFs is read-only — all write methods throw EROFS
- WorkspaceFs receives complete content per writeFile() — no buffering
- Path parsing replaces FUSE inodes: `parsePath()` → PgNode
- SQL uses `quoteIdent()` + `$N` params + `::text` casts
- `pg` command: complex SQL must be double-quoted
- Command safety: AST walk + regex fallback (pi-coding-agent pattern)
- Shell factory: MountableFs + customCommands + executionLimits + defenseInDepth (Supabase pattern)

## Sandbox

Uses stock OpenShell base image. No custom cluster or gateway images. The openeral-bash daemon holds a persistent just-bash shell on a Unix socket — each `bash -c` from Claude Code connects, executes, streams output.

## Migrations

Auto-run in `createOpeneralShell()`. Schema: `_openeral` with tables `workspace_config`, `workspace_files`, `schema_version`, `mount_log`, `cache_hints`. Must be idempotent.
