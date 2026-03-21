# openeral-shell

A persistent shell environment for AI agents. Your home directory, config, memory, and plans survive container restarts — backed by PostgreSQL.

## Quick Start

```bash
cd openeral-shell
docker compose up -d
docker compose exec openeral-shell bash
```

That's it. You have:

- **`/db/`** — your PostgreSQL database browsable as files (read-only)
- **`$HOME`** (`/home/agent`) — persistent home directory (read-write)

Everything under `$HOME` is stored in PostgreSQL and survives `docker compose down`/`up`.

## Claude Code

```bash
docker compose exec -e ANTHROPIC_API_KEY=sk-ant-... openeral-shell claude
```

Claude Code's `~/.claude/` directory (memory, plans, sessions, tasks, settings) persists across container restarts.

## Use Your Own PostgreSQL

By default, a bundled PostgreSQL is included. To use your own:

```bash
DATABASE_URL="postgres://user:pass@your-host/your-db" docker compose up -d
```

Or edit `docker-compose.yml` and set the `DATABASE_URL` environment variable directly.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | bundled postgres | PostgreSQL connection string |
| `ANTHROPIC_API_KEY` | *(none)* | API key for Claude Code |
| `WORKSPACE_ID` | `default` | Isolate state per agent |
| `WORKSPACE_CONFIG` | *(broad defaults)* | JSON: auto_dirs, seed_files |
| `STARTUP_TIMEOUT` | `15` | Seconds to wait for mounts |

## Default Directories

These directories are auto-created under `$HOME` on first start:

```
.claude/          .claude/memory/    .claude/plans/
.claude/sessions/ .claude/tasks/     .claude/todos/
.claude/skills/   .cache/            .local/
.config/          .npm/
```

Override with `WORKSPACE_CONFIG`:

```bash
WORKSPACE_CONFIG='{"auto_dirs":[".claude",".myagent/data"],"seed_files":{".bashrc":"export PS1=\"agent> \""}}'
```

## What Persists

Everything under `$HOME` is stored in PostgreSQL:

| Path | What |
|------|------|
| `~/.claude/memory/` | Remembered context |
| `~/.claude/plans/` | Implementation plans |
| `~/.claude/sessions/` | Conversation transcripts |
| `~/.config/` | Application config |
| `~/.cache/` | Cached data |
| Any file you create | Stored in PostgreSQL |

PostgreSQL data persists via the `pgdata` Docker volume.

## Multiple Agents

Use `WORKSPACE_ID` to isolate state per agent:

```yaml
# In docker-compose.yml
environment:
  WORKSPACE_ID: agent-alice
```

Each workspace ID gets its own isolated home directory in PostgreSQL.

## Skills

openeral-shell includes a built-in skill at `~/.claude/skills/openeral-shell/SKILL.md` that teaches AI agents how to use the environment — browsing `/db/`, writing persistent files, filtering, exporting, etc. The skill is automatically copied into the workspace on first start.

To add your own skills, place them in `openeral-shell/skills/` before building, or write them directly to `~/.claude/skills/` inside a running shell.

## Running Tests

From the repo root:

```bash
bash openeral-shell/tests/test_openeral_shell.sh
```

This builds the image, starts services, and runs assertions for:
- Mounts (`/db` and `$HOME`)
- Default directories (`.claude/*`, `.cache`, `.config`, `.local`, `.npm`)
- File read/write/delete
- `/db` is read-only
- Persistence across `docker compose down`/`up`
- PostgreSQL contains workspace data

## Docker Requirements

The container needs FUSE support:

```yaml
devices:
  - /dev/fuse
cap_add:
  - SYS_ADMIN
security_opt:
  - apparmor:unconfined
```

These are already configured in the provided `docker-compose.yml`.
