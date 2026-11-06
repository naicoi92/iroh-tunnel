import { describe, expect, test } from 'bun:test';
import { runMain } from '../../src/usecases/run-main';
import { NODE, WS, newWorld, startDeps, type World } from '../fakes';

const serveIn = { name: 'svc', target: '127.0.0.1:8080' };
const accessIn = { name: 'svc', 'node-id': NODE, listen: '127.0.0.1:15000' };
const bound = 'INFO listening for local clients listen_addr=127.0.0.1:15000\n';

function serveReady(w: World) {
  w.status = { node_id: NODE, services: [] };
}

describe('happy paths', () => {
  test('serve: sets node-id and main-ok, state saved in order', async () => {
    const w = newWorld(serveIn);
    w.onSleep = (x, n) => n === 2 && serveReady(x);
    await runMain('serve', startDeps(w));
    expect(w.failed).toEqual([]);
    expect(w.outputs).toEqual({ 'node-id': NODE });
    expect(w.state['main-ok']).toBe('1');
    expect(w.state['workspace']).toBe(WS.root);
    expect(w.state['pid']).toBe('4242');
    expect(w.state['role']).toBe('serve');
    const c = w.calls;
    expect(c.indexOf('setSecret')).toBeLessThan(c.indexOf('createWorkspace'));
    expect(c.indexOf('save:workspace')).toBeLessThan(c.indexOf('install'));
    expect(c.indexOf('install')).toBeLessThan(c.indexOf('spawn'));
    expect(c.indexOf('spawn')).toBeLessThan(c.indexOf('save:pid'));
    expect(c.indexOf('save:role')).toBeLessThan(c.indexOf('output:node-id'));
    const s = w.spawned[0]!;
    expect(s.exe).toBe('/bin/iroh-tunnel-serve');
    expect(s.args).toEqual(['run', '-c', WS.configPath]);
    expect(s.env).toEqual({ IROH_TUNNEL_STATE_DIR: WS.stateDir, NO_COLOR: '1', HOME: WS.homeDir });
    expect(w.secrets).toEqual([]);
  });

  test('windows spawn env has no HOME', async () => {
    const w = newWorld(serveIn);
    serveReady(w);
    await runMain('serve', startDeps(w, { platform: 'win32' }));
    expect(w.spawned[0]!.env).toEqual({ IROH_TUNNEL_STATE_DIR: WS.stateDir, NO_COLOR: '1' });
  });

  test('relay-token is masked first', async () => {
    const w = newWorld({ ...serveIn, 'relay-token': 'tok-123' });
    serveReady(w);
    await runMain('serve', startDeps(w));
    expect(w.calls[0]).toBe('setSecret');
    expect(w.secrets).toEqual(['tok-123']);
  });

  test('access tcp: probes exactly once, then waits for transports', async () => {
    const w = newWorld(accessIn);
    w.onSpawn = (x) => x.files.set(WS.logPath, bound);
    w.onSleep = (x, n) => {
      x.status = { node_id: NODE, services: [{ name: 'svc', transports: n >= 4 ? [{}] : [] }] };
    };
    await runMain('access', startDeps(w));
    expect(w.failed).toEqual([]);
    expect(w.probes).toBe(1);
    expect(w.outputs['node-id']).toBe(NODE);
    expect(w.state['main-ok']).toBe('1');
  });

  test('access udp: ready on bind, logs peer connection not checked', async () => {
    const w = newWorld({ ...accessIn, protocol: 'udp' });
    w.onSpawn = (x) => x.files.set(WS.logPath, 'listening for local udp clients listen_addr=127.0.0.1:15000\n');
    w.status = { node_id: NODE, services: [] };
    await runMain('access', startDeps(w));
    expect(w.failed).toEqual([]);
    expect(w.probes).toBe(0);
    expect(w.out.some((m) => m.includes('peer connection not checked'))).toBe(true);
    expect(w.outputs['node-id']).toBe(NODE);
  });
});

