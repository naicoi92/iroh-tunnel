import { afterAll, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { createWorkspace, removeWorkspace } from '../../src/adapters/workspace';

const tmps: string[] = [];
async function temp() {
  const d = await mkdtemp(join(tmpdir(), 'ws-test-'));
  tmps.push(d);
  return d;
}
afterAll(async () => { for (const d of tmps) await rm(d, { recursive: true, force: true }); });

test('creates root with state and home; roots differ', async () => {
  const t = await temp();
  const a = await createWorkspace(t);
  const b = await createWorkspace(t);
  expect(a.root).not.toBe(b.root);
  expect(basename(a.root)).toMatch(/^iroh-tunnel-/);
  expect(a.stateDir).toBe(join(a.root, 'state'));
  expect(a.homeDir).toBe(join(a.root, 'home'));
  expect((await stat(a.stateDir)).isDirectory()).toBe(true);
  expect((await stat(a.homeDir)).isDirectory()).toBe(true);
});

test.skipIf(process.platform === 'win32')('writeConfig uses mode 0o600', async () => {
  const w = await createWorkspace(await temp());
  await w.writeConfig('a = 1\n');
  expect(await readFile(w.configPath, 'utf8')).toBe('a = 1\n');
  expect((await stat(w.configPath)).mode & 0o777).toBe(0o600);
});

test('removeWorkspace deletes the tree and tolerates missing', async () => {
  const w = await createWorkspace(await temp());
  await removeWorkspace(w.root);
  await expect(stat(w.root)).rejects.toThrow();
  await removeWorkspace(w.root);
});
