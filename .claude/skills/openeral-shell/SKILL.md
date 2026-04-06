---
name: openeral-shell
description: Launch Claude Code with persistent PostgreSQL-backed home directory. Handles all setup automatically.
disable-model-invocation: false
user-invocable: true
allowed-tools: Read, Bash, Grep, Glob
argument-hint: [optional: workspace ID]
---

# OpenEral Shell

Launch Claude Code with a persistent home directory backed by PostgreSQL. One command — everything is set up automatically.

## Instructions

When this skill is invoked, execute the following steps. Do NOT just show documentation — actually run the commands.

### Step 1: Check environment

Run these checks and stop with a clear message if anything is missing:

```bash
echo "DATABASE_URL=${DATABASE_URL:-(not set)}"
echo "ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY:+(set)}"
echo "OPENSHELL_SANDBOX=${OPENSHELL_SANDBOX:-0}"
```

If `DATABASE_URL` is not set, tell the user:
> Set DATABASE_URL to your PostgreSQL connection string:
> `export DATABASE_URL='postgresql://user:pass@host:5432/dbname'`

If `ANTHROPIC_API_KEY` is not set, warn but continue (Claude Code will prompt for auth).

### Step 2: Detect environment

Check if we're inside an OpenShell sandbox or on a local machine:

```bash
[ "$OPENSHELL_SANDBOX" = "1" ] && echo "openshell" || echo "local"
```

### Step 3a: Local machine path

If local (not in OpenShell sandbox):

1. Find the openeral-js directory:
```bash
# Look relative to this repo
OPENERAL_DIR="$(git rev-parse --show-toplevel 2>/dev/null)/openeral-js"
[ -d "$OPENERAL_DIR" ] || OPENERAL_DIR="$HOME/openeral/openeral-js"
[ -d "$OPENERAL_DIR" ] && echo "found: $OPENERAL_DIR" || echo "not found"
```

2. If not found, clone and build:
```bash
git clone https://github.com/sandys/openeral.git /tmp/openeral-clone
cd /tmp/openeral-clone/openeral-js
pnpm install && pnpm build
OPENERAL_DIR=/tmp/openeral-clone/openeral-js
```

3. If found but no `dist/`, build:
```bash
cd "$OPENERAL_DIR" && [ -d dist ] || (pnpm install && pnpm build)
```

4. Launch:
```bash
cd "$OPENERAL_DIR" && npx openeral
```

If the user provided a workspace ID argument, pass it:
```bash
cd "$OPENERAL_DIR" && OPENERAL_WORKSPACE_ID="<argument>" npx openeral
```

### Step 3b: OpenShell sandbox path

If inside an OpenShell sandbox (`OPENSHELL_SANDBOX=1`):

The sandbox image should already have openeral-js installed at `/opt/openeral/`. Just run:

```bash
/opt/openeral/setup.sh
```

If `/opt/openeral/` doesn't exist, the sandbox wasn't built from the openeral image. Tell the user:
> This sandbox doesn't have openeral installed. Launch with the openeral image:
> ```
> openshell sandbox create \
>   --from ghcr.io/sandys/openeral/sandbox:just-bash \
>   --provider db --provider claude --auto-providers \
>   -- /opt/openeral/setup.sh
> ```

### Step 3c: Launch via OpenShell from outside

If the user wants to launch via OpenShell (they mentioned "openshell" but aren't inside a sandbox):

```bash
# Check openshell is installed
which openshell || echo "Install openshell first: https://github.com/NVIDIA/OpenShell"

# Start gateway if not running
openshell gateway list 2>/dev/null | grep -q running || openshell gateway start

# Create db provider (idempotent)
openshell provider create --name db --type generic --credential DATABASE_URL 2>/dev/null || true

# Launch
openshell sandbox create \
  --from ghcr.io/sandys/openeral/sandbox:just-bash \
  --provider db --provider claude --auto-providers \
  -- /opt/openeral/setup.sh
```

## What happens after launch

- Claude Code starts with `HOME` pointing to a persistent workspace
- All files written to `$HOME` are synced to PostgreSQL
- Files survive across sessions (same workspace ID = same files)
- The `pg` command is available for database queries: `pg "SELECT * FROM users LIMIT 5"`
