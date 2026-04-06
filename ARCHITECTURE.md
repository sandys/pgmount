# Architecture

## Overview

```
Agent ──bash tool──► openeral-bash ──► just-bash (TypeScript)
                                          │
                                     MountableFs
                                     ├── /db         → PgFs (read-only SQL)
                                     ├── /home/agent → WorkspaceFs (read-write PostgreSQL)
                                     └── /tmp        → InMemoryFs
```

For Claude Code (`npx openeral`):

```
                    ┌─────────────┐
PostgreSQL ◄──sync──┤ /home/agent ├──► Claude Code (Read, Write, Edit, Bash, ...)
                    └──────┬──────┘
                      file watcher
                           │
                    sync on change ──► PostgreSQL
```

## Components

### openeral-js (`openeral-js/`)

TypeScript package. Two filesystem implementations on just-bash's `IFileSystem` interface:

- **PgFs** (`src/pg-fs/`) — read-only. Parses paths into a `PgNode` discriminated union, dispatches to SQL queries. Generates content on-the-fly from live database. Caches schema metadata with TTL.
- **WorkspaceFs** (`src/workspace-fs/`) — read-write. Direct SQL CRUD against `_openeral.workspace_files`. Every write persists immediately.

Supporting modules:

- **sync** (`src/sync.ts`) — bidirectional sync between PostgreSQL and real filesystem. Used by the CLI for Claude Code, which needs real files for its Read/Write/Edit tools.
- **safety** (`src/safety.ts`) — command analysis via just-bash's `parse()` AST. Classifies commands as safe/destructive.
- **shell** (`src/shell.ts`) — `createOpeneralShell()` factory. Composes MountableFs + custom `pg` command + execution limits.
- **cli** (`src/cli.ts`) — `npx openeral` entry point. Sync + file watcher + Claude Code launcher.

### Sandbox (`sandboxes/openeral/`)

Stock OpenShell base image + Node.js + openeral-js. No custom cluster or gateway.

- **openeral-bash.mjs** — daemon/client bridge. Daemon holds a persistent just-bash shell on a Unix socket. Each `bash -c` from Claude Code connects, executes, streams output.
- **setup.sh** — entry point. Migrations → seed → daemon → Claude Code.
- **policy.yaml** — network policy for the OpenShell supervisor.

### Database schema (`_openeral`)

- `workspace_config` — workspace metadata (id, display_name, config JSONB)
- `workspace_files` — file content and metadata (workspace_id, path, content BYTEA, mode, size, timestamps)
- `schema_version`, `mount_log`, `cache_hints` — operational

### Legacy Rust (`crates/`)

Original FUSE implementation. Retained for reference, not used in the sandbox.