describe('failures', () => {
  test('invalid input: names the input, nothing else used', async () => {
    const w = newWorld({ ...serveIn, name: 'Bad Name' });
    await runMain('serve', startDeps(w));
    expect(w.failed).toHaveLength(1);
    expect(w.failed[0]).toContain('name');
    expect(w.calls).toEqual(['setFailed']);
  });

  test('exit code 2 fails immediately with class and log tail', async () => {
    const w = newWorld(serveIn);
    const lines = Array.from({ length: 60 }, (_, i) => `line ${i}`).join('\n');
    w.onSpawn = (x) => {
      x.exit = 2;
      x.files.set(WS.logPath, lines);
    };
    await runMain('serve', startDeps(w));
    expect(w.failed).toHaveLength(1);
    expect(w.failed[0]).toContain('exit code 2 (config)');
    expect(w.failed[0]).toContain('line 59');
    expect(w.failed[0]).toContain('line 20');
    expect(w.failed[0]).not.toContain('line 19\n');
    expect(w.polls).toBe(1); // one status read for the failure message, no readiness polling
    expect(w.outputs).toEqual({});
    expect(w.state['pid']).toBe('4242');
    expect(w.state['main-ok']).toBeUndefined();
  });

  test('exit code 6 explains the conflict (linux)', async () => {
    const w = newWorld(serveIn);
    w.onSpawn = (x) => (x.exit = 6);
    await runMain('serve', startDeps(w));
    const m = w.failed[0]!;
    expect(m).toContain('already running for this runner user');
    expect(m).toContain('system service');
    expect(m).toContain('another step of this job');
    expect(m).not.toContain('one Serve and one Access step per job on Windows');
  });

  test('exit failure includes the last status, or says there is none', async () => {
    const w = newWorld(serveIn);
    w.status = { node_id: NODE, services: [] };
    w.onSpawn = (x) => (x.exit = 4);
    await runMain('serve', startDeps(w));
    expect(w.failed[0]).toContain('Last status:');
    expect(w.failed[0]).toContain(NODE);
    const w2 = newWorld(serveIn);
    w2.onSpawn = (x) => (x.exit = 4);
    await runMain('serve', startDeps(w2));
    expect(w2.failed[0]).toContain('no status file yet');
  });

  test('exit code 6 on windows adds the limit', async () => {
    const w = newWorld(serveIn);
    w.onSpawn = (x) => (x.exit = 6);
    await runMain('serve', startDeps(w, { platform: 'win32' }));
    expect(w.failed[0]).toContain('one Serve and one Access step per job on Windows');
  });

  test('start fails fast on bind error', async () => {
    const w = newWorld(accessIn);
    w.onSpawn = (x) =>
      x.files.set(WS.logPath, 'ERROR failed to bind 127.0.0.1:15000: Address already in use (os error 98)\n');
    await runMain('access', startDeps(w));
    expect(w.failed).toHaveLength(1);
    expect(w.failed[0]).toContain('127.0.0.1:15000');
    expect(w.failed[0]).toContain('Address already in use');
    expect(w.failed[0]).toContain('input `listen` (127.0.0.1:15000): cannot bind');
    expect(w.failed[0]).toContain('Last status:');
    expect(w.failed[0]).toContain('no status file yet');
    expect(w.polls).toBe(1);
    expect(w.outputs).toEqual({});
    expect(w.state['pid']).toBe('4242');
  });

  test('timeout after 120 polls: last status and log tail', async () => {
    const w = newWorld(accessIn);
    w.onSpawn = (x) => x.files.set(WS.logPath, bound + 'still waiting\n');
    w.status = { node_id: NODE, services: [{ name: 'svc', transports: [] }] };
    await runMain('access', startDeps(w));
    expect(w.polls).toBe(120);
    expect(w.probes).toBe(1);
    const m = w.failed[0]!;
    expect(m).toContain('not ready within 60 s');
    expect(m).toContain(NODE);
    expect(m).toContain('still waiting');
    expect(w.outputs).toEqual({});
    expect(w.state['main-ok']).toBeUndefined();
    expect(w.state['pid']).toBe('4242');
  });

  test('timeout is a real 60 s deadline, not only a poll count', async () => {
    const w = newWorld(serveIn);
    let now = 0;
    await runMain(
      'serve',
      startDeps(w, {
        clock: {
          now: () => now,
          sleep: async (ms) => {
            now += ms * 4;
          },
        },
      }),
    );
    expect(w.polls).toBe(30);
    expect(w.failed[0]).toContain('not ready within 60 s');
  });

  test('child dies mid-poll', async () => {
    const w = newWorld(serveIn);
    w.onSleep = (x, n) => n === 3 && (x.exit = 1);
    await runMain('serve', startDeps(w));
    expect(w.failed[0]).toContain('exit code 1 (general)');
  });

  test('installer failure is reported and workspace state exists', async () => {
    const w = newWorld(serveIn);
    const d = startDeps(w, {
      installer: {
        async install() {
          throw new Error('download exploded');
        },
      },
    });
    await runMain('serve', d);
    expect(w.failed[0]).toContain('download exploded');
    expect(w.state['workspace']).toBe(WS.root);
    expect(w.state['pid']).toBeUndefined();
  });

  test('spawn failure is reported', async () => {
    const w = newWorld(serveIn);
    const d = startDeps(w);
    d.launcher.spawn = async () => {
      throw new Error('failed to start x: ENOENT');
    };
    await runMain('serve', d);
    expect(w.failed[0]).toContain('ENOENT');
  });

  test('error file written when env set', async () => {
    const w = newWorld(serveIn);
    w.onSpawn = (x) => (x.exit = 6);
    await runMain('serve', startDeps(w, { env: { IROH_TUNNEL_ACTION_ERROR_FILE: '/tmp/err.txt' }, platform: 'win32' }));
    expect(w.files.get('/tmp/err.txt')).toBe(w.failed[0]!);
    expect(w.files.get('/tmp/err.txt')).toContain('one Serve and one Access step per job on Windows');
  });

  test('error file also written for input errors; absent when env unset', async () => {
    const w = newWorld({ ...serveIn, name: '' });
    await runMain('serve', startDeps(w, { env: { IROH_TUNNEL_ACTION_ERROR_FILE: '/tmp/e' } }));
    expect(w.files.get('/tmp/e')).toContain('name');
    const w2 = newWorld({ ...serveIn, name: '' });
    await runMain('serve', startDeps(w2));
    expect([...w2.files.keys()]).toEqual([]);
  });
});
