import { expect, test } from 'bun:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createLauncher } from '../src/adapters/launcher';

test('cooperative cleanup uses only the invocation channel and never signals a reused PID', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'action-shutdown-'));
  const other = await mkdtemp(join(tmpdir(), 'action-shutdown-'));
  let signaled = false;
  const launcher = createLauncher(process.platform, undefined, undefined, () => { signaled = true; });
  try {
    const stop = launcher.stopAction(dir, 1000);
    for (let i = 0; i < 50; i++) {
      if (await readFile(join(dir, 'shutdown-request'), 'utf8').catch(() => '') === 'shutdown') break;
      await Bun.sleep(10);
    }
    expect(await readFile(join(dir, 'shutdown-request'), 'utf8')).toBe('shutdown');
    expect(await readFile(join(other, 'shutdown-request'), 'utf8').catch(() => '')).toBe('');
    await writeFile(join(dir, 'shutdown-complete'), 'complete');
    expect(await stop).toBe('exited');
    expect(signaled).toBe(false);
    expect(await launcher.stopAction(other, 10)).toBe('still-alive');
    expect(signaled).toBe(false);
  } finally {
    await rm(dir, { recursive: true, force: true });
    await rm(other, { recursive: true, force: true });
  }
});
