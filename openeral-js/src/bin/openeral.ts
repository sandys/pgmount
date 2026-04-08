#!/usr/bin/env node

import { main } from '../cli.js';

void main().catch((err: Error) => {
  process.stderr.write(`\x1b[31mopeneral: ${err.message}\x1b[0m\n`);
  process.exit(1);
});
