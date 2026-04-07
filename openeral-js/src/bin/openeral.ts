#!/usr/bin/env node

import { main } from '../cli.js';

void main().catch((err) => {
  process.stderr.write(`\x1b[31mopeneral: ${err.message}\x1b[0m\n`);
  process.exit(1);
});
