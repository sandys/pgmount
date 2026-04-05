# CLAUDE.md

## Build & Test

```bash
cd openeral-js
pnpm install
pnpm check                    # typecheck + lint + unit tests

# Integration (requires PostgreSQL)
DATABASE_URL='...' node test-integration.mjs

# E2E (3-session persistence + Claude API)
DATABASE_URL='...' ANTHROPIC_API_KEY='...' node test-e2e-claude.mjs
```

## Project Structure

- `openeral-js/` — TypeScript package (just-bash + PostgreSQL virtual filesystem)
  - `src/pg-fs/` — PgFs: read-only IFileSystem backed by SQL queries
  - `src/workspace-fs/` — WorkspaceFs: read-write IFileSystem backed by workspace_files
  - `src/db/` — SQL queries, migrations, pool, types
  - `src/safety.ts` — command safety analysis via just-bash parse() AST
  - `src/shell.ts` — createOpeneralShell(), createToolHandler()
  - `src/index.ts` — public API
  - `lint.mjs` — 8 structural lint rules
- `sandboxes/openeral/` — OpenShell sandbox image (stock base, no FUSE)
  - `Dockerfile` — Node.js + openeral-js on stock OpenShell base
  - `openeral-bash.mjs` — daemon/client bridge for Claude Code's bash
  - `setup.sh` — entry point: migrate, seed, daemon, exec claude
  - `policy.yaml` — network policy with boundary secret injection
- `crates/` — original Rust implementation (reference, not used in sandbox)

## Conventions

- IFileSystem implementations are path-based (no inodes)
- `parsePath()` returns a `PgNode` discriminated union
- SQL queries use `quoteIdent()` for identifiers, `$N` params for values, `::text` casts
- PgFs throws EROFS on all write methods
- WorkspaceFs receives complete content per writeFile() — no write-back buffering
- Command safety: just-bash parse() AST walk with regex fallback
- `pg` command: SQL with parens or quotes must be double-quoted (`pg "SELECT count(*) ..."`)

## Hard Rules

- **Never fix forward from the middle.** Stop and restart the flow from scratch.
- **Never delete, move, or overwrite user files without explicit permission.**
- **If a file appears risky, stop and ask first.**

## Commit Style

Descriptive, imperative mood. Look at `git log --oneline` for examples.
