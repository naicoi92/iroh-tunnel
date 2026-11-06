import { describe, expect, test } from 'bun:test';
import { archiveName, resolveTarget, UnsupportedPlatformError } from '../../src/adapters/target';

describe('resolveTarget', () => {
  test('linux x64 glibc', () => {
    expect(resolveTarget('linux', 'x64', false)).toEqual({ os: 'linux', arch: 'amd64', musl: false, archiveExt: '.tar.gz', checksumFile: 'checksums.txt' });
  });
  test('linux x64 musl', () => {
    expect(resolveTarget('linux', 'x64', true).musl).toBe(true);
  });
  test('linux arm64', () => {
    expect(resolveTarget('linux', 'arm64', false)).toMatchObject({ os: 'linux', arch: 'arm64', musl: false });
  });
  test('darwin arm64 uses checksums-darwin.txt', () => {
    expect(resolveTarget('darwin', 'arm64')).toEqual({ os: 'darwin', arch: 'arm64', musl: false, archiveExt: '.tar.gz', checksumFile: 'checksums-darwin.txt' });
  });
  test('windows x64 and arm64', () => {
    expect(resolveTarget('win32', 'x64')).toEqual({ os: 'windows', arch: 'amd64', musl: false, archiveExt: '.zip', checksumFile: 'checksums.txt' });
    expect(resolveTarget('win32', 'arm64')).toMatchObject({ os: 'windows', arch: 'arm64', archiveExt: '.zip' });
  });
  test('musl flag is ignored outside linux', () => {
    expect(resolveTarget('darwin', 'arm64', true).musl).toBe(false);
  });
  for (const [p, a] of [['darwin', 'x64'], ['linux', 'riscv64']] as const) {
    test(`${p}/${a} is unsupported`, () => {
      try {
        resolveTarget(p, a);
        throw new Error('no throw');
      } catch (e) {
        expect(e).toBeInstanceOf(UnsupportedPlatformError);
        const err = e as UnsupportedPlatformError;
        expect(err.supported).toEqual(['linux-amd64', 'linux-arm64', 'darwin-arm64', 'windows-amd64', 'windows-arm64']);
        expect(err.message).toContain('linux-amd64, linux-arm64, darwin-arm64, windows-amd64, windows-arm64');
      }
    });
  }
});

test('archiveName', () => {
  expect(archiveName('iroh-tunnel-serve', '0.6.0', { os: 'linux', arch: 'amd64', musl: true, archiveExt: '.tar.gz', checksumFile: 'checksums.txt' })).toBe('iroh-tunnel-serve-0.6.0-linux-amd64-musl.tar.gz');
  expect(archiveName('iroh-tunnel-access', '0.5.0', resolveTarget('win32', 'x64'))).toBe('iroh-tunnel-access-0.5.0-windows-amd64.zip');
});
