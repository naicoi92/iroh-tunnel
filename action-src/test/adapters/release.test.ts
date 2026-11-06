import { afterAll, beforeAll, beforeEach, describe, expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInstaller, type ToolCache } from '../../src/adapters/release';
import { archiveName, resolveTarget } from '../../src/adapters/target';

const target = resolveTarget('linux', 'x64', false);
const BODY = 'archive-bytes';
const sha = (s: string) => createHash('sha256').update(s).digest('hex');
let server: ReturnType<typeof Bun.serve>;
let root: string;
let requests: string[] = [];
let latestTag = '0.7.0';
let badDigest = false;
let work: string;

beforeAll(async () => {
  work = await mkdtemp(join(tmpdir(), 'rel-test-'));
  server = Bun.serve({
    port: 0,
    fetch(req) {
      const u = new URL(req.url);
      requests.push(u.pathname);
      if (u.pathname === '/latest') return new Response(null, { status: 302, headers: { location: `/tag/v${latestTag}` } });
      const m = /^\/download\/v([^/]+)\/(.+)$/.exec(u.pathname);
      if (m && m[1] !== '9.9.9') {
        const [, ver, file] = m;
        if (file === 'checksums.txt') {
          const digest = badDigest ? '0'.repeat(64) : sha(BODY);
          return new Response(`${'f'.repeat(64)}  other-file.tar.gz\n${digest}  ${archiveName('iroh-tunnel-serve', ver!, target)}\n${digest}  ${archiveName('iroh-tunnel-serve', ver!, resolveTarget('linux', 'x64', true))}\n`);
        }
        return new Response(BODY);
      }
      return new Response('nope', { status: 404 });
    },
  });
  root = `http://127.0.0.1:${server.port}`;
});
afterAll(() => server.stop(true));

type Calls = { extract: number; cacheDir: number; finds: number };
function fakeCache(calls: Calls, store = new Map<string, string>()): ToolCache {
  return {
    find: (tool, version, arch) => {
      calls.finds++;
      return store.get(`${tool}/${version}/${arch}`) ?? '';
    },
    downloadTool: async (url) => {
      const r = await fetch(url);
      if (!r.ok) throw new Error(`Unexpected HTTP response: ${r.status}`);
      const p = join(work, `dl-${Math.random().toString(36).slice(2)}`);
      await writeFile(p, Buffer.from(await r.arrayBuffer()));
      return p;
    },
    extractTar: async () => {
      calls.extract++;
      const d = await mkdtemp(join(work, 'x-'));
      await writeFile(join(d, 'iroh-tunnel-serve'), 'bin');
      return d;
    },
    extractZip: async () => {
      calls.extract++;
      return mkdtemp(join(work, 'x-'));
    },
    cacheDir: async (src, tool, version, arch) => {
      calls.cacheDir++;
      store.set(`${tool}/${version}/${arch}`, src);
      return src;
    },
  };
}

let calls: Calls;
const mk = (extra: Partial<Parameters<typeof createInstaller>[0]> = {}, cache?: ToolCache) =>
  createInstaller({ releaseRoot: root, target, cache: cache ?? fakeCache(calls), fetch, buildVersion: '0.6.0', minVersion: '0.6.0', ...extra });

beforeEach(() => {
  requests = [];
  badDigest = false;
  latestTag = '0.7.0';
  calls = { extract: 0, cacheDir: 0, finds: 0 };
});

describe('installer', () => {
  test('default installs the build version', async () => {
    const r = await mk().install('serve', { kind: 'default' });
    expect(r.version).toBe('0.6.0');
    expect(r.exePath.endsWith('iroh-tunnel-serve')).toBe(true);
    expect(requests).toContain('/download/v0.6.0/iroh-tunnel-serve-0.6.0-linux-amd64.tar.gz');
    expect(requests).toContain('/download/v0.6.0/checksums.txt');
  });

  test('latest resolves through the releases/latest redirect', async () => {
    const r = await mk().install('serve', { kind: 'latest' });
    expect(r.version).toBe('0.7.0');
    expect(requests[0]).toBe('/latest');
  });

  test('exact below the minimum fails without HTTP', async () => {
    await expect(mk().install('serve', { kind: 'exact', version: '0.5.0' })).rejects.toThrow(/0\.6\.0/);
    expect(requests).toEqual([]);
  });

  test('404 names the version and URL', async () => {
    const p = mk().install('serve', { kind: 'exact', version: '9.9.9' });
    await expect(p).rejects.toThrow(/9\.9\.9/);
    await expect(mk().install('serve', { kind: 'exact', version: '9.9.9' })).rejects.toThrow(`${root}/download/v9.9.9/`);
  });

  test('digest mismatch names the archive and extracts nothing', async () => {
    badDigest = true;
    await expect(mk().install('serve', { kind: 'default' })).rejects.toThrow(/iroh-tunnel-serve-0\.6\.0-linux-amd64\.tar\.gz/);
    expect(calls.extract).toBe(0);
  });

  test('second install uses the cache with zero HTTP', async () => {
    const cache = fakeCache(calls);
    const inst = mk({}, cache);
    const a = await inst.install('serve', { kind: 'default' });
    requests = [];
    const b = await inst.install('serve', { kind: 'default' });
    expect(requests).toEqual([]);
    expect(b.exePath).toBe(a.exePath);
    expect(calls.cacheDir).toBe(1);
  });
});

describe('IROH_TUNNEL_ACTION_BIN_DIR', () => {
  test('returns the executable in binDir with zero HTTP', async () => {
    const dir = await mkdtemp(join(work, 'bin-'));
    const exe = process.platform === 'win32' ? 'iroh-tunnel-access.exe' : 'iroh-tunnel-access';
    await writeFile(join(dir, exe), 'x');
    const r = await mk({ binDir: dir }).install('access', { kind: 'default' });
    expect(r.exePath).toBe(join(dir, exe));
    expect(requests).toEqual([]);
  });

  test('missing executable is an error', async () => {
    const dir = await mkdtemp(join(work, 'empty-'));
    await mkdir(dir, { recursive: true });
    await expect(mk({ binDir: dir }).install('access', { kind: 'default' })).rejects.toThrow(/iroh-tunnel-access/);
  });
});

describe('cache keys', () => {
  test('musl and glibc builds of the same version and role use distinct keys', async () => {
    const store = new Map<string, string>();
    const gnu = resolveTarget('linux', 'x64', false);
    const musl = resolveTarget('linux', 'x64', true);
    const mkFor = (t: typeof gnu) => mk({ target: t }, fakeCache(calls, store));
    await mkFor(gnu).install('serve', { kind: 'default' });
    await mkFor(musl).install('serve', { kind: 'default' });
    expect(calls.cacheDir).toBe(2);
    expect(store.size).toBe(2);
  });
});
