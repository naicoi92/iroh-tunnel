import { createHash } from 'node:crypto';
import { access, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { BUILD_VERSION, MIN_VERSION } from '../version';
import { compareVersions } from '../domain/version';
import type { VersionSpec } from '../domain/version';
import type { Role } from '../domain/inputs';
import { archiveName, type ReleaseTarget } from './target';

export type ToolCache = {
  find(tool: string, version: string, arch?: string): string;
  downloadTool(url: string): Promise<string>;
  extractTar(file: string, dest?: string, flags?: string | string[]): Promise<string>;
  extractZip(file: string, dest?: string): Promise<string>;
  cacheDir(src: string, tool: string, version: string, arch?: string): Promise<string>;
};

export interface Installer {
  install(role: Role, spec: VersionSpec): Promise<{ exePath: string; version: string }>;
}

export type InstallerOptions = {
  releaseRoot: string;
  target: ReleaseTarget;
  cache: ToolCache;
  fetch: typeof fetch;
  binDir?: string;
  /** Test seams; default to the build-time constants. */
  buildVersion?: string;
  minVersion?: string;
};

export function exeName(role: Role, target: ReleaseTarget): string {
  return `iroh-tunnel-${role}${target.os === 'windows' ? '.exe' : ''}`;
}

export function createInstaller(o: InstallerOptions): Installer {
  const buildVersion = o.buildVersion ?? BUILD_VERSION;
  const minVersion = o.minVersion ?? MIN_VERSION;
  const root = o.releaseRoot.replace(/\/+$/, '');

  async function resolveVersion(spec: VersionSpec): Promise<string> {
    if (spec.kind === 'default') return buildVersion;
    if (spec.kind === 'exact') return spec.version;
    const url = `${root}/latest`;
    const res = await o.fetch(url, { redirect: 'manual' });
    const loc = res.headers.get('location');
    const m = loc ? /\/tag\/v(.+?)\/?$/.exec(loc) : null;
    if (!m) throw new Error(`could not resolve the latest release from ${url} (HTTP ${res.status})`);
    return m[1]!;
  }

  async function fetchText(url: string, what: string, version: string): Promise<string> {
    let res: Response;
    try {
      res = await o.fetch(url);
    } catch (e) {
      throw new Error(`failed to download ${what} for iroh-tunnel ${version} from ${url}: ${(e as Error).message}`);
    }
    if (!res.ok) throw new Error(`failed to download ${what} for iroh-tunnel ${version} from ${url}: HTTP ${res.status}`);
    return res.text();
  }

  return {
    async install(role, spec) {
      const name = `iroh-tunnel-${role}`;
      const exe = exeName(role, o.target);
      if (o.binDir) {
        const p = join(o.binDir, exe);
        try {
          await access(p);
        } catch {
          throw new Error(`${exe} not found in IROH_TUNNEL_ACTION_BIN_DIR (${o.binDir})`);
        }
        return { exePath: p, version: 'local' };
      }

      if (spec.kind === 'exact' && compareVersions(spec.version, minVersion) < 0) {
        throw new Error(`version ${spec.version} is not supported; the minimum version is ${minVersion}`);
      }
      const version = await resolveVersion(spec);
      if (compareVersions(version, minVersion) < 0) {
        throw new Error(`version ${version} is not supported; the minimum version is ${minVersion}`);
      }

      const cacheArch = `${o.target.os}-${o.target.arch}${o.target.musl ? '-musl' : ''}`;
      const cached = o.cache.find(name, version, cacheArch);
      if (cached) return { exePath: join(cached, exe), version };

      const base = `${root}/download/v${version}`;
      const archive = archiveName(name, version, o.target);
      const sums = await fetchText(`${base}/${o.target.checksumFile}`, o.target.checksumFile, version);
      const line = sums
        .split(/\r?\n/)
        .map((l) => /^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$/.exec(l))
        .find((m) => m && m[2] === archive);
      if (!line) throw new Error(`${o.target.checksumFile} for iroh-tunnel ${version} has no entry for ${archive}`);
      const expected = line[1]!.toLowerCase();

      const archiveUrl = `${base}/${archive}`;
      let file: string;
      try {
        file = await o.cache.downloadTool(archiveUrl);
      } catch (e) {
        throw new Error(`failed to download ${archive} for iroh-tunnel ${version} from ${archiveUrl}: ${(e as Error).message}`);
      }
      const actual = createHash('sha256').update(await readFile(file)).digest('hex');
      if (actual !== expected) {
        throw new Error(`SHA-256 mismatch for ${archive}: expected ${expected}, got ${actual}`);
      }

      const dir = o.target.archiveExt === '.zip' ? await o.cache.extractZip(file) : await o.cache.extractTar(file);
      const cachedDir = await o.cache.cacheDir(dir, name, version, cacheArch);
      return { exePath: join(cachedDir, exe), version };
    },
  };
}
