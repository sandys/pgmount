# OpenEral

Persistent home directory and database access for AI agents, backed by PostgreSQL.

## Run Claude Code with OpenEral

You need two things: a PostgreSQL database and an Anthropic API key.

### Quickest: inside Claude Code

If you're already in Claude Code, just say:

> I want to run Claude Code using openeral-shell

Claude will handle the rest — it clones the repo, builds, and launches.

### From your terminal

```bash
git clone https://github.com/sandys/openeral.git
cd openeral/openeral-js
pnpm install && pnpm build

export DATABASE_URL='postgresql://user:pass@host:5432/dbname'
export ANTHROPIC_API_KEY='sk-ant-...'

npx openeral
```

That's it. Claude Code starts with a home directory that persists to PostgreSQL.

### Via OpenShell

```bash
export DATABASE_URL='postgresql://user:pass@host:5432/dbname'

openshell gateway start

openshell provider create \
  --name db --type generic --credential DATABASE_URL

openshell sandbox create \
  --from ghcr.io/sandys/openeral/sandbox:just-bash \
  --provider db --provider claude --auto-providers \
  -- /opt/openeral/setup.sh
```

Stock OpenShell — no custom cluster or gateway images.

## What you get

- **Persistent home** — files written to `$HOME` survive across sessions, backed by PostgreSQL
- **Database access** — `pg "SELECT * FROM users LIMIT 5"` from Claude's bash
- **Automatic sync** — file changes sync to PostgreSQL in the background, final save on exit
- **Session isolation** — different `OPENERAL_WORKSPACE_ID` = different workspace

## Persistence

Same machine = same workspace (keyed to hostname by default).

```bash
# Session 1
npx openeral -- -p 'Write "hello" to $HOME/notes.txt' --dangerously-skip-permissions

# Session 2 — file is still there
npx openeral -- -p 'Run: cat $HOME/notes.txt' --dangerously-skip-permissions
# → hello
```

Use `$HOME/` (not `~/`) in prompts — Claude Code's file tools resolve `~` to the OS user home.

Multiple workspaces:

```bash
OPENERAL_WORKSPACE_ID=project-alpha npx openeral
```

## Database access

Claude can query the connected database:

```bash
pg "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'"
pg "SELECT * FROM public.users LIMIT 5"
pg "\d public.users"
```

The `pg` command is automatically available — OpenEral writes a `CLAUDE.md` that teaches Claude how to use it.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | (required) | PostgreSQL connection string |
| `ANTHROPIC_API_KEY` | (required for Claude) | Anthropic API key |
| `OPENERAL_WORKSPACE_ID` | hostname | Workspace identifier |
| `OPENERAL_HOME` | `/tmp/openeral-<id>` | Local workspace directory |

## How it works

```
                    ┌─────────────┐
PostgreSQL ◄──sync──┤ /home/agent ├──► Claude Code (Read, Write, Edit, Bash, ...)
                    └──────┬──────┘
                      file watcher
                           │
                    sync on change ──► PostgreSQL
```

On startup, OpenEral restores your workspace from PostgreSQL to a real directory. Claude Code runs normally — all its tools (Read, Write, Edit, Bash, Glob, Grep) work on real files. A background watcher syncs changes back to PostgreSQL. On exit, a final sync saves everything.

## Custom agents

For agents with a single bash tool (not the Claude Code CLI), use the just-bash virtual filesystem directly:

```typescript
import { createOpeneralShell, createToolHandler } from 'openeral-js'

const shell = await createOpeneralShell({
  connectionString: process.env.DATABASE_URL,
  workspaceId: 'my-session',
})

const handleBash = createToolHandler(shell)
await shell.exec('cat /db/public/users/.info/count')   // → "42\n"
await shell.exec('echo hello > /home/agent/notes.txt') // persisted
```

This path uses [just-bash](https://github.com/vercel-labs/just-bash) with PostgreSQL-backed virtual mounts at `/db` (read-only) and `/home/agent` (read-write).

## Build & test

```bash
cd openeral-js
pnpm install && pnpm build
pnpm check                    # typecheck + lint + unit tests

DATABASE_URL='...' node test-integration.mjs
DATABASE_URL='...' ANTHROPIC_API_KEY='...' node test-e2e-claude.mjs
```

## Project structure

```
openeral-js/                  # TypeScript package
  src/cli.ts                  # npx openeral entry point
  src/sync.ts                 # PostgreSQL ↔ filesystem sync
  src/shell.ts                # createOpeneralShell() for custom agents
  src/pg-fs/                  # Read-only /db filesystem
  src/workspace-fs/           # Read-write /home/agent filesystem
  src/db/                     # SQL queries, migrations
  src/safety.ts               # Command safety analysis
  lint.mjs                    # 20 structural lint rules

sandboxes/openeral/           # OpenShell sandbox image
  Dockerfile                  # Stock base + Node.js + openeral-js
  setup.sh                    # Sandbox entry point
  policy.yaml                 # Network policy
```
