---
name: openeral-shell
description: Work inside the published openeral OpenShell sandbox with PostgreSQL at /db and a persistent workspace at /home/agent
---

# OpenEral in OpenShell

This skill assumes you are already inside the published `openeral` OpenShell sandbox image.

OpenShell is responsible for the mount setup:

- the custom cluster image deploys the FUSE device plugin and the gateway requests `github.com/fuse`
- the sandbox image declares two `fuse.openeral` entries in `/etc/fstab`
- the side-loaded `openshell-sandbox` supervisor mounts them before the child process starts
- the provider's `DATABASE_URL` is mapped to `OPENERAL_DATABASE_URL` for the FUSE daemon

Do not try to bootstrap mounts manually as the normal workflow. Do not rely on `openeral-start.sh`; that path is obsolete.

## Mounted Paths

- **`/db`** — read-only PostgreSQL filesystem
- **`/home/agent`** — read-write persistent workspace backed by PostgreSQL

Persistence is keyed to `OPENSHELL_SANDBOX_ID`:

- reconnecting to the same sandbox keeps the same `/home/agent`
- deleting and recreating a sandbox gives you a fresh `/home/agent`

Important: the persistent workspace is `/home/agent`, not necessarily `~`. Some shells and tools still start with `HOME=/sandbox`. If you need state to persist, write it under `/home/agent` explicitly or launch the tool with `HOME=/home/agent`.

## Claude Auth

The preferred product flow is:

- the gateway already has a generic provider pointing at the live PostgreSQL database
- the host has `ANTHROPIC_API_KEY`
- `openshell sandbox create --provider <db-provider> --provider claude --auto-providers -- ...` auto-creates the Claude provider from host env and starts Claude directly

Do not treat `sandbox upload` of local Claude auth files as the default workflow. That is a manual fallback path, not the primary OpenShell flow for this sandbox.

## Database Layout

```text
/db/<schema>/<table>/
  .info/
    columns.json
    schema.sql
    count
    primary_key
  .export/
    data.json/page_1.json
    data.csv/page_1.csv
    data.yaml/page_1.yaml
  .filter/<column>/<value>/<pk>/
    <column>
    row.json
    row.csv
    row.yaml
  .order/<column>/asc/<pk>/
  .order/<column>/desc/<pk>/
  .indexes/<index_name>
  page_1/<pk>/
    <column>
    row.json
    row.csv
    row.yaml
  page_2/
  ...
```

Current implementation detail:

- `page_N/` is paginated table browsing
- `.filter/<column>/<value>/` returns matching row directories directly
- `.order/<column>/asc|desc/` returns row directories directly
- `.filter` and `.order` currently expose only the first page-size batch of results, not a separate `page_1/` tree

## Recommended Workflow

**Confirm the environment first:**

```bash
[ -d /db ] && echo db-ok
[ -d /home/agent ] && echo workspace-ok
grep -E ' /db | /home/agent ' /proc/mounts
```

**Understand a table before scanning it:**

```bash
ls /db
ls /db/public
cat /db/public/users/.info/columns.json
cat /db/public/users/.info/count
cat /db/public/users/.info/schema.sql
```

**Read rows efficiently:**

```bash
# First page browse
cat /db/public/users/page_1/1/row.json
cat /db/public/users/page_1/1/email

# Targeted lookup: prefer this over scanning pages
cat /db/public/users/.filter/id/42/42/row.json
ls /db/public/users/.filter/email/alice@example.com/

# Sorted sample: current layout is direct row directories, not page_1/
ls /db/public/orders/.order/created_at/desc/
```

**Export larger datasets:**

```bash
cat /db/public/users/.export/data.json/page_1.json
cat /db/public/users/.export/data.csv/page_1.csv
cat /db/public/users/.export/data.yaml/page_1.yaml
```

**Keep all persistent state under `/home/agent`:**

```bash
mkdir -p /home/agent/projects/analysis
printf 'findings\n' > /home/agent/projects/analysis/notes.md

# For tools that store state under $HOME
HOME=/home/agent <tool>

# Claude Code with provider-backed API auth
HOME=/home/agent claude
```

## Operational Rules

1. **`/db` is read-only.** Any write attempt should be treated as a mistake.
2. **Check `.info/count` before broad scans.** Large tables can have many pages.
3. **Prefer `.filter/` for lookups.** It is the targeted path.
4. **Use `.order/` for ordered samples, not full-table exports.**
5. **Composite primary keys** appear as `col1=val1,col2=val2`.
6. **NULL values** appear as empty files.
7. **Persistent work belongs in `/home/agent`.** `/sandbox` is not the durable workspace.
8. **If `/db` or `/home/agent` is missing, treat it as an infrastructure issue.** Check `/proc/mounts`, then report the mount failure instead of trying ad hoc `mknod`, `mount`, or cgroup workarounds from inside the sandbox.
