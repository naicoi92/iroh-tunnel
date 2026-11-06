import { expect, test } from 'bun:test';
import { createServer } from 'node:net';
import { probe } from '../../src/adapters/probe';

function listen(): Promise<{ port: number; close(): void }> {
  return new Promise((res) => {
    const s = createServer((c) => c.end());
    s.listen(0, '127.0.0.1', () => res({ port: (s.address() as { port: number }).port, close: () => s.close() }));
  });
}

test('probe maps unspecified to loopback', async () => {
  const s = await listen();
  await probe({ host: '0.0.0.0', port: s.port, family: 'ipv4' }, 2000);
  s.close();
});

test('probe connects to a plain address', async () => {
  const s = await listen();
  await probe({ host: '127.0.0.1', port: s.port, family: 'ipv4' }, 2000);
  s.close();
});

test('probe rejects on refused connection within the timeout', async () => {
  const s = await listen();
  const port = s.port;
  s.close();
  await Bun.sleep(50);
  const t = Date.now();
  await expect(probe({ host: '127.0.0.1', port, family: 'ipv4' }, 2000)).rejects.toThrow();
  expect(Date.now() - t).toBeLessThan(2500);
});
