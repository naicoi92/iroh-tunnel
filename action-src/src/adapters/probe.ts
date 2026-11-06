import { connect } from 'node:net';
import type { HostPort } from '../domain/hostport';

export function probe(addr: HostPort, timeoutMs: number): Promise<void> {
  const host = addr.host === '0.0.0.0' ? '127.0.0.1' : addr.host === '::' ? '::1' : addr.host;
  return new Promise((resolve, reject) => {
    const s = connect({ host, port: addr.port });
    const timer = setTimeout(() => {
      s.destroy();
      reject(new Error(`timed out connecting to ${host}:${addr.port}`));
    }, timeoutMs);
    s.once('connect', () => {
      clearTimeout(timer);
      s.end();
      s.destroy();
      resolve();
    });
    s.once('error', (e) => {
      clearTimeout(timer);
      s.destroy();
      reject(e);
    });
  });
}
