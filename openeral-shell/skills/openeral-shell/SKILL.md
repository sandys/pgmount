---
name: openeral-shell
description: Persistent shell environment — database at /db, persistent home at $HOME
---

# Your Environment

You are running inside **openeral-shell** — a persistent shell environment.

## What You Have

- **`/db/`** — read-only PostgreSQL database browsable as files
- **`$HOME`** (`/home/agent`) — persistent home directory backed by PostgreSQL

Everything you write under `$HOME` survives container restarts.

## Browse the Database

```bash
ls /db/                                        # schemas
ls /db/public/                                 # tables
cat /db/public/users/.info/columns.json        # column definitions
cat /db/public/users/.info/count               # row count

# Read rows
cat /db/public/users/page_1/1/row.json         # full row as JSON
cat /db/public/users/page_1/1/email            # single column

# Filter (FAST — targeted SQL query)
cat /db/public/users/.filter/id/42/42/row.json
ls /db/public/users/.filter/active/true/

# Sort
ls /db/public/users/.order/created_at/desc/page_1/

# Export
cat /db/public/users/.export/data.csv/page_1.csv
```

### Database Layout

```
/db/<schema>/<table>/
  .info/columns.json      column definitions
  .info/schema.sql        CREATE TABLE DDL
  .info/count             row count
  .info/primary_key       PK column(s)
  .export/data.json/      paginated JSON export
  .export/data.csv/       paginated CSV export
  .filter/<col>/<val>/    rows matching column=value
  .order/<col>/asc|desc/  sorted rows
  .indexes/<name>         index definitions
  page_1/<pk>/            row directory
    <column>              column value as text
    row.json              full row as JSON
```

## Persistent Home

Your `~/.claude/` directory and everything else under `$HOME` persists in PostgreSQL.

```bash
ls ~/.claude/                        # memory, plans, sessions, tasks, todos
echo "note" > ~/.claude/memory/context.md
mkdir -p ~/projects/myapp
echo "data" > ~/projects/myapp/notes.txt
# All of this survives container restarts
```

## Rules

1. **`/db` is read-only.** Writes return "Read-only file system".
2. **Check `.info/count` before scanning a table.** Large tables have many pages.
3. **Use `.filter/` for lookups.** Much faster than browsing pages.
4. **Pages hold up to 1000 rows.**
5. **Composite PKs** use `col1=val1,col2=val2` as directory names.
6. **NULL values** appear as empty files.
7. **`$HOME` is read-write.** Treat it like a normal home directory.
