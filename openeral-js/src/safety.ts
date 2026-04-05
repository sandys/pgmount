/**
 * Command safety analysis using just-bash's parse() function.
 * Follows the pi-coding-agent pattern: AST walk, wrapper resolution,
 * redirect detection, subcommand awareness, graceful regex fallback.
 */

let parseBash: ((input: string) => any) | null = null;
let loadAttempted = false;

async function ensureParserLoaded(): Promise<void> {
  if (loadAttempted) return;
  loadAttempted = true;
  try {
    const mod = await import('just-bash');
    if (typeof mod?.parse === 'function') {
      parseBash = mod.parse;
    }
  } catch {
    parseBash = null;
  }
}

export interface BashInvocation {
  commandNameRaw: string;
  commandName: string;
  effectiveCommandName: string;
  effectiveArgs: string[];
  hasWriteRedirection: boolean;
}

export interface AnalysisResult {
  safe: boolean;
  reason?: string;
  invocations: BashInvocation[];
  parseError?: string;
}

const WRITE_REDIRECTION_OPERATORS = new Set(['>', '>>', '>|', '<>', '&>', '&>>', '>&']);
const WRAPPER_COMMANDS = new Set(['command', 'builtin', 'exec', 'nohup']);

function commandBaseName(value: string): string {
  const normalized = value.replace(/\\+/g, '/');
  const idx = normalized.lastIndexOf('/');
  const base = idx >= 0 ? normalized.slice(idx + 1) : normalized;
  return base.toLowerCase();
}

function partToText(part: any): string {
  if (!part || typeof part !== 'object') return '';
  switch (part.type) {
    case 'Literal':
    case 'SingleQuoted':
    case 'Escaped':
      return typeof part.value === 'string' ? part.value : '';
    case 'DoubleQuoted':
      return Array.isArray(part.parts) ? part.parts.map(partToText).join('') : '';
    case 'Glob':
      return typeof part.pattern === 'string' ? part.pattern : '';
    case 'TildeExpansion':
      return typeof part.user === 'string' && part.user.length > 0 ? `~${part.user}` : '~';
    case 'ParameterExpansion':
      return typeof part.parameter === 'string' && part.parameter.length > 0
        ? '${' + part.parameter + '}'
        : '${}';
    case 'CommandSubstitution':
      return '$(...)';
    case 'ProcessSubstitution':
      return part.direction === 'output' ? '>(...) ' : '<(...)';
    case 'ArithmeticExpansion':
      return '$((...))';
    default:
      return '';
  }
}

function wordToText(word: any): string {
  if (!word || typeof word !== 'object' || !Array.isArray(word.parts)) return '';
  return word.parts.map(partToText).join('');
}

function resolveEffectiveCommand(
  commandNameRaw: string,
  args: string[],
): { effectiveCommandName: string; effectiveArgs: string[] } {
  const primary = commandNameRaw.trim();
  const primaryBase = commandBaseName(primary);

  if (WRAPPER_COMMANDS.has(primaryBase)) {
    const next = args[0] ?? '';
    return { effectiveCommandName: commandBaseName(next), effectiveArgs: args.slice(1) };
  }

  if (primaryBase === 'env') {
    let idx = 0;
    while (idx < args.length) {
      const token = args[idx] ?? '';
      if (token === '--') { idx += 1; break; }
      if (token.startsWith('-') || /^[A-Za-z_][A-Za-z0-9_]*=.*/.test(token)) { idx += 1; continue; }
      break;
    }
    const next = args[idx] ?? '';
    return { effectiveCommandName: commandBaseName(next), effectiveArgs: args.slice(idx + 1) };
  }

  if (primaryBase === 'sudo') {
    let idx = 0;
    while (idx < args.length) {
      const token = args[idx] ?? '';
      if (token === '--') { idx += 1; break; }
      if (token.startsWith('-')) { idx += 1; continue; }
      break;
    }
    const next = args[idx] ?? '';
    return { effectiveCommandName: commandBaseName(next), effectiveArgs: args.slice(idx + 1) };
  }

  return { effectiveCommandName: primaryBase, effectiveArgs: args };
}

function collectNestedScripts(word: any, collect: (script: any) => void): void {
  if (!word || typeof word !== 'object' || !Array.isArray(word.parts)) return;
  for (const part of word.parts) {
    if (!part || typeof part !== 'object') continue;
    if (part.type === 'DoubleQuoted') {
      collectNestedScripts(part, collect);
      continue;
    }
    if ((part.type === 'CommandSubstitution' || part.type === 'ProcessSubstitution') && part.body) {
      collect(part.body);
    }
  }
}

