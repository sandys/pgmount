# openeral-shell

A persistent shell for Claude Code. Memory, plans, sessions, and tasks survive container restarts — backed by PostgreSQL. Built on [OpenShell](https://github.com/NVIDIA/OpenShell-Community) with Claude Code pre-installed.

## Quick Start

```bash
# Create .env with your credentials (from repo root)
cat > .env <<EOF
DATABASE_URL=postgres://user:pass@host/db
ANTHROPIC_API_KEY=sk-ant-...
EOF

# Create the sandbox
openshell sandbox create --from . \
  --upload .env:/sandbox/.env \
  --policy openeral-shell/policy.yaml

# Connect and run Claude Code
openshell sandbox connect <sandbox-name> -- claude
```

## What Claude Code Gets

- **Persistent `~/.claude/`** — memory, plans, sessions, tasks, todos, skills
- **`/db/`** — your PostgreSQL database browsable as files (read-only)
- **`/home/agent/`** — full read-write home directory, all files persist
- **Pre-installed skill** teaching Claude how to use the environment

## Using Claude Code

```bash
# Interactive session
openshell sandbox connect <sandbox-name> -- claude

# Choose a model
openshell sandbox connect <sandbox-name> -- claude --model claude-sonnet-4-6

# One-shot prompt
openshell sandbox connect <sandbox-name> -- claude -p "explain the database schema at /db/"
```

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

## Environment Variables

Set these in your `.env` file:

| Variable | Default | Description |
|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | *(required)* | Anthropic API key for Claude Code |
| `DATABASE_URL` | *(required)* | PostgreSQL connection string |
| `WORKSPACE_ID` | `default` | Isolate state per agent |
| `WORKSPACE_CONFIG` | *(broad defaults)* | JSON: auto_dirs, seed_files |
| `STARTUP_TIMEOUT` | `15` | Seconds to wait for mounts |

## Multiple Agents

Use `WORKSPACE_ID` to give each agent its own isolated home:

```env
WORKSPACE_ID=agent-alice
```

## Security

openeral-shell extends the [OpenShell base sandbox](https://github.com/NVIDIA/OpenShell-Community) with:

- **Landlock policy** — `/db/` read-only, `/home/agent/` read-write, system dirs locked
- **FUSE isolation** — database and workspace are separate FUSE mounts
- **Non-root execution** — runs as `sandbox` user (UID 1000)
- **SYS_ADMIN scoped** — only for FUSE mount operations
