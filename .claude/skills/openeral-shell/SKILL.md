---
name: openeral-shell
description: Launch Claude Code with an isolated home directory. Optionally backed by PostgreSQL for cross-session persistence.
disable-model-invocation: false
user-invocable: true
allowed-tools: Read, Bash, Grep, Glob
argument-hint: [optional: workspace ID]
---

# OpenEral Shell

Launch Claude Code with an isolated home directory. If `DATABASE_URL` is set, files persist to PostgreSQL across sessions. Without it, Claude Code still launches — just without persistence.

## Instructions

When this skill is invoked, execute the following steps. Do NOT just show documentation — actually run the commands.

### Step 1: Check environment

```bash
echo "DATABASE_URL=${DATABASE_URL:-(not set)}"
echo "ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY:+(set)}"
echo "OPENSHELL_SANDBOX=${OPENSHELL_SANDBOX:-0}"
```

- If `DATABASE_URL` is not set, **continue** — persistence is optional. Tell the user: "Running without persistence. Set DATABASE_URL to enable it."
- If `ANTHROPIC_API_KEY` is not set, warn but continue.

### Step 2: Detect environment

```bash
[ "$OPENSHELL_SANDBOX" = "1" ] && echo "openshell" || echo "local"
```

### Step 3a: Local machine

1. Find the openeral-js directory:
```bash
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

3. If found but missing `dist/` or `node_modules/`, install and build:
```bash
cd "$OPENERAL_DIR" && [ -d dist ] && [ -d node_modules ] || (pnpm install && pnpm build)
```

4. Launch:
```bash
cd "$OPENERAL_DIR" && npx openeral
```

If the user provided a workspace ID argument:
```bash
cd "$OPENERAL_DIR" && OPENERAL_WORKSPACE_ID="<argument>" npx openeral
```

### Step 3b: Inside OpenShell sandbox

```bash
/opt/openeral/setup.sh
```

If `/opt/openeral/` doesn't exist:
> This sandbox doesn't have openeral. Launch with the openeral image:
> `openshell sandbox create --from ghcr.io/sandys/openeral/sandbox:just-bash --provider db --provider claude --auto-providers -- /opt/openeral/setup.sh`

### Step 3c: Launch via OpenShell from outside

```bash
which openshell || echo "Install openshell: https://github.com/NVIDIA/OpenShell"
openshell gateway list 2>/dev/null | grep -q running || openshell gateway start
openshell provider create --name db --type generic --credential DATABASE_URL 2>/dev/null || true
# Optional: Socket.dev package scanning
openshell provider create --name socket --type generic --credential SOCKET_TOKEN 2>/dev/null || true
openshell sandbox create \
  --from ghcr.io/sandys/openeral/sandbox:just-bash \
  --provider db --provider claude --provider socket --auto-providers \
  -- /opt/openeral/setup.sh
```

## What happens after launch

- Claude Code starts with `HOME` pointing to an isolated workspace
- Without `DATABASE_URL`: local temp home, no persistence, no `pg` command
- With `DATABASE_URL`: files sync to PostgreSQL, `pg` command available, files survive across sessions
- With `SOCKET_TOKEN` (OpenShell): npm routes through Socket.dev with credential injection
- Credential injection: API keys stay as placeholders in the sandbox; the OpenShell proxy resolves them at egress
