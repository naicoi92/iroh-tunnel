import { expect, test } from 'bun:test';
import { runMain } from '../../src/usecases/run-main';
import { stop } from '../../src/usecases/stop';
import { NODE, SECRET_KEY, WS, newWorld, startDeps, stopDeps, type World } from '../fakes';

const CONFIG = `secret_key = "${SECRET_KEY}"\n`;
const base = { name: 'svc', target: '127.0.0.1:8080', 'relay-token': 'tok-123' };

function leaks(w: World): string[] {
  // setSecret calls are not recorded in w.out by design
  return w.out.filter((m) => m.includes(SECRET_KEY) || m.includes('tok-123'));
}

function logWithKey(extra = ''): string {
  // A misbehaving child that printed config-adjacent text must not be echoed as a secret by us:
  // we only assert that *we* never print the config file.
  return `starting\n${extra}`;
}

test('no secret in any output: start failure, timeout and stop', async () => {
  // start failure
  const a = newWorld(base);
  a.files.set(WS.configPath, CONFIG);
  a.onSpawn = (x) => {
    x.exit = 2;
    x.files.set(WS.logPath, logWithKey('config error'));
  };
  await runMain('serve', startDeps(a));
  expect(a.failed).toHaveLength(1);
  expect(leaks(a)).toEqual([]);
  expect(a.secrets).toEqual(['tok-123']);

  // timeout
  const b = newWorld(base);
  b.files.set(WS.configPath, CONFIG);
  b.onSpawn = (x) => x.files.set(WS.logPath, logWithKey());
  await runMain('serve', startDeps(b));
  expect(b.failed[0]).toContain('not ready within 60 s');
  expect(leaks(b)).toEqual([]);

  // stop, healthy and failed job
  for (const failed of [false, true]) {
    const c = newWorld(base);
    c.state = { workspace: WS.root, pid: '4242', role: 'serve' };
    c.status = { node_id: NODE, services: [] };
    c.files.set(WS.configPath, CONFIG);
    c.files.set(WS.logPath, logWithKey());
    c.jobFailed = failed;
    await stop(stopDeps(c));
    expect(leaks(c)).toEqual([]);
  }
});
