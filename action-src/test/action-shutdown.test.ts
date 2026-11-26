import { expect, test } from 'bun:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createLauncher } from '../src/adapters/launcher';
import { createWorkspace, removeWorkspace, type Workspace } from '../src/adapters/workspace';
import { stop } from '../src/usecases/stop';
import { newWorld, stopDeps } from './fakes';

async function until(check: () => boolean | Promise<boolean>, ms = 5000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await check()) return;
    await Bun.sleep(10);
  }
  throw new Error('timed out waiting for child');
}

// Real detached processes exercise the action half of the file protocol.
// tests/action_shutdown.rs exercises the Rust half with actual tunnel binaries.
const cooperativeChild = `
  const fs = require('node:fs');
  const path = require('node:path');
  const root = process.env.IROH_TUNNEL_ACTION_SHUTDOWN_DIR;
  const timer = setInterval(() => {
    let request;
    try { request = fs.readFileSync(path.join(root, 'shutdown-request'), 'utf8'); } catch {}
    if (request !== 'shutdown') return;
    clearInterval(timer);
    fs.writeFileSync(path.join(root, 'shutdown-complete'), 'complete');
  }, 10);
  console.log('ready');
`;

function saved(ws: Workspace, pid: number, mainOk: boolean) {
  const w = newWorld();
  w.state = { workspace: ws.root, pid: String(pid), role: 'serve', 'shutdown-mode': 'action' };
  if (mainOk) w.state['main-ok'] = '1';
  else w.jobFailed = true;
  return w;
}

test('shuts down only the original action invocation', async () => {
  const root = await mkdtemp(join(tmpdir(), 'action-shutdown-'));
  const launch = createLauncher();
  const children: number[] = [];
  const signals: string[] = [];
  // A new launcher models post running in a separate process with no child
  // handles. No process identity lookup or PID signal is allowed on this path.
  const post = createLauncher(process.platform, undefined,
    async () => { throw new Error('cooperative cleanup must not inspect PID identity'); },
    (_pid, signal) => { signals.push(signal); },
    async () => { signals.push('taskkill'); });
  try {
    const first = await createWorkspace(root);
    const second = await createWorkspace(root);
    const a = await launch.spawn(process.execPath, ['-e', cooperativeChild], { IROH_TUNNEL_ACTION_SHUTDOWN_DIR: first.root }, first.logPath);
    children.push(a.pid);
    const b = await launch.spawn(process.execPath, ['-e', cooperativeChild], { IROH_TUNNEL_ACTION_SHUTDOWN_DIR: second.root }, second.logPath);
    children.push(b.pid);
    await until(async () => (await readFile(first.logPath, 'utf8')).includes('ready') && (await readFile(second.logPath, 'utf8')).includes('ready'));
    expect(a.pid).not.toBe(b.pid);

    // Simulate the saved numeric PID now belonging to an unrelated process.
    // Cleanup must still reach the original child, not the replacement PID.
    const wa = saved(first, process.pid, true);
    await stop(stopDeps(wa, { launcher: post, removeWorkspace }));
    await until(() => a.exitCode() !== null);
    expect(a.exitCode()).toBe(0);
    expect(wa.failed).toEqual([]);
    expect(b.exitCode()).toBeNull();
    expect(launch.isAlive(b.pid)).toBe(true);
    expect(await readFile(join(second.root, 'shutdown-request'), 'utf8').catch(() => '')).toBe('');
    expect(signals).toEqual([]);

    // The second main step failed after spawning; its own post still stops it.
    const wb = saved(second, b.pid, false);
    await stop(stopDeps(wb, { launcher: post, removeWorkspace }));
    await until(() => b.exitCode() !== null);
    expect(b.exitCode()).toBe(0);
    expect(wb.failed).toEqual([]);
    expect(signals).toEqual([]);
  } finally {
    for (const pid of children) {
      try { process.kill(pid, 'SIGKILL'); } catch { /* test teardown only */ }
    }
    await rm(root, { recursive: true, force: true });
  }
});

test('unconfirmed shutdown is bounded and never signals an unresponsive child', async () => {
  const root = await mkdtemp(join(tmpdir(), 'action-shutdown-'));
  const signals: string[] = [];
  const launcher = createLauncher(process.platform, undefined, undefined,
    (_pid, signal) => { signals.push(signal); }, async () => { signals.push('taskkill'); });
  let pid: number | undefined;
  try {
    const child = await launcher.spawn(process.execPath, ['-e', `console.log('ready'); setInterval(() => {}, 1000);`], {}, join(root, 'log'));
    pid = child.pid;
    await until(async () => (await readFile(join(root, 'log'), 'utf8')).includes('ready'));
    const started = Date.now();
    expect(await launcher.stopAction(root, 100)).toBe('still-alive');
    expect(Date.now() - started).toBeLessThan(1500);
    expect(child.exitCode()).toBeNull();
    expect(launcher.isAlive(child.pid)).toBe(true);
    expect(await readFile(join(root, 'shutdown-request'), 'utf8')).toBe('shutdown');
    expect(signals).toEqual([]);
  } finally {
    if (pid !== undefined) {
      try { process.kill(pid, 'SIGKILL'); } catch { /* test teardown only */ }
    }
    await rm(root, { recursive: true, force: true });
  }
});

test('an already completed invocation needs no new request or live PID', async () => {
  const root = await mkdtemp(join(tmpdir(), 'action-shutdown-'));
  try {
    await writeFile(join(root, 'shutdown-complete'), 'complete');
    expect(await createLauncher().stopAction(root, 0)).toBe('exited');
    expect(await readFile(join(root, 'shutdown-request'), 'utf8').catch(() => '')).toBe('');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
