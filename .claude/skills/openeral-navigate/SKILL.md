---
name: openeral-navigate
description: Explore /db and manage files in /home/agent — PostgreSQL-backed virtual filesystem via just-bash
---

# OpenEral Navigate

Two mounts, both backed by PostgreSQL:

- `/db` — read-only database (schemas, tables, rows as files)
- `/home/agent` — read-write persistent workspace

## /db — Database Exploration

```bash
ls /db                                          # schemas
ls /db/public                                   # tables
cat /db/public/users/.info/columns.json         # column metadata
cat /db/public/users/.info/schema.sql           # CREATE TABLE DDL
cat /db/public/users/.info/count                # row count
cat /db/public/users/.info/primary_key          # PK columns
cat /db/public/users/page_1/1/row.json          # row as JSON
cat /db/public/users/page_1/1/name              # single column value
grep -r "Alice" /db/public/users/               # search rows
ls /db/public/users/.filter/status/active/      # filtered by column value
ls /db/public/users/.order/created_at/desc/     # sorted rows
ls /db/public/users/.indexes/                   # index metadata
ls /db/public/users/.export/data.json/          # paginated JSON export
pg "SELECT count(*) FROM public.users"          # direct SQL (quote complex queries)
```

Prefer `.filter/` and `.info/count` over scanning page trees.

## /home/agent — Workspace

Writes persist to PostgreSQL immediately:

```bash
mkdir -p /home/agent/work
echo "notes" > /home/agent/work/todo.txt
cat /home/agent/work/todo.txt
```

Persists across sessions (same workspaceId = same files).

## Rules

- `/db` is read-only — writes throw EROFS
- `/tmp` is ephemeral (in-memory, lost between sessions)
- `$HOME` is `/home/agent`
