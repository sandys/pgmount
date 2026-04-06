import { describe, it, expect } from 'vitest';
import { analyzeCommandSync } from './safety.js';

describe('analyzeCommandSync (regex fallback — no just-bash parser in test env)', () => {
  it('allows cat', () => {
    const result = analyzeCommandSync('cat /db/public/users/.info/count');
    expect(result.safe).toBe(true);
  });

  it('allows ls', () => {
    const result = analyzeCommandSync('ls /db');
    expect(result.safe).toBe(true);
  });

  it('allows grep', () => {
    const result = analyzeCommandSync('grep -r "Alice" /db/public/users/');
    expect(result.safe).toBe(true);
  });

  it('allows git status', () => {
    const result = analyzeCommandSync('git status');
    expect(result.safe).toBe(true);
  });

  it('allows git log', () => {
    const result = analyzeCommandSync('git log --oneline -5');
    expect(result.safe).toBe(true);
  });

  it('blocks rm', () => {
    const result = analyzeCommandSync('rm -rf /');
    expect(result.safe).toBe(false);
  });

  it('blocks sudo', () => {
    const result = analyzeCommandSync('sudo apt install something');
    expect(result.safe).toBe(false);
  });

  it('blocks write redirection', () => {
    const result = analyzeCommandSync('echo x > /db/test');
    expect(result.safe).toBe(false);
  });

  it('blocks append redirection', () => {
    const result = analyzeCommandSync('echo x >> /home/agent/file');
    expect(result.safe).toBe(false);
  });

  it('blocks git push', () => {
    const result = analyzeCommandSync('git push origin main');
    expect(result.safe).toBe(false);
  });

  it('blocks npm install', () => {
    const result = analyzeCommandSync('npm install express');
    expect(result.safe).toBe(false);
  });

  it('allows jq', () => {
    const result = analyzeCommandSync('jq .name /db/public/users/page_1/1/row.json');
    expect(result.safe).toBe(true);
  });

  it('allows find', () => {
    const result = analyzeCommandSync('find /db -name "*.json"');
    expect(result.safe).toBe(true);
  });

  it('allows awk', () => {
    const result = analyzeCommandSync("awk '{print $1}' /db/public/users/.info/count");
    expect(result.safe).toBe(true);
  });
});
