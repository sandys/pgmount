# openeral-shell

A persistent shell for Claude Code. Memory, plans, sessions, and tasks survive container restarts — backed by PostgreSQL. Built on [OpenShell](https://github.com/NVIDIA/OpenShell-Community) with Claude Code pre-installed.

## Quick Start (Docker Compose)

```bash
cd openeral-shell
cp .env.example .env
# Edit .env — set ANTHROPIC_API_KEY=sk-ant-...
docker compose up -d
docker compose exec openeral-shell claude
```

## Quick Start (OpenShell)

```bash
# Build the sandbox image (one time, from repo root)
docker build -t openshell-openeral-shell -f openeral-shell/Dockerfile .

# Create and run
openshell sandbox create --from openeral-shell \
  -e DATABASE_URL="postgres://user:pass@host/db" \
  -e ANTHROPIC_API_KEY="sk-ant-..." \
  -- openeral-shell-start
```

## What Claude Code Gets

- **Persistent `~/.claude/`** — memory, plans, sessions, tasks, todos, skills
- **`/db/`** — your PostgreSQL database browsable as files (read-only)
- **`/home/agent/`** — full read-write home directory, all files persist
- **Pre-installed skill** teaching Claude how to use the environment

## Using Claude Code

```bash
# Interactive session
docker compose exec openeral-shell claude

# Choose a model
docker compose exec openeral-shell claude --model claude-sonnet-4-6

# One-shot prompt
docker compose exec openeral-shell claude -p "explain the database schema at /db/"

# Write output to a persistent file
docker compose exec openeral-shell claude -p "write a plan to /home/agent/plans/my-plan.md" \
  --allowedTools "Write,Read,Bash"
```

### Example: Generate a plan with Sonnet

```bash
docker compose exec openeral-shell claude -p \
  "Write an implementation plan for a Notion clone to /home/agent/plans/notion.md" \
  --model claude-sonnet-4-6 \
  --allowedTools "Write,Read,Bash"
```

The plan file persists at `/home/agent/plans/notion.md`. Restart the container — it's still there.

## What Persists

Everything under `/home/agent/` is stored in PostgreSQL:

| Path | What |
|------|------|
| `~/.claude/memory/` | Remembered context across conversations |
| `~/.claude/plans/` | Implementation plans |
| `~/.claude/sessions/` | Conversation transcripts |
| `~/.claude/tasks/` | Task tracking |
| `~/.claude/todos/` | Todo lists |
| `~/.claude/skills/` | Agent skills |
| `~/.config/`, `~/.cache/` | Application config and cache |
| Any file you create | Stored in PostgreSQL |

Data survives `docker compose down`/`up`. To wipe: `docker compose down -v`.

## Connect Your Own Database

By default a bundled PostgreSQL is included. To browse your own database at `/db/`:

```env
# In .env
DATABASE_URL=postgres://user:pass@your-host/your-db
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | *(required)* | Anthropic API key for Claude Code |
| `DATABASE_URL` | bundled postgres | PostgreSQL connection string |
| `WORKSPACE_ID` | `default` | Isolate state per agent |
| `WORKSPACE_CONFIG` | *(broad defaults)* | JSON: auto_dirs, seed_files |
| `STARTUP_TIMEOUT` | `15` | Seconds to wait for mounts |

## Multiple Agents

Use `WORKSPACE_ID` to give each agent its own isolated home:

```env
WORKSPACE_ID=agent-alice
```

## Running Tests

```bash
bash openeral-shell/tests/test_openeral_shell.sh
```

## Security

openeral-shell extends the [OpenShell base sandbox](https://github.com/NVIDIA/OpenShell-Community) with:

- **Landlock policy** — `/db/` read-only, `/home/agent/` read-write, system dirs locked
- **FUSE isolation** — database and workspace are separate FUSE mounts
- **Non-root execution** — runs as `sandbox` user (UID 1000)
- **SYS_ADMIN scoped** — only for FUSE mount operations
