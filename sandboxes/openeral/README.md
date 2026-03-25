# OpenEral Sandbox

Prebuilt OpenShell sandbox image for mounting PostgreSQL at `/db` and a persistent workspace at `/home/agent`.

## Quick Start

```bash
# 1. Install the stock OpenShell CLI
curl -fsSL https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh | sh

# 2. Infrastructure assumptions:
# - a gateway is already running with the custom cluster image
# - a generic provider named "$OPENERAL_DB_PROVIDER" already exists on that gateway
#   and points at the live PostgreSQL database
# - the host has ANTHROPIC_API_KEY available
#
# Compose stock openshell commands with shell variables.
# No wrapper scripts are required.
export OPENERAL_SANDBOX_IMAGE='<sandbox image ref>'
export OPENERAL_DB_PROVIDER=openeral-db
export OPENERAL_SANDBOX_NAME=openeral-demo
set -a
. ./.env
set +a

# 3. One command: create the sandbox, auto-create the Claude provider from host env,
# mount /db and /home/agent, and run Claude with persistent HOME.
openshell sandbox create \
  --name "$OPENERAL_SANDBOX_NAME" \
  --from "$OPENERAL_SANDBOX_IMAGE" \
  --provider "$OPENERAL_DB_PROVIDER" \
  --provider claude \
  --auto-providers \
  --no-tty -- env HOME=/home/agent claude
```

This is the preferred shippable flow: no wrapper scripts, no `sandbox upload`,
and no follow-up `sandbox connect` step just to start Claude.

## How It Works

- The custom cluster image deploys the FUSE device plugin and configures the gateway to request `github.com/fuse` for sandbox pods.
- The sandbox image declares two supervisor-managed FUSE mounts in `/etc/fstab`:
  - `env /db fuse.openeral ro,allow_other,noauto 0 0`
  - `env#workspace#${OPENSHELL_SANDBOX_ID} /home/agent fuse.openeral rw,allow_other,noauto 0 0`
- OpenShell side-loads `openshell-sandbox`, which reads `/etc/fstab`, resolves the mount sources, and launches `mount.fuse3` before the child process starts.
- `DATABASE_URL` from the provider is mapped to `OPENERAL_DATABASE_URL` for the FUSE daemon automatically.

The database itself and the OpenShell provider that exposes its `DATABASE_URL`
are infrastructure prerequisites. The user-facing sandbox launch stays a single
`openshell sandbox create ...` command.

## Persistence Model

- `/db` is a read-only PostgreSQL mount.
- `/home/agent` is a read-write openeral workspace keyed to `OPENSHELL_SANDBOX_ID`.
- Reconnecting to the same sandbox preserves `/home/agent`.
- Deleting and recreating a sandbox creates a new workspace because the sandbox id changes.

## Database Permissions

The database role used by the provider needs:

- `USAGE` on the application schemas it should browse
- `SELECT` on the application tables it should read
- either `CREATE` on the database for the first mount, or a pre-created `_openeral` schema
- write access to the `_openeral` schema for migrations and workspace storage

Example:

```sql
GRANT CONNECT ON DATABASE myapp TO agent_readonly;
GRANT USAGE ON SCHEMA public TO agent_readonly;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO agent_readonly;

GRANT ALL ON SCHEMA _openeral TO agent_readonly;
GRANT ALL ON ALL TABLES IN SCHEMA _openeral TO agent_readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA _openeral GRANT ALL ON TABLES TO agent_readonly;
```

## Developer Notes

Build the sandbox image from the repo root:

```bash
docker build -f sandboxes/openeral/Dockerfile -t openeral-sandbox:dev .
```

Important constraints:

- `openshell sandbox create --from sandboxes/openeral` is not the supported user flow. The image is designed to be published first, then referenced by image tag.
- The image `ENTRYPOINT` is not used under OpenShell. The supervisor overrides the container command and mounts FUSE from `/etc/fstab`.
