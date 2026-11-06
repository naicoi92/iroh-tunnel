export type ReleaseTarget = {
  os: 'linux' | 'darwin' | 'windows';
  arch: 'amd64' | 'arm64';
  musl: boolean;
  archiveExt: '.tar.gz' | '.zip';
  checksumFile: 'checksums.txt' | 'checksums-darwin.txt';
};

const SUPPORTED = ['linux-amd64', 'linux-arm64', 'darwin-arm64', 'windows-amd64', 'windows-arm64'];

export class UnsupportedPlatformError extends Error {
  readonly supported: string[];
  constructor(message: string, supported: string[]) {
    super(message);
    this.name = 'UnsupportedPlatformError';
    this.supported = supported;
  }
}

export function isMusl(): boolean {
  if (process.platform !== 'linux') return false;
  const report = process.report?.getReport() as unknown as { header: { glibcVersionRuntime?: string } } | undefined;
  return report?.header.glibcVersionRuntime === undefined;
}

export function resolveTarget(platform: string, arch: string, isMusl = false): ReleaseTarget {
  const os = platform === 'win32' ? 'windows' : platform === 'linux' || platform === 'darwin' ? platform : undefined;
  const a = arch === 'x64' ? 'amd64' : arch === 'arm64' ? 'arm64' : undefined;
  const key = os && a ? `${os}-${a}` : undefined;
  if (!os || !a || !key || !SUPPORTED.includes(key)) {
    throw new UnsupportedPlatformError(
      `unsupported platform ${platform}/${arch}; supported targets: ${SUPPORTED.join(', ')}`,
      [...SUPPORTED],
    );
  }
  return {
    os,
    arch: a,
    musl: os === 'linux' && isMusl,
    archiveExt: os === 'windows' ? '.zip' : '.tar.gz',
    checksumFile: os === 'darwin' ? 'checksums-darwin.txt' : 'checksums.txt',
  };
}

export function archiveName(exe: string, version: string, t: ReleaseTarget): string {
  return `${exe}-${version}-${t.os}-${t.arch}${t.musl ? '-musl' : ''}${t.archiveExt}`;
}
