export type HostPort = { host: string; port: number; family: 'ipv4' | 'ipv6' | 'name' };

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;
const IPV6 = /^[0-9a-fA-F:.]+$/;
const NAME = /^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$/;

function parsePort(p: string): number {
  if (!/^\d{1,5}$/.test(p)) throw new Error(`invalid port "${p}"; expected 1-65535`);
  const n = Number(p);
  if (n < 1 || n > 65535) throw new Error(`invalid port "${p}"; expected 1-65535`);
  return n;
}

export function parseHostPort(s: string, opts: { allowName: boolean }): HostPort {
  const fmt = opts.allowName ? 'HOST:PORT or [IPV6]:PORT' : 'IPV4:PORT or [IPV6]:PORT';
  if (s.startsWith('[')) {
    const m = /^\[([^\]]*)\]:(.*)$/.exec(s);
    if (!m || !m[1] || !IPV6.test(m[1]) || !m[1].includes(':')) throw new Error(`invalid address; expected ${fmt}`);
    return { host: m[1], port: parsePort(m[2]!), family: 'ipv6' };
  }
  const idx = s.lastIndexOf(':');
  if (idx <= 0) throw new Error(`invalid address; expected ${fmt}`);
  const host = s.slice(0, idx);
  if (host.includes(':')) throw new Error('IPv6 addresses must be bracketed, e.g. [::1]:5432');
  const port = parsePort(s.slice(idx + 1));
  const v4 = IPV4.exec(host);
  if (v4) {
    if (v4.slice(1).some((o) => Number(o) > 255)) throw new Error('invalid IPv4 address');
    return { host, port, family: 'ipv4' };
  }
  if (!opts.allowName) throw new Error(`host names are not allowed; expected ${fmt}`);
  if (!NAME.test(host)) throw new Error('invalid host name');
  return { host, port, family: 'name' };
}
