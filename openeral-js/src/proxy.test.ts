import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '../..');
const policy = readFileSync(join(repoRoot, 'sandboxes/openeral/policy.yaml'), 'utf8');
const setup = readFileSync(join(repoRoot, 'sandboxes/openeral/setup.sh'), 'utf8');

describe('proxy policy (PROXY-PLAN compliance)', () => {
  it('has no secret_injection: fields (stock SecretResolver handles it)', () => {
    expect(policy).not.toMatch(/secret_injection:/);
  });

  it('has no egress_via: fields (not in stock OpenShell)', () => {
    expect(policy).not.toMatch(/egress_via:/);
  });

  it('has no egress_profile: fields (not in stock OpenShell)', () => {
    expect(policy).not.toMatch(/egress_profile:/);
  });

  it('Anthropic endpoint has protocol: rest + tls: terminate', () => {
    const anthropicSection = policy.slice(
      policy.indexOf('api.anthropic.com'),
      policy.indexOf('binaries:', policy.indexOf('api.anthropic.com')),
    );
    expect(anthropicSection).toContain('protocol: rest');
    expect(anthropicSection).toContain('tls: terminate');
  });

  it('Socket.dev endpoint exists with protocol: rest + tls: terminate', () => {
    expect(policy).toContain('registry.socket.dev');
    const socketSection = policy.slice(
      policy.indexOf('registry.socket.dev'),
      policy.indexOf('binaries:', policy.indexOf('registry.socket.dev')),
    );
    expect(socketSection).toContain('protocol: rest');
    expect(socketSection).toContain('tls: terminate');
  });
});

describe('setup.sh Socket.dev integration', () => {
  it('configures npm registry when SOCKET_TOKEN is present', () => {
    expect(setup).toContain('SOCKET_TOKEN');
    expect(setup).toContain('registry.socket.dev');
    expect(setup).toContain('npm config set registry');
    expect(setup).toContain('_authToken');
  });

  it('does not hardcode the SOCKET_TOKEN value', () => {
    // Must reference $SOCKET_TOKEN (env var), not a literal token
    expect(setup).toContain('"$SOCKET_TOKEN"');
    expect(setup).not.toMatch(/sock_[a-zA-Z0-9]/);
  });

  it('Socket.dev config is conditional (only when SOCKET_TOKEN is set)', () => {
    expect(setup).toMatch(/if \[ -n "\$\{SOCKET_TOKEN:-\}"/);
  });
});
