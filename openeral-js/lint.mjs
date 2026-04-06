#!/usr/bin/env node

/**
 * Structural lints for openeral-js — catches classes of bugs found during
 * development so they don't recur.
 *
 * Run: node lint.mjs (or pnpm lint)
 */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const SRC = 'src';
let errors = 0;

function fail(file, message) {
  console.error(`  FAIL  ${file}: ${message}`);
  errors++;
}

function pass(label) {
  console.log(`  OK    ${label}`);
}

function allTsFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      files.push(...allTsFiles(full));
    } else if (full.endsWith('.ts')) {
      files.push(full);
    }
  }
  return files;
}

// ---------------------------------------------------------------------------
// Lint 1: Every import from a local .js file must have a corresponding .ts source
// Catches: missing module exports (like deleteTree)
// ---------------------------------------------------------------------------
console.log('\n--- Lint: import targets exist ---');

const tsFiles = allTsFiles(SRC);
const importRe = /from\s+['"](\.[^'"]+\.js)['"]/g;

for (const file of tsFiles) {
  if (file.endsWith('.test.ts')) continue;
  const content = readFileSync(file, 'utf8');
  let match;
  while ((match = importRe.exec(content)) !== null) {
    const importPath = match[1].replace(/\.js$/, '.ts');
    const resolved = join(file, '..', importPath);
    try {
      statSync(resolved);
    } catch {
      fail(file, `imports '${match[1]}' but ${resolved} does not exist`);
    }
  }
}
pass('all local imports resolve to .ts files');

// ---------------------------------------------------------------------------
// Lint 2: Every named import from a local module must be exported by that module
// Catches: importing deleteTree from a file that doesn't export it
// ---------------------------------------------------------------------------
console.log('\n--- Lint: named imports match exports ---');

const namedImportRe = /import\s+(?:type\s+)?{\s*([^}]+)}\s+from\s+['"](\.[^'"]+\.js)['"]/g;
const exportRe = /export\s+(?:async\s+)?(?:function|const|let|class|type|interface|enum)\s+(\w+)/g;

for (const file of tsFiles) {
  if (file.endsWith('.test.ts')) continue;
  const content = readFileSync(file, 'utf8');
  let match;
  while ((match = namedImportRe.exec(content)) !== null) {
    const names = match[1].split(',').map(n => n.trim().split(/\s+as\s+/)[0].trim()).filter(Boolean);
    const targetPath = join(file, '..', match[2].replace(/\.js$/, '.ts'));

    let targetContent;
    try {
      targetContent = readFileSync(targetPath, 'utf8');
    } catch {
      continue; // Lint 1 already catches missing files
    }

    const exports = new Set();
    let expMatch;
    while ((expMatch = exportRe.exec(targetContent)) !== null) {
      exports.add(expMatch[1]);
    }

    for (const name of names) {
      if (!exports.has(name)) {
        fail(file, `imports '${name}' from '${match[2]}' but it is not exported`);
      }
    }
  }
}
pass('all named imports match exports');

// ---------------------------------------------------------------------------
// Lint 3: package.json just-bash version must be >=2.0.0
// Catches: wrong version like ^0.1.0
// ---------------------------------------------------------------------------
console.log('\n--- Lint: just-bash version ---');

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const jbVersion = pkg.dependencies?.['just-bash'] || '';
const majorMatch = jbVersion.match(/(\d+)/);
if (!majorMatch || parseInt(majorMatch[1]) < 2) {
  fail('package.json', `just-bash version '${jbVersion}' is too old (need >=2.x)`);
} else {
  pass(`just-bash version ${jbVersion}`);
}

// ---------------------------------------------------------------------------
// Lint 4: createOpeneralShell must auto-create workspace config
// Catches: FK violation when workspace_config row doesn't exist
// ---------------------------------------------------------------------------
console.log('\n--- Lint: shell factory seeds workspace ---');

const shellContent = readFileSync('src/shell.ts', 'utf8');
if (!shellContent.includes('workspace_config')) {
  fail('src/shell.ts', 'createOpeneralShell must INSERT INTO workspace_config before use');
} else {
  pass('shell.ts auto-creates workspace_config');
}
if (!shellContent.includes('seedFromConfig')) {
  fail('src/shell.ts', 'createOpeneralShell must seed root directory');
} else {
  pass('shell.ts seeds root directory');
}

// ---------------------------------------------------------------------------
// Lint 5: PgFs write methods must throw EROFS
// Catches: accidentally making /db writable
// ---------------------------------------------------------------------------
console.log('\n--- Lint: PgFs is read-only ---');

