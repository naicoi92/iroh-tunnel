import { describe, expect, test } from 'bun:test';
import { stop } from '../../src/usecases/stop';
import { NODE, newWorld, stopDeps } from '../fakes';

function saved() {
  const w = newWorld();
  w.state = { workspace: '/tmp/ws', pid: '4242', role: 'serve', 'main-ok': '1' };
  w.status = { node_id: NODE, services: [] };
  w.files.set('/tmp/ws/tunnel.log', 'log line A\nlog line B\n');
  return w;
}

describe('stop', () => {
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
    expect(w.stopCalls).toEqual([[4242, 5000]]);
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

  test('main failed before spawn (no pid): still cleans up', async () => {
    const w = newWorld();
    w.state = { workspace: '/tmp/ws' };
    w.jobFailed = true;
    await stop(stopDeps(w));
    expect(w.stopCalls).toEqual([]);
    expect(w.removed).toEqual(['/tmp/ws']);
  });
});
