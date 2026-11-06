import { describe, expect, test } from 'bun:test';
import { renderConfig } from '../../src/domain/config';
import type { AccessRequest, ServeRequest } from '../../src/domain/inputs';

const serve = (o: Partial<ServeRequest> = {}): ServeRequest => ({
  role: 'serve', name: 'db', target: { host: 'postgres', port: 5432, family: 'name' },
  protocol: 'tcp', relayUrls: [], version: { kind: 'default' }, ...o,
});
const access = (o: Partial<AccessRequest> = {}): AccessRequest => ({
  role: 'access', name: 'db', nodeId: 'a'.repeat(64), listen: { host: '127.0.0.1', port: 15432, family: 'ipv4' },
  protocol: 'tcp', relayUrls: [], version: { kind: 'default' }, ...o,
});

describe('renderConfig', () => {
  test('serve', () => {
    expect(renderConfig(serve())).toBe('[[services]]\nname = "db"\nprotocol = "tcp"\nhost = "postgres"\nport = 5432\n');
  });
  test('ipv6 unbracketed', () => {
    expect(renderConfig(serve({ target: { host: '::1', port: 1, family: 'ipv6' } }))).toContain('host = "::1"\n');
  });
  test('access has node_id', () => {
    const t = renderConfig(access());
    expect(t).toContain(`node_id = "${'a'.repeat(64)}"\n`);
    expect(t).toContain('host = "127.0.0.1"\nport = 15432\n');
    expect(t).not.toContain('[node]');
  });
  test('relays and token', () => {
    const t = renderConfig(serve({ relayUrls: ['https://a', 'https://b'], relayToken: 't' }));
    expect(t.startsWith('[node]\nrelay_urls = ["https://a", "https://b"]\nrelay_token = "t"\n\n[[services]]\n')).toBe(true);
    expect(t).not.toContain('secret_key');
  });
  test('escapes token', () => {
    expect(renderConfig(serve({ relayToken: 'a"b\\c' }))).toContain('relay_token = "a\\"b\\\\c"');
  });
});
