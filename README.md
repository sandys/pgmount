# OpenEral

Persistent home directory and read-only database access for AI agents, backed by PostgreSQL. Works with stock [OpenShell](https://github.com/nvidia/openshell-community) — no custom cluster, no custom gateway, no FUSE, no kernel modules.

Built on [just-bash](https://github.com/vercel-labs/just-bash) (TypeScript bash interpreter with pluggable `IFileSystem`). The agent's bash tool routes through just-bash with two PostgreSQL-backed virtual mounts:

- `/home/agent` — read-write persistent workspace
- `/db` — read-only navigable view of the database

## Quick Start

### As a library

```typescript
import { createOpeneralShell } from 'openeral-js'

const shell = await createOpeneralShell({
  connectionString: process.env.DATABASE_URL,
  workspaceId: 'my-session',
})

await shell.exec('cat /db/public/users/.info/count')   // → "42\n"
await shell.exec('echo hello > /home/agent/notes.txt') // persisted to PostgreSQL
await shell.exec('cat /home/agent/notes.txt')           // → "hello\n"
```

### In OpenShell (stock, no fork)

```bash
openshell gateway start

openshell provider create \
  --name db --type generic --credential DATABASE_URL

openshell sandbox create \
  --from ghcr.io/<owner>/openeral/sandbox:latest \
  --provider db --provider claude --auto-providers \
  -- /opt/openeral/setup.sh
```

Claude Code starts with `HOME=/home/agent`. The `openeral-bash` daemon intercepts every `bash -c` call and routes it through just-bash + PostgreSQL.

### As an agent tool

```python
TOOL = [{
    "name": "bash",
    "description": "Execute shell command",
    "input_schema": {
        "type": "object",
        "properties": {"command": {"type": "string"}},
        "required": ["command"],
    },
}]
```

```typescript
import { createOpeneralShell, createToolHandler } from 'openeral-js'

const shell = await createOpeneralShell({ connectionString, workspaceId })
const handleBash = createToolHandler(shell)
```

## What the Agent Sees

```
/db/
  public/
    users/
      .info/columns.json, schema.sql, count, primary_key
      .export/data.json/page_1.json, data.csv/, data.yaml/
      .filter/<column>/<value>/          # filtered rows
      .order/<column>/asc|desc/          # sorted rows
      .indexes/<index_name>              # index metadata
      page_1/
        1/id, name, row.json, row.csv    # row data as files
      page_2/...
  other_schema/...

/home/agent/
  .claude.json
  .claude/settings.json, projects/...
  <anything the agent writes>

/tmp/                                    # ephemeral, in-memory
```

## How It Works

```
Agent ──bash tool──► openeral-bash ──► just-bash (TypeScript)
                                          │
                                     MountableFs
                                     ├── /db         → PgFs (read-only SQL)
                                     ├── /home/agent → WorkspaceFs (read-write PostgreSQL)
                                     └── /tmp        → InMemoryFs
```

**PgFs** parses paths into a `PgNode` discriminated union, dispatches to SQL queries, generates content on-the-fly. Schema metadata cached with configurable TTL.

**WorkspaceFs** does direct SQL CRUD against `_openeral.workspace_files`. Every write persists immediately — no buffering.

**Command safety** uses just-bash's `parse()` to build ASTs and classify commands. Resolves through wrappers (`env`, `sudo`, `exec`), detects write redirections, knows which subcommands are safe.

**`pg` command** provides direct SQL: `pg "SELECT count(*) FROM public.users"`

## OpenShell Sandbox

The sandbox image uses stock OpenShell base. No custom cluster or gateway needed.

`setup.sh` (the sandbox entry point):
1. Runs migrations against `$DATABASE_URL` (injected by OpenShell provider)
2. Seeds the workspace
3. Starts the `openeral-bash` daemon on a Unix socket
4. Launches Claude Code with `HOME=/home/agent SHELL=/usr/local/bin/openeral-bash`

Build: `docker build -f sandboxes/openeral/Dockerfile -t openeral/sandbox:latest .`

The policy file (`sandboxes/openeral/policy.yaml`) authorizes Claude API traffic with boundary secret injection — the child process only sees placeholder credentials.

## Persistence

Keyed to `workspaceId` (defaults to `OPENSHELL_SANDBOX_ID` in the sandbox):

- Same `workspaceId` = same `/home/agent` across sessions
- Different `workspaceId` = fresh workspace

Verified across 3 sessions: writes in session 1 survive through sessions 2 and 3. Deletes persist. Direct PostgreSQL query confirms rows in `_openeral.workspace_files`.

## Build & Test

```bash
cd openeral-js
pnpm install
pnpm check                    # typecheck + lint + unit tests

# Integration test (requires PostgreSQL)
DATABASE_URL='...' node test-integration.mjs

# E2E test (3-session persistence + Claude API)
DATABASE_URL='...' ANTHROPIC_API_KEY='...' node test-e2e-claude.mjs
```

## Project Structure

```
openeral-js/                  # TypeScript package
  src/
    pg-fs/                    # PgFs: read-only /db filesystem
      pg-fs.ts                # IFileSystem implementation
      path-parser.ts          # parsePath() → PgNode
    workspace-fs/             # WorkspaceFs: read-write /home/agent
      workspace-fs.ts         # IFileSystem implementation
    db/                       # PostgreSQL layer
      queries.ts              # SQL queries (introspection, rows, stats, indexes)
      workspace-queries.ts    # Workspace CRUD
      migrations.ts           # V1-V4 schema migrations
      pool.ts, types.ts
    safety.ts                 # Command analysis via just-bash parse()
    shell.ts                  # createOpeneralShell(), createToolHandler()
    index.ts                  # Public API
  lint.mjs                    # Structural lints (8 rules)
  test-integration.mjs        # Live PostgreSQL tests
  test-e2e-claude.mjs         # 3-session persistence + Claude API

sandboxes/openeral/           # OpenShell sandbox image
  Dockerfile                  # Stock base + Node.js + openeral-js
  openeral-bash.mjs           # Daemon/client bridge for Claude Code
  setup.sh                    # Entry point: migrate → seed → daemon → claude
  policy.yaml                 # Network policy (Claude API, GitHub, PyPI, etc.)
```
