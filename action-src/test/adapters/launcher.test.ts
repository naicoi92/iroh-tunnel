import { afterAll, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createLauncher } from '../../src/adapters/launcher';

let dir: string;
const ready = (async () => { dir = await mkdtemp(join(tmpdir(), 'launch-test-')); })();
afterAll(() => rm(dir, { recursive: true, force: true }));

async function until(f: () => Promise<boolean> | boolean, ms = 5000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (await f()) return; await Bun.sleep(25); }
  throw new Error('timeout');
}
const exe = process.execPath;
async function script(name: string, body: string) {
  await ready;
  const p = join(dir, name);
  await writeFile(p, body);
  return p;
}

test('writes stdout+stderr to the log and sets the child env', async () => {
  const s = await script('env.js', `console.log('out', process.env.IROH_TUNNEL_STATE_DIR, process.env.NO_COLOR, process.env.HOME); console.error('err-line');`);
  const log = join(dir, 'env.log');
  const l = createLauncher();
  const h = await l.spawn(exe, [s], { IROH_TUNNEL_STATE_DIR: '/s/state', NO_COLOR: '1', ...(process.platform === 'win32' ? {} : { HOME: '/s/home' }) }, log);
  expect(h.pid).toBeGreaterThan(0);
  await until(async () => (await readFile(log, 'utf8').catch(() => '')).includes('err-line'));
  const text = await readFile(log, 'utf8');
  expect(text).toContain('out /s/state 1');
  if (process.platform !== 'win32') expect(text).toContain('/s/home');
  await until(() => h.exitCode() === 0);
});

test.skipIf(process.platform === 'win32')('stop returns exited for a child that handles SIGTERM, and isAlive is false', async () => {
  const s = await script('term.js', `process.on('SIGTERM', () => process.exit(0)); console.log('up'); setInterval(() => {}, 1000);`);
  const log = join(dir, 'term.log');
  const l = createLauncher();
  const h = await l.spawn(exe, [s], {}, log);
  await until(async () => (await readFile(log, 'utf8').catch(() => '')).includes('up'));
  expect(l.isAlive(h.pid)).toBe(true);
  expect(await l.stop(h.pid, 200)).toBe('exited');
  expect(l.isAlive(h.pid)).toBe(false);
});

test.skipIf(process.platform === 'win32')('stop returns killed for a child ignoring SIGTERM', async () => {
  const s = await script('ign.js', `process.on('SIGTERM', () => {}); console.log('up'); setInterval(() => {}, 1000);`);
  const log = join(dir, 'ign.log');
  const l = createLauncher();
  const h = await l.spawn(exe, [s], {}, log);
  await until(async () => (await readFile(log, 'utf8').catch(() => '')).includes('up'));
  expect(await l.stop(h.pid, 200)).toBe('killed');
  expect(l.isAlive(h.pid)).toBe(false);
});

test('stop on an already-dead pid is exited', async () => {
  const s = await script('quick.js', `process.exit(0)`);
  const l = createLauncher();
  const h = await l.spawn(exe, [s], {}, join(dir, 'q.log'));
  await until(() => h.exitCode() === 0);
  expect(await l.stop(h.pid, 100)).toBe('exited');
});

test('spawning a nonexistent executable rejects with the cause and emits no unhandled error', async () => {
  await ready;
  const unhandled: unknown[] = [];
  const onErr = (e: unknown) => unhandled.push(e);
  process.on('uncaughtException', onErr);
  try {
    const l = createLauncher();
    await expect(l.spawn(join(dir, 'does-not-exist'), [], {}, join(dir, 'nx.log'))).rejects.toThrow(/ENOENT/);
    await Bun.sleep(100);
    expect(unhandled).toEqual([]);
  } finally {
    process.off('uncaughtException', onErr);
  }
});
