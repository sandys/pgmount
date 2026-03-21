# openeral-shell

A persistent shell for Claude Code. Memory, plans, sessions, and tasks survive container restarts — backed by PostgreSQL. Claude Code comes pre-installed.

## Quick Start

```bash
cd openeral-shell
cp .env.example .env
# Edit .env — set ANTHROPIC_API_KEY=sk-ant-...
docker compose up -d
docker compose exec openeral-shell claude
```

Claude Code starts with a persistent home directory. Everything it writes — memory, plans, sessions, tasks — is stored in PostgreSQL and survives `docker compose down`/`up`.

## What Claude Code Gets

- **Persistent `~/.claude/`** — memory, plans, sessions, tasks, todos, skills
- **`/db/`** — your PostgreSQL database browsable as files (read-only)
- **`/home/agent/`** — full read-write home directory, all files persist
- **Pre-installed skill** teaching Claude how to browse `/db/` and use the environment

## Setup

### 1. Set your API key

```bash
cd openeral-shell
cp .env.example .env
```

Edit `.env`:

```env
ANTHROPIC_API_KEY=sk-ant-api03-your-key-here
```

### 2. Start

```bash
docker compose up -d
```

First build compiles from source (~5 min). Subsequent starts are instant.

### 3. Use Claude Code

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

# Just a shell
docker compose exec openeral-shell bash
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

Persistence is backed by a Docker volume (`pgdata`). Data survives `docker compose down`/`up`. To wipe everything: `docker compose down -v`.

## Connect Your Own Database

By default a bundled PostgreSQL is included. To browse your own database at `/db/`:

```bash
# In .env
DATABASE_URL=postgres://user:pass@your-host/your-db
```

Or override directly:

```bash
DATABASE_URL="postgres://user:pass@your-host/your-db" docker compose up -d
```

## Environment Variables

Set in `.env` or `docker-compose.yml`:

| Variable | Default | Description |
|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | *(required)* | Anthropic API key for Claude Code |
| `DATABASE_URL` | bundled postgres | PostgreSQL connection string |
| `WORKSPACE_ID` | `default` | Isolate state per agent |
| `WORKSPACE_CONFIG` | *(broad defaults)* | JSON: auto_dirs, seed_files |
| `STARTUP_TIMEOUT` | `15` | Seconds to wait for mounts |

## Multiple Agents

Use `WORKSPACE_ID` to give each agent its own isolated home directory:

```env
WORKSPACE_ID=agent-alice
```

## Custom Directories

The default auto-created directories cover Claude Code and common agents:

```
.claude/  .claude/memory/  .claude/plans/  .claude/sessions/
.claude/tasks/  .claude/todos/  .claude/skills/
.cache/  .local/  .config/  .npm/
```

Override with `WORKSPACE_CONFIG`:

```env
WORKSPACE_CONFIG={"auto_dirs":[".claude",".claude/memory",".myagent/data"],"seed_files":{".bashrc":"export PS1='agent> '"}}
```

## Running Tests

```bash
bash openeral-shell/tests/test_openeral_shell.sh
```

## Docker Requirements

Already configured in `docker-compose.yml`. For reference:

```yaml
devices:
  - /dev/fuse
cap_add:
  - SYS_ADMIN
security_opt:
  - apparmor:unconfined
```
