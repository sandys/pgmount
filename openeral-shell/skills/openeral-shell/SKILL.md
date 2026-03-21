---
name: openeral-shell
description: Persistent shell — your files, memory, plans, and sessions survive restarts
---

# Your Environment

You are running inside a persistent shell. Everything you write persists across restarts.

## Key Paths

- **`/home/agent/`** — your persistent home directory (read-write, backed by PostgreSQL)
- **`/db/`** — PostgreSQL database browsable as files (read-only)

## Your Persistent State

Your `~/.claude/` directory persists automatically:

```
~/.claude/memory/     remembered context across conversations
~/.claude/plans/      implementation plans
~/.claude/sessions/   conversation transcripts
~/.claude/tasks/      task tracking
~/.claude/todos/      todo lists
~/.claude/skills/     agent skills (including this one)
```

Write files anywhere under `/home/agent/` — they all persist:

```bash
mkdir -p ~/projects/myapp
echo "data" > ~/projects/myapp/notes.txt
# Still there after restart
```

## Browse the Database

The PostgreSQL database is mounted at `/db/`. No SQL needed — use standard file tools.

```bash
# Discover structure
ls /db/                                        # schemas
ls /db/public/                                 # tables
cat /db/public/users/.info/columns.json        # column definitions
cat /db/public/users/.info/count               # row count
cat /db/public/users/.info/primary_key         # primary key

# Read rows
cat /db/public/users/page_1/1/row.json         # full row as JSON
cat /db/public/users/page_1/1/email            # single column value

# Filter (FAST — targeted SQL query, much faster than scanning pages)
cat /db/public/users/.filter/id/42/42/row.json
ls /db/public/users/.filter/active/true/

# Sort
ls /db/public/users/.order/created_at/desc/page_1/

# Export
cat /db/public/users/.export/data.csv/page_1.csv
cat /db/public/users/.export/data.json/page_1.json
```

### Database Layout

```
/db/<schema>/<table>/
  .info/
    columns.json        column names, types, nullability
    schema.sql          CREATE TABLE DDL
    count               row count
    primary_key         PK column(s)
  .export/
    data.json/          paginated JSON (page_1.json, page_2.json, ...)
    data.csv/           paginated CSV
  .filter/<col>/<val>/  rows where column = value
  .order/<col>/asc/     rows sorted ascending
  .indexes/<name>       index definitions
  page_1/
    <pk_value>/         row directory
      <column>          column value as text
      row.json          full row as JSON
      row.csv           full row as CSV
      row.yaml          full row as YAML
```

## Rules

1. **`/db/` is read-only.** Writes return "Read-only file system".
2. **Always check `.info/count` first.** Tables with millions of rows have thousands of pages — don't `ls` them all.
3. **Use `.filter/` for lookups.** It runs a targeted SQL query. Much faster than scanning pages.
4. **Pages hold up to 1000 rows each.**
5. **Composite primary keys** appear as `col1=val1,col2=val2` directory names.
6. **NULL values** appear as empty files.
7. **Everything under `/home/agent/` persists.** Write freely — it's backed by PostgreSQL.
