---
name: openeral-navigate
description: Navigate a PostgreSQL database mounted at /db and manage persistent workspace state
---

# OpenEral — Database at /db, Workspace at $HOME

You have two mounted filesystems:

- **`/db`** — read-only PostgreSQL database. Browse schemas, tables, rows as directories and files.
- **`$HOME`** (typically `/home/agent`) — read-write workspace backed by PostgreSQL. Your `~/.claude/` directory persists across restarts.

## Database: Quick Reference

```bash
# Discover structure
ls /db/                                        # list schemas
ls /db/public/                                 # list tables
cat /db/public/users/.info/columns.json        # column definitions
cat /db/public/users/.info/count               # row count
cat /db/public/users/.info/primary_key         # primary key

# Read rows
cat /db/public/users/page_1/1/row.json         # full row as JSON
cat /db/public/users/page_1/1/email            # single column

# Filter (FAST — runs a targeted query)
cat /db/public/users/.filter/id/42/42/row.json
ls /db/public/users/.filter/active/true/

# Sort
ls /db/public/users/.order/created_at/desc/page_1/

# Export
cat /db/public/users/.export/data.csv/page_1.csv
cat /db/public/users/.export/data.json/page_1.json
```

## Database: Filesystem Layout

```
/db/<schema>/<table>/
  .info/columns.json     — column names, types, nullability
  .info/schema.sql       — CREATE TABLE statement
  .info/count            — row count
  .info/primary_key      — PK column(s)
  .export/data.json/     — paginated JSON export (page_1.json, page_2.json, ...)
  .export/data.csv/      — paginated CSV export
  .filter/<col>/<val>/   — rows matching column=value
  .order/<col>/asc|desc/ — sorted rows
  .indexes/<name>        — index definitions
  page_1/                — first 1000 rows
    <pk>/                — row directory (named by primary key value)
      <column>           — column value as plain text
      row.json           — full row as JSON
      row.csv            — full row as CSV
      row.yaml           — full row as YAML
  page_2/                — next 1000 rows
```

## Workspace: Persistent State

Your `~/.claude/` directory is backed by PostgreSQL. Everything you write persists across container restarts.

```bash
# All of these persist automatically
ls ~/.claude/                      # memory, plans, sessions, tasks, todos
cat ~/.claude/settings.json        # your settings
echo "note" > ~/.claude/memory/context.md

# Create any files/directories — they all persist
mkdir -p ~/projects/myapp
echo "hello" > ~/projects/myapp/notes.txt
```

## Rules

1. **`/db` is read-only.** Any write attempt returns "Read-only file system".
2. **Always check `.info/count` before scanning a table.** Tables with millions of rows have thousands of pages — don't `ls` them all.
3. **Use `.filter/` for lookups.** It runs a targeted SQL query and is much faster than browsing pages.
4. **Pages contain up to 1000 rows** (configurable via `OPENERAL_PAGE_SIZE`).
5. **Composite primary keys** use the format `col1=val1,col2=val2` as directory names.
6. **NULL values** appear as empty files.
7. **`$HOME` is read-write.** Files persist in PostgreSQL — treat it like a normal home directory.
