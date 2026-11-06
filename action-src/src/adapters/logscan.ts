import type { HostPort } from '../domain/hostport';

export type LogSignals = { bound: boolean; bindError?: string };

export function scanLog(text: string, listen: HostPort): LogSignals {
  // The CLI prints the address as written in config: `host:port`, IPv6 unbracketed.
  const addr = `${listen.host}:${listen.port}`;
  const boundMarks = [`listening for local clients listen_addr=${addr}`, `listening for local udp clients listen_addr=${addr}`];
  const errMarks = [`failed to bind ${addr}:`, `failed to bind udp ${addr}:`];
  const out: LogSignals = { bound: false };
  for (const line of text.split(/\r?\n/)) {
    for (const m of boundMarks) {
      const i = line.indexOf(m);
      if (i >= 0 && !/\d/.test(line[i + m.length] ?? '')) out.bound = true;
    }
    if (out.bindError === undefined && errMarks.some((m) => line.includes(m))) out.bindError = line.trim();
  }
  return out;
}
