import { describe, expect, test } from 'bun:test';
import { compareVersions, parseVersion } from '../../src/domain/version';

describe('parseVersion', () => {
  test('default/latest', () => {
    expect(parseVersion('')).toEqual({ kind: 'default' });
    expect(parseVersion('latest')).toEqual({ kind: 'latest' });
  });
  test('exact', () => {
    expect(parseVersion('0.6.0')).toEqual({ kind: 'exact', version: '0.6.0' });
    expect(parseVersion('v0.6.0')).toEqual({ kind: 'exact', version: '0.6.0' });
    expect(parseVersion('v0.6.1-rc.1')).toEqual({ kind: 'exact', version: '0.6.1-rc.1' });
  });
  test.each(['0.6', 'v', 'latest2'])('rejects %s', (s) => {
    expect(() => parseVersion(s)).toThrow();
  });
});

describe('compareVersions', () => {
  test('orders', () => {
    expect(compareVersions('0.5.9', '0.6.0')).toBeLessThan(0);
    expect(compareVersions('0.10.0', '0.6.0')).toBeGreaterThan(0);
    expect(compareVersions('0.6.0', '0.6.0')).toBe(0);
  });
  test('pre-release lower than release', () => {
    expect(compareVersions('0.6.0-rc.1', '0.6.0')).toBeLessThan(0);
    expect(compareVersions('0.6.0', '0.6.0-rc.1')).toBeGreaterThan(0);
    expect(compareVersions('0.6.0-rc.2', '0.6.0-rc.10')).toBeLessThan(0);
  });
});