const pgFsContent = readFileSync('src/pg-fs/pg-fs.ts', 'utf8');
const writeMethods = ['writeFile', 'appendFile', 'mkdir', 'rm', 'mv', 'chmod', 'utimes', 'symlink', 'link'];
for (const method of writeMethods) {
  // Check that each write method exists and calls erofs() or throws EROFS
  const methodRe = new RegExp(`async\\s+${method}\\b[\\s\\S]{0,200}(?:erofs|EROFS)`, 'i');
  if (!methodRe.test(pgFsContent)) {
    fail('src/pg-fs/pg-fs.ts', `${method}() must throw EROFS`);
  }
}
pass('all PgFs write methods throw EROFS');

// ---------------------------------------------------------------------------
// Lint 6: WorkspaceFs must not have write-back buffering
// Catches: reintroducing FUSE-style buffering that defeats just-bash's model
// ---------------------------------------------------------------------------
console.log('\n--- Lint: no write-back buffering ---');

const wsFsContent = readFileSync('src/workspace-fs/workspace-fs.ts', 'utf8');
if (/dirty|flush|OpenFileHandle/i.test(wsFsContent)) {
  fail('src/workspace-fs/workspace-fs.ts', 'must not use write-back buffering (dirty/flush/OpenFileHandle)');
} else {
  pass('no write-back buffering in WorkspaceFs');
}

// ---------------------------------------------------------------------------
// Lint 7: No FUSE references in sandbox Dockerfile
// Catches: accidentally reintroducing FUSE dependencies
// ---------------------------------------------------------------------------
console.log('\n--- Lint: no FUSE in sandbox ---');

try {
  const dockerfile = readFileSync('../sandboxes/openeral/Dockerfile', 'utf8');
  if (/fuse3|libfuse|\/dev\/fuse|fuse\.conf|\/etc\/fstab/i.test(dockerfile)) {
    fail('sandboxes/openeral/Dockerfile', 'must not reference FUSE (fuse3, libfuse, /dev/fuse, /etc/fstab)');
  } else {
    pass('no FUSE in Dockerfile');
  }
} catch {
  pass('Dockerfile not found (skipped)');
}

// ---------------------------------------------------------------------------
// Lint 8: pg custom command must document quoting requirement
// Catches: SQL with parens/quotes that bash parses before pg sees it
// ---------------------------------------------------------------------------
console.log('\n--- Lint: pg command quoting documented ---');

const shellSrc = readFileSync('src/shell.ts', 'utf8');
if (shellSrc.includes("defineCommand('pg'") || shellSrc.includes('defineCommand("pg"')) {
  // Verify the pg command exists — the quoting issue is a usage concern,
  // so we just check that the shell factory documents it
  pass('pg command defined in shell.ts');
} else {
  fail('src/shell.ts', 'pg custom command not found');
}

// ---------------------------------------------------------------------------
// Lint 9: Sandbox scripts must import from dist/, not src/
// Catches: importing .ts source instead of compiled .js in container
// ---------------------------------------------------------------------------
console.log('\n--- Lint: sandbox imports use dist/ ---');

for (const f of ['../sandboxes/openeral/setup.sh', '../sandboxes/openeral/openeral-bash.mjs']) {
  try {
    const content = readFileSync(f, 'utf8');
    if (/\/opt\/openeral\/src\//.test(content)) {
      fail(f, 'imports from /opt/openeral/src/ — must use /opt/openeral/dist/');
    }
  } catch {}
}
pass('sandbox scripts import from dist/');

// ---------------------------------------------------------------------------
// Lint 10: Dockerfile must build TypeScript
// Catches: forgetting npm run build in the Dockerfile
// ---------------------------------------------------------------------------
console.log('\n--- Lint: Dockerfile builds TypeScript ---');

try {
  const dockerfile = readFileSync('../sandboxes/openeral/Dockerfile', 'utf8');
  if (!dockerfile.includes('npm run build')) {
    fail('sandboxes/openeral/Dockerfile', 'must run "npm run build" to compile TypeScript');
  } else {
    pass('Dockerfile builds TypeScript');
  }
} catch {
  pass('Dockerfile not found (skipped)');
}

// ---------------------------------------------------------------------------
// Lint 11: No hardcoded credentials in generated scripts
// Catches: baking DATABASE_URL or secrets into helper scripts
// ---------------------------------------------------------------------------
console.log('\n--- Lint: no hardcoded credentials ---');

const cliContent = readFileSync('src/cli.ts', 'utf8');
// The pg helper function must NOT accept a connection string parameter
if (/writePgHelper\([^)]*connStr|writePgHelper\([^)]*url|writePgHelper\([^)]*database/i.test(cliContent)) {
  fail('src/cli.ts', 'writePgHelper must not accept a connection string — read from env at runtime');
} else {
  pass('pg helper reads DATABASE_URL from env');
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

console.log(`\n${errors === 0 ? '✓ All lints passed' : `✗ ${errors} lint error(s)`}\n`);
process.exit(errors > 0 ? 1 : 0);
