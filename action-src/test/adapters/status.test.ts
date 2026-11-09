import { afterAll, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { readStatus } from '../../src/adapters/status';

let dir: string;
const ready = (async () => { dir = await mkdtemp(join(tmpdir(), 'status-test-')); })();
afterAll(() => rm(dir, { recursive: true, force: true }));

test('missing file is undefined', async () => {
  await ready;
  expect(await readStatus(dir, 'serve')).toBeUndefined();
});

test('half-written JSON is undefined', async () => {
  await ready;
  await writeFile(join(dir, 'access-status.json'), '{"node_id": "ab');
  expect(await readStatus(dir, 'access')).toBeUndefined();
});

test('reads serve and access snapshots', async () => {
  await ready;
  await writeFile(join(dir, 'serve-status.json'), JSON.stringify({ node_id: 'n1', home_relay: null, pid: 1, started_at: 't', services: [{ name: 'a' }], connections: [] }));
  await writeFile(join(dir, 'access-status.json'), JSON.stringify({ node_id: 'n2', pid: 1, started_at: 't', services: [{ name: 'a', listen_addr: 'x', peer: 'p', transports: [1], local_bound_addrs: [] }] }));
  expect((await readStatus(dir, 'serve'))?.node_id).toBe('n1');
  const a = await readStatus(dir, 'access');
  expect(a?.node_id).toBe('n2');
  expect(a?.services[0]?.transports).toEqual([1]);
});

test('serve home_relay: string kept, null/empty/non-string normalized to null, absent omitted', async () => {
  await ready;
  const put = (o: object) => writeFile(join(dir, 'serve-status.json'), JSON.stringify({ node_id: 'n1', services: [], ...o }));
  await put({ home_relay: 'https://r.example./' });
  expect((await readStatus(dir, 'serve'))?.home_relay).toBe('https://r.example./');
  for (const home_relay of [null, '', 42]) {
    await put({ home_relay });
    expect((await readStatus(dir, 'serve'))?.home_relay).toBeNull();
  }
  await put({});
  expect(await readStatus(dir, 'serve')).not.toHaveProperty('home_relay');
});

test('JSON without node_id is not yet', async () => {
  await ready;
  await writeFile(join(dir, 'serve-status.json'), '{"services": []}');
  expect(await readStatus(dir, 'serve')).toBeUndefined();
});
