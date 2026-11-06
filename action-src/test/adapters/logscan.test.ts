import { expect, test } from 'bun:test';
import { scanLog } from '../../src/adapters/logscan';

const l4 = { host: '127.0.0.1', port: 15432, family: 'ipv4' as const };

test('bound line', () => {
  const t = '2026-11-06T07:14:04.193225Z  INFO listening for local clients listen_addr=127.0.0.1:15432\n';
  expect(scanLog(t, l4)).toEqual({ bound: true });
});
test('udp bound line', () => {
  expect(scanLog('INFO listening for local udp clients listen_addr=127.0.0.1:15432', l4).bound).toBe(true);
});
test('different port is not bound', () => {
  expect(scanLog('INFO listening for local clients listen_addr=127.0.0.1:15433', l4).bound).toBe(false);
  expect(scanLog('INFO listening for local clients listen_addr=127.0.0.1:154321', l4).bound).toBe(false);
});
test('endpoint-ready line is not bound', () => {
  expect(scanLog('INFO access endpoint ready, listening for local clients', l4).bound).toBe(false);
});
test('bind error', () => {
  const addr = { host: '127.0.0.1', port: 15998, family: 'ipv4' as const };
  const r = scanLog('ERROR failed to bind 127.0.0.1:15998: Address already in use (os error 48)', addr);
  expect(r.bound).toBe(false);
  expect(r.bindError).toContain('Address already in use');
});
test('udp bind error', () => {
  const r = scanLog('ERROR failed to bind udp 127.0.0.1:15432: Address already in use', l4);
  expect(r.bindError).toContain('failed to bind udp');
});
test('iroh endpoint bind failure is not a bind error', () => {
  expect(scanLog('ERROR failed to bind iroh endpoint: boom', l4).bindError).toBeUndefined();
});
test('bind error for a different address is ignored', () => {
  expect(scanLog('ERROR failed to bind 127.0.0.1:15998: Address already in use', l4).bindError).toBeUndefined();
});
test('ipv6 is matched unbracketed as written in config', () => {
  const a = { host: '::1', port: 15999, family: 'ipv6' as const };
  expect(scanLog('INFO listening for local clients listen_addr=::1:15999', a).bound).toBe(true);
  expect(scanLog('ERROR failed to bind ::1:15999: in use', a).bindError).toBeDefined();
});