const READ_ONLY_COMMANDS = new Set([
  'cat', 'head', 'tail', 'less', 'more', 'grep', 'find', 'ls', 'pwd', 'echo', 'printf',
  'wc', 'sort', 'uniq', 'diff', 'file', 'stat', 'du', 'df', 'tree', 'which', 'whereis',
  'type', 'env', 'printenv', 'uname', 'whoami', 'id', 'date', 'cal', 'uptime', 'ps',
  'top', 'htop', 'free', 'jq', 'awk', 'rg', 'fd', 'bat', 'exa', 'curl', 'seq',
  'basename', 'dirname', 'realpath', 'readlink', 'test', 'true', 'false', 'expr',
]);

const BLOCKED_COMMANDS = new Set([
  'rm', 'rmdir', 'mv', 'cp', 'mkdir', 'touch', 'chmod', 'chown', 'chgrp', 'ln',
  'tee', 'truncate', 'dd', 'shred', 'sudo', 'su', 'kill', 'pkill', 'killall',
  'reboot', 'shutdown', 'systemctl', 'service', 'vim', 'vi', 'nano', 'emacs',
  'code', 'subl', 'apt', 'apt-get', 'brew', 'pip',
]);

const ALLOWED_GIT_SUBCOMMANDS = new Set([
  'status', 'log', 'diff', 'show', 'branch', 'remote', 'config', 'ls-files', 'ls-tree', 'ls-remote',
]);

const ALLOWED_NPM_SUBCOMMANDS = new Set(['list', 'ls', 'view', 'info', 'search', 'outdated', 'audit']);

function isInvocationReadOnly(inv: BashInvocation): boolean {
  const cmd = inv.effectiveCommandName || inv.commandName;
  const args = inv.effectiveArgs;
  if (!cmd) return true;
  if (BLOCKED_COMMANDS.has(cmd)) return false;

  if (cmd === 'git') {
    const sub = (args[0] ?? '').toLowerCase();
    if (!sub) return true;
    if (sub === 'config') return args[1] === '--get';
    return ALLOWED_GIT_SUBCOMMANDS.has(sub) || sub.startsWith('ls-');
  }
  if (cmd === 'npm' || cmd === 'pnpm' || cmd === 'yarn') {
    const sub = (args[0] ?? '').toLowerCase();
    return !sub || ALLOWED_NPM_SUBCOMMANDS.has(sub);
  }
  if (cmd === 'sed') return args.includes('-n');
  if (cmd === 'node' || cmd === 'python' || cmd === 'python3') {
    return args.length > 0 && args.every(a => a === '--version');
  }

  return READ_ONLY_COMMANDS.has(cmd);
}

function analyzeBashScript(command: string): { parseError?: string; invocations: BashInvocation[] } {
  if (!parseBash) {
    return { parseError: 'just-bash parse unavailable', invocations: [] };
  }

  try {
    const ast = parseBash(command);
    const invocations: BashInvocation[] = [];

    const visitScript = (script: any) => {
      if (!script || typeof script !== 'object' || !Array.isArray(script.statements)) return;
      for (const statement of script.statements) {
        if (!statement || typeof statement !== 'object' || !Array.isArray(statement.pipelines)) continue;
        for (const pipeline of statement.pipelines) {
          if (!pipeline || typeof pipeline !== 'object' || !Array.isArray(pipeline.commands)) continue;
          for (const commandNode of pipeline.commands) {
            if (!commandNode || typeof commandNode !== 'object') continue;

            if (commandNode.type === 'SimpleCommand') {
              const commandNameRaw = wordToText(commandNode.name).trim();
              const commandName = commandBaseName(commandNameRaw);
              const args = Array.isArray(commandNode.args)
                ? commandNode.args.map((arg: any) => wordToText(arg)).filter(Boolean)
                : [];
              const redirections = Array.isArray(commandNode.redirections)
                ? commandNode.redirections.map((r: any) => (typeof r?.operator === 'string' ? r.operator : ''))
                : [];
              const effective = resolveEffectiveCommand(commandNameRaw, args);

              invocations.push({
                commandNameRaw,
                commandName,
                effectiveCommandName: effective.effectiveCommandName,
                effectiveArgs: effective.effectiveArgs,
                hasWriteRedirection: redirections.some((op: string) => WRITE_REDIRECTION_OPERATORS.has(op)),
              });

              if (commandNode.name) collectNestedScripts(commandNode.name, visitScript);
              if (Array.isArray(commandNode.args)) {
                for (const arg of commandNode.args) collectNestedScripts(arg, visitScript);
              }
              continue;
            }

            // Recurse into compound commands
            if (Array.isArray(commandNode.body)) visitScript({ statements: commandNode.body });
            if (Array.isArray(commandNode.condition)) visitScript({ statements: commandNode.condition });
            if (Array.isArray(commandNode.clauses)) {
              for (const clause of commandNode.clauses) {
                if (Array.isArray(clause?.condition)) visitScript({ statements: clause.condition });
                if (Array.isArray(clause?.body)) visitScript({ statements: clause.body });
              }
            }
            if (Array.isArray(commandNode.elseBody)) visitScript({ statements: commandNode.elseBody });
            if (Array.isArray(commandNode.items)) {
              for (const item of commandNode.items) {
                if (Array.isArray(item?.body)) visitScript({ statements: item.body });
              }
            }
          }
        }
      }
    };

    visitScript(ast);
    return { invocations };
  } catch (error: any) {
    return { parseError: error?.message ?? String(error), invocations: [] };
  }
}

