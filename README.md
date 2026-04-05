# OpenEral

OpenEral gives AI agents a persistent home directory and read-only database
access, backed by PostgreSQL. No kernel modules, no FUSE, no privileged
containers.

The agent's only tool is `bash`. OpenEral replaces the real shell with
[just-bash](https://github.com/vercel-labs/just-bash) — a TypeScript bash
interpreter with a pluggable `IFileSystem` interface — and mounts two
PostgreSQL-backed virtual filesystems:

- `/home/agent` — read-write persistent workspace (`_openeral.workspace_files`)
- `/db` — read-only navigable view of the database (schemas, tables, rows)

Every `cat`, `grep`, `ls`, `echo > file`, `python script.py` goes through
just-bash. The agent sees a normal Linux filesystem. The reality is pure
database.

## Quick Start

```typescript
import { createOpeneralShell } from 'openeral-js'

const shell = await createOpeneralShell({
  connectionString: process.env.DATABASE_URL,
  workspaceId: 'my-agent-session',
})

// Agent tool handler
const result = await shell.exec('cat /db/public/users/.info/count')
// → "42\n"

await shell.exec('echo "hello" > /home/agent/notes.txt')
await shell.exec('cat /home/agent/notes.txt')
// → "hello\n"
```

Wire it as the agent's bash tool:

```typescript
const TOOL = [{
  name: "bash",
  description: "Execute shell command",
  input_schema: {
    type: "object",
    properties: { command: { type: "string" } },
    required: ["command"],
  },
}]

async function handleBashTool(command: string) {
  const result = await shell.exec(command)
  return { stdout: result.stdout, stderr: result.stderr, exitCode: result.exitCode }
}
```

Works with Claude Code, OpenClaw, or any LLM agent framework that exposes a
single bash tool.

## What the Agent Sees

```
/db/
  public/
    users/
      .info/
        columns.json          # column metadata
        schema.sql            # CREATE TABLE DDL
        count                 # exact row count
        primary_key           # PK column names
      .export/
        data.json/page_1.json # paginated JSON export
        data.csv/page_1.csv
      .filter/status/active/  # filtered rows
      .order/name/asc/        # sorted rows
      .indexes/users_pkey     # index metadata
      page_1/
        1/
          id                  # "1"
          name                # "Alice"
          row.json            # {"id": 1, "name": "Alice", ...}
          row.csv
          row.yaml
        2/
        ...
      page_2/
        ...
  test_schema/
    ...

/home/agent/
  .claude/
    settings.json
    projects/...
  .claude.json
  work/
    notes.txt
```

## Architecture

```
Agent ──bash tool──► just-bash (TypeScript)
                        │
                   MountableFs
                   ├── /db          → PgFs (read-only, SQL queries)
                   ├── /home/agent  → WorkspaceFs (read-write, workspace_files table)
                   └── /tmp         → InMemoryFs (ephemeral)
```

### Two Filesystem Implementations

1. **PgFs** (`openeral-js/src/pg-fs/`) — read-only. Parses paths into a
   `PgNode` discriminated union, dispatches to SQL queries. Content generated
   on-the-fly from live database. Caches schema metadata with configurable TTL.

2. **WorkspaceFs** (`openeral-js/src/workspace-fs/`) — read-write. Direct SQL
   CRUD against `_openeral.workspace_files`. Persists every write to PostgreSQL
   immediately. No write-back buffering needed — just-bash's `writeFile()`
   delivers complete content in one call.

### Command Safety Layer

Uses just-bash's `parse()` function to build ASTs and classify commands before
execution. Walks the full parse tree including nested `$(...)`, resolves through
wrappers (`env`, `sudo`, `exec`), detects write redirections, and knows which
subcommands are safe (`git status` yes, `git push` no).

### Custom Commands

The `pg` command provides direct SQL access:

```bash
pg SELECT count(*) FROM public.users
```

## Configuration

```typescript
const shell = await createOpeneralShell({
  connectionString: 'postgresql://...',
  workspaceId: 'session-123',
  pageSize: 1000,        // rows per page in /db (default: 1000)
  cacheTtlMs: 30000,     // metadata cache TTL (default: 30s)
  python: true,           // enable CPython WASM runtime
  javascript: false,      // enable QuickJS runtime
  migrate: true,          // auto-run DB migrations (default: true)
  executionLimits: {      // override safety limits
    maxCommandCount: 1000,
    maxLoopIterations: 1000,
    maxOutputSize: 1024 * 1024,
  },
  env: {                  // extra environment variables
    CUSTOM_VAR: 'value',
  },
})
```

## Persistence Model

Persistence is keyed to the `workspaceId`:

- Same `workspaceId` across shell instances = same `/home/agent`
- Different `workspaceId` = fresh `/home/agent`

Files the agent typically persists:

- `~/.claude.json`
- `~/.claude/settings.json`
- `~/.claude/projects/...`
- Agent transcripts, plans, and local state

All stored as rows in `_openeral.workspace_files`.

## Database Migrations

OpenEral auto-runs migrations on `createOpeneralShell()` (unless `migrate: false`).
The schema lives in `_openeral` and includes:

- `workspace_config` — workspace metadata
- `workspace_files` — file content and metadata (primary persistence table)
- `schema_version`, `mount_log`, `cache_hints` — operational tables

## Legacy: FUSE Path

The `crates/` directory contains the original Rust + FUSE implementation. That
path requires `/dev/fuse`, kernel modules, and privileged containers. It is
retained for the OpenShell sandbox flow (`sandboxes/openeral/`) where the
supervisor manages FUSE mounts via `/etc/fstab`.

For new integrations, prefer `openeral-js` — no kernel dependencies.

## Project Structure

- `openeral-js/` — TypeScript package (just-bash + PostgreSQL virtual filesystem)
  - `src/pg-fs/` — read-only database filesystem (PgFs)
  - `src/workspace-fs/` — read-write workspace filesystem (WorkspaceFs)
  - `src/db/` — SQL queries, migrations, connection pool
  - `src/safety.ts` — command safety analysis via just-bash parser
  - `src/shell.ts` — shell factory and agent tool handler
- `crates/openeral/` — Rust CLI entry point (legacy FUSE path)
- `crates/openeral-core/` — Rust library (FUSE filesystem, DB queries)
- `crates/openeral-core/migrations/` — SQL migrations (V1-V4)
- `sandboxes/openeral/` — OpenShell sandbox image (FUSE path)
- `vendor/openshell/` — vendored OpenShell fork (custom cluster/gateway images)
- `tests/` — integration tests

## Compatibility

| Agent Framework | Works? | Notes |
|---|---|---|
| Claude Code (bash-only tool) | Yes | Complete replacement |
| OpenClaw | Yes | Agent-facing shell |
| OpenAI agents | Yes | Single bash tool |
| Any LLM with bash tool | Yes | Framework-agnostic |
| pi-coding-agent | Yes | Already uses just-bash parser |
