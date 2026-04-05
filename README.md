# OpenEral

Persistent home directory and database access for AI agents, backed by PostgreSQL.

## Claude Code

```bash
export DATABASE_URL='postgresql://user:pass@host:5432/dbname'
export ANTHROPIC_API_KEY='sk-ant-...'

npx openeral
```

That's it. Claude Code starts with a persistent home directory backed by PostgreSQL. Files survive across sessions. A `pg` command gives Claude direct database access.

### What happens

1. OpenEral connects to your PostgreSQL database
2. Restores your workspace from a previous session (if any)
3. Starts Claude Code with `HOME` pointing to the persistent workspace
4. Watches for file changes and syncs them to PostgreSQL in the background
5. On exit, saves everything

### Persistence

Same machine = same workspace (keyed to hostname by default).

```bash
# Session 1: Claude writes a file
npx openeral -- -p 'Write "hello" to ~/notes.txt'

# Session 2: it's still there
npx openeral -- -p 'Read ~/notes.txt'
# → hello
```

Override the workspace ID to manage multiple workspaces:

```bash
OPENERAL_WORKSPACE_ID=project-alpha npx openeral
```

### Database access

Claude can query the connected database using the `pg` command:

```bash
pg "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'"
pg "SELECT * FROM public.users LIMIT 5"
pg "\d public.users"
```

This is automatically available — OpenEral writes a `CLAUDE.md` in the home directory that teaches Claude how to use it.

### Options

| Environment Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | (required) | PostgreSQL connection string |
| `ANTHROPIC_API_KEY` | (required) | Claude API key |
| `OPENERAL_WORKSPACE_ID` | hostname | Workspace identifier for persistence |
| `OPENERAL_HOME` | `/tmp/openeral-<id>` | Local directory for the workspace |

## OpenShell

Works with stock [OpenShell](https://github.com/nvidia/openshell-community) — no custom cluster or gateway images.

```bash
openshell gateway start

openshell provider create \
  --name db --type generic --credential DATABASE_URL

openshell sandbox create \
  --from ghcr.io/<owner>/openeral/sandbox:latest \
  --provider db --provider claude --auto-providers \
  -- /opt/openeral/setup.sh
```

Build the sandbox image: `docker build -f sandboxes/openeral/Dockerfile -t openeral/sandbox:latest .`

## Custom Agents

For agents with a single bash tool (not the Claude Code CLI):

```typescript
import { createOpeneralShell, createToolHandler } from 'openeral-js'

const shell = await createOpeneralShell({
  connectionString: process.env.DATABASE_URL,
  workspaceId: 'my-session',
})

// Use as an agent tool handler
const handleBash = createToolHandler(shell)

// Or call directly
await shell.exec('cat /db/public/users/.info/count')   // → "42\n"
await shell.exec('echo hello > /home/agent/notes.txt') // persisted
await shell.exec('cat /home/agent/notes.txt')           // → "hello\n"
```

This path uses [just-bash](https://github.com/vercel-labs/just-bash) — a TypeScript bash interpreter that intercepts all file operations. The agent sees `/db` (read-only database) and `/home/agent` (persistent workspace) as normal directories.

## How It Works

### Claude Code path (`npx openeral`)

```
                    ┌─────────────┐
PostgreSQL ◄──sync──┤ /home/agent ├──► Claude Code (Read, Write, Edit, Bash, ...)
                    └──────┬──────┘
                      file watcher
                           │
                    sync on change ──► PostgreSQL
```

Claude Code uses the real filesystem. OpenEral syncs it to/from PostgreSQL on startup, on file changes, and on exit. The `pg` command provides database access via Claude's Bash tool.

### Custom agent path (`createOpeneralShell`)

```
Agent ──bash tool──► just-bash ──► MountableFs
                                   ├── /db         → PgFs (read-only SQL)
                                   ├── /home/agent → WorkspaceFs (read-write PostgreSQL)
                                   └── /tmp        → InMemoryFs
```

All file operations route through just-bash with PostgreSQL-backed virtual filesystems.

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
openeral-js/
  src/
    cli.ts              # npx openeral — Claude Code entry point
    sync.ts             # PostgreSQL ↔ real filesystem sync
    shell.ts            # createOpeneralShell() for custom agents
    pg-fs/              # Read-only /db filesystem (just-bash IFileSystem)
    workspace-fs/       # Read-write /home/agent (just-bash IFileSystem)
    db/                 # SQL queries, migrations, pool
    safety.ts           # Command safety analysis
  lint.mjs              # 8 structural lint rules

sandboxes/openeral/
  Dockerfile            # OpenShell sandbox (stock base + openeral-js)
  setup.sh              # Sandbox entry point
  policy.yaml           # Network policy
```
