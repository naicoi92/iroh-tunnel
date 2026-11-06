import { describe, expect, test } from 'bun:test';
import { InputError } from '../../src/domain/errors';
import { parseAccess, parseServe } from '../../src/domain/inputs';

const HEX = 'a'.repeat(64);
const reader = (m: Record<string, string>, seen: string[] = []) => (n: string) => {
  seen.push(n);
  return (m[n] ?? '').trim();
};
const access = (extra: Record<string, string> = {}) => ({ name: 'db', 'node-id': HEX, listen: '127.0.0.1:15432', ...extra });
const serve = (extra: Record<string, string> = {}) => ({ name: 'db', target: 'postgres:5432', ...extra });
const errOf = (f: () => unknown): InputError => {
  try {
    f();
  } catch (e) {
    expect(e).toBeInstanceOf(InputError);
    return e as InputError;
  }
  throw new Error('did not throw');
};

describe('parseAccess', () => {
  test('minimal', () => {
    expect(parseAccess(reader(access()))).toEqual({
      role: 'access', name: 'db', nodeId: HEX,
      listen: { host: '127.0.0.1', port: 15432, family: 'ipv4' },
      protocol: 'tcp', relayUrls: [], relayToken: undefined, version: { kind: 'default' },
    });
  });
  test('missing node-id', () => {
    expect(errOf(() => parseAccess(reader({ name: 'db', listen: '127.0.0.1:1' }))).input).toBe('node-id');
  });
  test('missing name / listen', () => {
    expect(errOf(() => parseAccess(reader({ 'node-id': HEX, listen: '127.0.0.1:1' }))).input).toBe('name');
    expect(errOf(() => parseAccess(reader({ name: 'db', 'node-id': HEX }))).input).toBe('listen');
  });
  test('listen rejects hostname', () => {
    expect(errOf(() => parseAccess(reader(access({ listen: 'localhost:1' })))).input).toBe('listen');
  });
  test('node-id rules', () => {
    expect(errOf(() => parseAccess(reader(access({ 'node-id': '0123456789' })))).input).toBe('node-id');
    expect(parseAccess(reader(access({ 'node-id': 'a2'.repeat(26) }))).nodeId).toBe('a2'.repeat(26));
    expect(parseAccess(reader(access({ 'node-id': 'A'.repeat(64) }))).nodeId).toBe('A'.repeat(64));
  });
  test('never reads target', () => {
    const seen: string[] = [];
    parseAccess(reader(access({ target: 'x:1' }), seen));
    expect(seen).not.toContain('target');
  });
});

describe('parseServe', () => {
  test('minimal', () => {
    const r = parseServe(reader(serve()));
    expect(r.target).toEqual({ host: 'postgres', port: 5432, family: 'name' });
    expect(r.protocol).toBe('tcp');
  });
  test('missing target', () => {
    expect(errOf(() => parseServe(reader({ name: 'db' }))).input).toBe('target');
  });
  test('never reads node-id/listen', () => {
    const seen: string[] = [];
    parseServe(reader(serve({ 'node-id': HEX, listen: '127.0.0.1:1' }), seen));
    expect(seen).not.toContain('node-id');
    expect(seen).not.toContain('listen');
  });
});

describe('shared', () => {
  test('name rules', () => {
    const e = errOf(() => parseServe(reader(serve({ name: 'My_DB' }))));
    expect(e.input).toBe('name');
    expect(e.message).toContain('a-z, 0-9, -');
    expect(errOf(() => parseServe(reader(serve({ name: 'a'.repeat(64) })))).input).toBe('name');
    expect(parseServe(reader(serve({ name: 'a'.repeat(63) }))).name).toHaveLength(63);
  });
  test('protocol', () => {
    const e = errOf(() => parseServe(reader(serve({ protocol: 'sctp' }))));
    expect(e.input).toBe('protocol');
    expect(e.message).toContain('tcp');
    expect(e.message).toContain('udp');
    expect(parseServe(reader(serve({ protocol: 'udp' }))).protocol).toBe('udp');
  });
  test('relay-url', () => {
    for (const u of ['relay.example.com', 'http://relay.example.com']) {
      const e = errOf(() => parseServe(reader(serve({ 'relay-url': u }))));
      expect(e.input).toBe('relay-url');
      expect(e.message).toContain('https://');
    }
    expect(parseServe(reader(serve({ 'relay-url': 'https://a.example\n\n  https://b.example ' }))).relayUrls).toEqual([
      'https://a.example',
      'https://b.example',
    ]);
  });
  test('relay-token', () => {
    expect(parseServe(reader(serve({ 'relay-token': 'abc\n' }))).relayToken).toBe('abc');
    const e = errOf(() => parseServe(reader(serve({ 'relay-token': 'sec\nret' }))));
    expect(e.input).toBe('relay-token');
    expect(e.message).not.toContain('sec');
    expect(e.message).not.toContain('ret');
  });
  test('version', () => {
    expect(errOf(() => parseServe(reader(serve({ version: '0.6' })))).input).toBe('version');
    expect(parseServe(reader(serve({ version: 'v0.6.0' }))).version).toEqual({ kind: 'exact', version: '0.6.0' });
  });
});
