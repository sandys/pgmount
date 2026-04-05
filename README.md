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

## OpenShell Sandbox

OpenEral works with **stock OpenShell** — no custom cluster or gateway images
needed. Just build one sandbox image.

### Launch Claude Code in OpenShell

```bash
# Stock upstream openshell
openshell gateway start

# Create a database provider
openshell provider create \
  --name db \
  --type generic \
  --credential DATABASE_URL

# Launch Claude Code with persistent /home/agent + /db
openshell sandbox create \
  --from ghcr.io/<owner>/openeral/sandbox:latest \
  --provider db \
  --provider claude \
  --auto-providers \
  -- /opt/openeral/setup.sh
```

No `OPENSHELL_CLUSTER_IMAGE`, no `IMAGE_REPO_BASE`, no 3-image version lock.

### How it works in the sandbox

1. `setup.sh` runs migrations against `$DATABASE_URL` (injected by provider)
2. Seeds the workspace (creates `/home/agent/.claude/` dirs)
3. Starts the `openeral-bash` daemon (persistent just-bash shell on Unix socket)
4. Launches Claude Code with `HOME=/home/agent SHELL=/usr/local/bin/openeral-bash`

Claude Code's bash commands route through `openeral-bash` → daemon → just-bash →
PostgreSQL. The agent sees `/db` and `/home/agent` as normal directories.

### Build the sandbox image

```bash
docker build -f sandboxes/openeral/Dockerfile -t openeral/sandbox:latest .
```

## Project Structure

- `openeral-js/` — TypeScript package (just-bash + PostgreSQL virtual filesystem)
  - `src/pg-fs/` — read-only database filesystem (PgFs)
  - `src/workspace-fs/` — read-write workspace filesystem (WorkspaceFs)
  - `src/db/` — SQL queries, migrations, connection pool
  - `src/safety.ts` — command safety analysis via just-bash parser
  - `src/shell.ts` — shell factory and agent tool handler
- `sandboxes/openeral/` — OpenShell sandbox image
  - `Dockerfile` — stock base + Node.js + openeral-js
  - `openeral-bash.mjs` — daemon/client bridge for Claude Code
  - `setup.sh` — sandbox entry point
  - `policy.yaml` — network policy (Claude API, GitHub, etc.)
- `crates/` — legacy Rust + FUSE implementation (retained, not used in sandbox)
- `tests/` — integration tests

## Compatibility

| Agent Framework | Works? | Notes |
|---|---|---|
| Claude Code (bash-only tool) | Yes | Complete replacement |
| OpenClaw | Yes | Agent-facing shell |
| OpenAI agents | Yes | Single bash tool |
| Any LLM with bash tool | Yes | Framework-agnostic |
| pi-coding-agent | Yes | Already uses just-bash parser |
