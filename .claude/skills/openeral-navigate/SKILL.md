---
name: openeral-navigate
description: Use /db for read-only database context and /home/agent for persistent workspace via just-bash
---

# OpenEral Navigate

The agent's bash tool runs through just-bash with two PostgreSQL-backed mounts:

- `/home/agent` — read-write persistent workspace
- `/db` — read-only database view

## Database Reads

```bash
ls /db                                          # list schemas
ls /db/public                                   # list tables
cat /db/public/users/.info/columns.json         # column metadata
cat /db/public/users/.info/schema.sql           # CREATE TABLE DDL
cat /db/public/users/.info/count                # exact row count
cat /db/public/users/.info/primary_key          # PK columns
cat /db/public/users/page_1/1/row.json          # single row as JSON
cat /db/public/users/page_1/1/name              # single column value
grep -r "Alice" /db/public/users/               # search rows
ls /db/public/users/.filter/status/active/      # filtered rows
ls /db/public/users/.order/created_at/desc/     # sorted rows
ls /db/public/users/.indexes/                   # index metadata
ls /db/public/users/.export/data.json/          # paginated export
pg SELECT count(*) FROM public.users            # direct SQL
```

Prefer `.filter/` for targeted lookups — cheapest path for database inspection.

## Workspace

Any files the agent should keep must go under `/home/agent`:

```bash
mkdir -p /home/agent/work
echo "notes" > /home/agent/work/todo.txt
cat /home/agent/work/todo.txt
```

Persistence is automatic — every write goes to PostgreSQL immediately.

## What Not To Do

- Do not write to `/db` (read-only, throws EROFS)
- Do not assume `/tmp` is durable (ephemeral in-memory)
- Do not scan huge tables when `.filter/` or `.info/count` answers the question
