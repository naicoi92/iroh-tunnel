import { expect, test } from 'bun:test';
import { parseAccess, parseServe } from '../../src/domain/inputs';
import { isReady } from '../../src/usecases/readiness';

const N = 'a'.repeat(64);
const serve = parseServe((k) => ({ name: 'svc', target: '127.0.0.1:80' })[k] ?? '');
const access = (protocol: string) =>
  parseAccess((k) => ({ name: 'svc', 'node-id': N, listen: '127.0.0.1:15000', protocol })[k] ?? '');
const bound = 'INFO listening for local clients listen_addr=127.0.0.1:15000\n';
const boundUdp = 'INFO listening for local udp clients listen_addr=127.0.0.1:15000\n';

test('serve is ready iff node_id is present; home_relay null does not block', () => {
  expect(isReady(serve, undefined, '')).toEqual({ ready: false });
  const s = { node_id: N, services: [], home_relay: null } as never;
  expect(isReady(serve, s, '')).toEqual({ ready: true });
});

test('access tcp: not bound -> not ready', () => {
  expect(isReady(access('tcp'), { node_id: N, services: [] }, '')).toEqual({ ready: false });
});

test('access tcp: bound with no transports -> needsProbe', () => {
  const st = { node_id: N, services: [{ name: 'svc', transports: [] }] };
  expect(isReady(access('tcp'), st, bound)).toEqual({ ready: false, needsProbe: true });
  expect(isReady(access('tcp'), undefined, bound)).toEqual({ ready: false, needsProbe: true });
});

test('access tcp: bound with transports -> ready', () => {
  const st = { node_id: N, services: [{ name: 'svc', transports: [{}] }] };
  expect(isReady(access('tcp'), st, bound)).toEqual({ ready: true });
});

test('access tcp: transports of another service do not count', () => {
  const st = { node_id: N, services: [{ name: 'other', transports: [{}] }] };
  expect(isReady(access('tcp'), st, bound)).toEqual({ ready: false, needsProbe: true });
});

test('access udp: bound -> ready without status', () => {
  expect(isReady(access('udp'), undefined, boundUdp)).toEqual({ ready: true });
  expect(isReady(access('udp'), undefined, '')).toEqual({ ready: false });
});

test('bind error -> failed with the address', () => {
  const log = 'ERROR failed to bind 127.0.0.1:15000: Address already in use\n';
  const r = isReady(access('tcp'), undefined, log);
  expect(r).toHaveProperty('failed');
  const f = (r as { failed: string }).failed;
  expect(f).toContain('127.0.0.1:15000');
  expect(f).toContain('Address already in use');
});
