import { expect, test } from 'bun:test';
import { runMain } from '../../src/usecases/run-main';
import { stop } from '../../src/usecases/stop';
import { NODE, SECRET_KEY, WS, newWorld, startDeps, stopDeps, type World } from '../fakes';

const TOKEN = 'tok-123';
const serve = { name: 'svc', target: '127.0.0.1:8080', 'relay-token': TOKEN };
const access = { name: 'svc', 'node-id': NODE, listen: '127.0.0.1:15000', 'relay-token': TOKEN };

/** What the real binary writes into its config, and (worst case) leaks into its log. */
const KEY_LINE = `secret_key = "${SECRET_KEY}"`;
const dirtyLog = (extra = '') => `starting\n${KEY_LINE}\nrelay_token=${TOKEN}\n${extra}\n`;

function leaks(w: World): string[] {
  return w.out.filter((m) => m.includes(SECRET_KEY) || m.includes(TOKEN));
}

/** The fake workspace records what start renders; the child "writes" the key like the real binary. */
function child(w: World, log: string, exit: number | null = null) {
  w.onSpawn = (x) => {
    x.files.set(WS.configPath, `${x.files.get(WS.configPath) ?? ''}${KEY_LINE}\n`);
    x.files.set(WS.logPath, log);
    x.exit = exit;
  };
}

test('fixture is meaningful: the rendered config really holds the token and the key', async () => {
  const w = newWorld(serve);
  child(w, dirtyLog(), 2);
  await runMain('serve', startDeps(w));
  const cfg = w.files.get(WS.configPath)!;
  expect(cfg).toContain(TOKEN);
  expect(cfg).toContain(SECRET_KEY);
});

test('no secret in any output: start exit, bind error, timeout', async () => {
  const a = newWorld(serve);
  child(a, dirtyLog('config error'), 2);
  await runMain('serve', startDeps(a));
  expect(a.failed).toHaveLength(1);
  expect(a.failed[0]).toContain('config error');
  expect(leaks(a)).toEqual([]);
  expect(a.secrets).toEqual([TOKEN]);

  const b = newWorld(access);
  child(b, dirtyLog('ERROR failed to bind 127.0.0.1:15000: Address already in use'));
  await runMain('access', startDeps(b));
  expect(b.failed[0]).toContain('Address already in use');
  expect(leaks(b)).toEqual([]);

  const c = newWorld(serve);
  child(c, dirtyLog());
  await runMain('serve', startDeps(c));
  expect(c.failed[0]).toContain('not ready within 60 s');
  expect(leaks(c)).toEqual([]);
});

test('no secret in any output: stop with job failed or not', async () => {
  for (const failed of [false, true]) {
    const c = newWorld(serve);
    c.state = { workspace: WS.root, pid: '4242', role: 'serve' };
    c.status = { node_id: NODE, services: [] };
    c.files.set(WS.configPath, KEY_LINE);
    c.files.set(WS.logPath, dirtyLog('tail'));
    c.jobFailed = failed;
    await stop(stopDeps(c));
    if (failed) expect(c.out.join('\n')).toContain('tail');
    expect(leaks(c).filter((m) => m.includes(SECRET_KEY))).toEqual([]);
  }
});
