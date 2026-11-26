import { describe, expect, test } from 'bun:test';
import { stop } from '../../src/usecases/stop';
import { runMain } from '../../src/usecases/run-main';
import { NODE, WS, newWorld, startDeps, stopDeps } from '../fakes';

function saved() {
  const w = newWorld();
  w.state = { workspace: '/tmp/ws', pid: '4242', role: 'serve', 'main-ok': '1', 'process-identity': 'fake-identity' };
  w.status = { node_id: NODE, services: [] };
  w.files.set('/tmp/ws/tunnel.log', 'log line A\nlog line B\n');
  return w;
}

describe('stop', () => {
  test('cleans up only the tunnel saved by that invocation', async () => {
    const first = newWorld({ name: 'svc', target: '127.0.0.1:8080' });
    const second = newWorld({ name: 'svc', target: '127.0.0.1:8081' });
    first.status = { node_id: NODE, home_relay: 'https://relay.example./', services: [] };
    const secondWorkspace = { ...WS, root: '/tmp/ws2', stateDir: '/tmp/ws2/state', homeDir: '/tmp/ws2/home', configPath: '/tmp/ws2/config.toml', logPath: '/tmp/ws2/tunnel.log' };
    const secondStart = startDeps(second, {
      createWorkspace: async () => secondWorkspace,
      readStatus: async () => { throw new Error('status read failed after spawn'); },
    });
    const spawn = secondStart.launcher.spawn;
    secondStart.launcher.spawn = async (...args) => ({ ...await spawn(...args), pid: 4243 });
    await runMain('serve', startDeps(first));
    await runMain('serve', secondStart);
    expect(first.failed).toEqual([]);
    expect(second.failed).toHaveLength(1);
    expect(second.failed[0]).toContain('status read failed after spawn');
    expect(second.exit).toBeNull();
    expect(second.state['main-ok']).toBeUndefined();
    second.jobFailed = true;
    await stop(stopDeps(second));
    expect(second.stopCalls).toEqual([[4243, 5000, 'fake-identity']]);
    expect(second.removed).toEqual(['/tmp/ws2']);
    expect(first.stopCalls).toEqual([]);
    expect(first.removed).toEqual([]);
    await stop(stopDeps(first));
    expect(first.stopCalls).toEqual([[4242, 5000, 'fake-identity']]);
    expect(first.removed).toEqual(['/tmp/ws']);
  });

  test('cooperative cleanup after startup failure never uses PID fallback', async () => {
    const w = newWorld({ name: 'svc', target: '127.0.0.1:8080' });
    const deps = startDeps(w, {
      installer: { async install() { return { exePath: '/bin/serve', version: 'local' }; } },
      readStatus: async () => { throw new Error('startup failed'); },
    });
    await runMain('serve', deps);
    expect(w.failed[0]).toContain('startup failed');
    expect(w.spawned[0]!.env.IROH_TUNNEL_ACTION_SHUTDOWN_DIR).toBe(WS.root);
    expect(w.state['shutdown-mode']).toBe('action');
    await stop(stopDeps(w));
    expect(w.calls).toContain('launcher.stopAction:/tmp/ws:5000');
    expect(w.stopCalls).toEqual([]);
  });

  test('cooperative shutdown failure never falls back to a PID', async () => {
    const w = saved();
    w.state['shutdown-mode'] = 'action';
    w.stopResult = 'still-alive';
    await stop(stopDeps(w));
    expect(w.failed[0]).toContain('did not confirm action-scoped shutdown');
    expect(w.stopCalls).toEqual([]);
    expect(w.removed).toEqual([]);
  });

  test('no saved workspace: success, no calls', async () => {
    const w = newWorld();
    await stop(stopDeps(w));
    expect(w.calls).toEqual([]);
  });

  test('healthy job: status in a group, stops, no log, removes workspace', async () => {
    const w = saved();
    await stop(stopDeps(w));
    expect(w.calls.filter((c) => c.startsWith('group:'))).toEqual(['group:iroh-tunnel final status']);
    expect(w.out.join('\n')).toContain(NODE);
    expect(w.out.join('\n')).not.toContain('log line A');
    expect(w.stopCalls).toEqual([[4242, 5000, 'fake-identity']]);
    expect(w.removed).toEqual(['/tmp/ws']);
    expect(w.failed).toEqual([]);
  });

  test('failed job prints the log', async () => {
    const w = saved();
    w.jobFailed = true;
    await stop(stopDeps(w));
    expect(w.out.join('\n')).toContain('log line B');
    expect(w.removed).toEqual(['/tmp/ws']);
    expect(w.failed).toEqual([]);
  });

  test('still-alive fails the step', async () => {
    const w = saved();
    w.stopResult = 'still-alive';
    await stop(stopDeps(w));
    expect(w.failed).toHaveLength(1);
    expect(w.failed[0]).toContain('4242');
    expect(w.removed).toEqual(['/tmp/ws']);
  });

  test.each(['identity-mismatch', 'identity-unavailable'] as const)('%s fails cleanup without hiding the unsafe process', async (result) => {
    const w = saved();
    const deps = stopDeps(w);
    deps.launcher.stop = async () => result;
    await stop(deps);
    expect(w.failed[0]).toContain('Cannot safely stop');
    expect(w.removed).toEqual(['/tmp/ws']);
  });

  test('main failed before spawn (no pid): still cleans up', async () => {
    const w = newWorld();
    w.state = { workspace: '/tmp/ws' };
    w.jobFailed = true;
    await stop(stopDeps(w));
    expect(w.stopCalls).toEqual([]);
    expect(w.removed).toEqual(['/tmp/ws']);
  });

  describe('cleanup on dependency errors', () => {
    const cases: [string, (w: ReturnType<typeof saved>) => Parameters<typeof stopDeps>[1]][] = [
      ['readStatus', () => ({ readStatus: async () => { throw new Error('status boom'); } })],
      ['launcher.stop', (w) => {
        const d = stopDeps(w);
        return { launcher: { ...d.launcher, stop: async () => { throw new Error('stop boom'); } } };
      }],
      ['readLog', () => ({ readLog: async () => { throw new Error('log boom'); } })],
    ];
    for (const [name, mk] of cases) {
      test(`${name} throws: still kills, removes workspace, fails the step`, async () => {
        const w = saved();
        w.jobFailed = true;
        const over = mk(w);
        await stop(stopDeps(w, over));
        expect(w.removed).toEqual(['/tmp/ws']);
        expect(w.failed).toHaveLength(1);
        expect(w.failed[0]).toContain(`${name === 'launcher.stop' ? 'stop' : name === 'readStatus' ? 'status' : 'log'} boom`);
        if (name !== 'launcher.stop') expect(w.stopCalls).toEqual([[4242, 5000, 'fake-identity']]);
      });
    }

    test('removeWorkspace throws: step fails', async () => {
      const w = saved();
      await stop(stopDeps(w, { removeWorkspace: async () => { throw new Error('rm boom'); } }));
      expect(w.failed[0]).toContain('rm boom');
    });
  });
});