// Regex fallback patterns (used when AST parsing fails)
const DESTRUCTIVE_PATTERNS = [
  /\brm\b/i, /\brmdir\b/i, /\bmv\b/i, /\bmkdir\b/i, /\btouch\b/i,
  /\bchmod\b/i, /\bchown\b/i, /\btee\b/i, /\btruncate\b/i, /\bdd\b/i,
  /[^<]>(?![>&])/, />>/, /\bsudo\b/i, /\bkill\b/i,
  /\bgit\s+(add|commit|push|pull|merge|rebase|reset)/i,
  /\bnpm\s+(install|uninstall|update)/i,
  /\bpip\s+(install|uninstall)/i,
];

const SAFE_PATTERNS = [
  /^\s*cat\b/, /^\s*head\b/, /^\s*tail\b/, /^\s*grep\b/, /^\s*find\b/,
  /^\s*ls\b/, /^\s*pwd\b/, /^\s*echo\b/, /^\s*wc\b/, /^\s*sort\b/,
  /^\s*uniq\b/, /^\s*diff\b/, /^\s*stat\b/, /^\s*tree\b/, /^\s*jq\b/,
  /^\s*awk\b/, /^\s*rg\b/, /^\s*git\s+(status|log|diff|show|branch)/i,
];

/**
 * Analyze a bash command for safety. Returns whether the command is safe
 * (read-only) and details about what commands it invokes.
 */
export async function analyzeCommand(command: string): Promise<AnalysisResult> {
  await ensureParserLoaded();
  const analysis = analyzeBashScript(command);

  if (!analysis.parseError) {
    // AST-based analysis
    if (analysis.invocations.some(i => i.hasWriteRedirection)) {
      return {
        safe: false,
        reason: 'Command contains write redirection',
        invocations: analysis.invocations,
      };
    }
    const unsafe = analysis.invocations.find(i => !isInvocationReadOnly(i));
    if (unsafe) {
      return {
        safe: false,
        reason: `Command "${unsafe.effectiveCommandName || unsafe.commandName}" is not read-only`,
        invocations: analysis.invocations,
      };
    }
    return { safe: true, invocations: analysis.invocations };
  }

  // Regex fallback
  if (DESTRUCTIVE_PATTERNS.some(p => p.test(command))) {
    return { safe: false, reason: 'Command matches destructive pattern', invocations: [], parseError: analysis.parseError };
  }
  if (SAFE_PATTERNS.some(p => p.test(command))) {
    return { safe: true, invocations: [], parseError: analysis.parseError };
  }

  // Unknown — default to unsafe
  return { safe: false, reason: 'Cannot determine safety', invocations: [], parseError: analysis.parseError };
}

/**
 * Synchronous version that uses pre-loaded parser.
 * Call ensureParserLoaded() first if you need AST analysis.
 */
export function analyzeCommandSync(command: string): AnalysisResult {
  const analysis = analyzeBashScript(command);

  if (!analysis.parseError) {
    if (analysis.invocations.some(i => i.hasWriteRedirection)) {
      return { safe: false, reason: 'Command contains write redirection', invocations: analysis.invocations };
    }
    const unsafe = analysis.invocations.find(i => !isInvocationReadOnly(i));
    if (unsafe) {
      return {
        safe: false,
        reason: `Command "${unsafe.effectiveCommandName || unsafe.commandName}" is not read-only`,
        invocations: analysis.invocations,
      };
    }
    return { safe: true, invocations: analysis.invocations };
  }

  if (DESTRUCTIVE_PATTERNS.some(p => p.test(command))) {
    return { safe: false, reason: 'Command matches destructive pattern', invocations: [], parseError: analysis.parseError };
  }
  if (SAFE_PATTERNS.some(p => p.test(command))) {
    return { safe: true, invocations: [], parseError: analysis.parseError };
  }
  return { safe: false, reason: 'Cannot determine safety', invocations: [], parseError: analysis.parseError };
}
