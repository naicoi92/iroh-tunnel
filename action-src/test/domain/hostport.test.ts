import { describe, expect, test } from 'bun:test';
import { parseHostPort } from '../../src/domain/hostport';

describe('parseHostPort', () => {
  test('ipv4', () => {
    expect(parseHostPort('127.0.0.1:65535', { allowName: false })).toEqual({ host: '127.0.0.1', port: 65535, family: 'ipv4' });
  });
  test('bracketed ipv6', () => {
    expect(parseHostPort('[::1]:5432', { allowName: false })).toEqual({ host: '::1', port: 5432, family: 'ipv6' });
  });
  test('name when allowed', () => {
    expect(parseHostPort('postgres:5432', { allowName: true })).toEqual({ host: 'postgres', port: 5432, family: 'name' });
  });
  test.each([
    ['127.0.0.1:0', false],
    ['127.0.0.1:70000', false],
    ['::1:5432', true],
    ['localhost:15432', false],
    [':80', true],
    ['host', true],
    ['127.0.0.1:80a', false],
  ])('rejects %s', (s, allowName) => {
    expect(() => parseHostPort(s, { allowName })).toThrow();
  });
});
