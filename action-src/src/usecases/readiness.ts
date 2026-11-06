import type { StatusSnapshot } from '../adapters/status';
import { scanLog } from '../adapters/logscan';
import type { TunnelRequest } from '../domain/inputs';

export type Readiness = { ready: true } | { ready: false; needsProbe?: boolean } | { failed: string };

/**
 * Serve: the status file has a node id (home relay is not awaited, issue #177).
 * Access: the local listener is bound; TCP also needs a transport in the status row.
 */
export function isReady(req: TunnelRequest, status: StatusSnapshot | undefined, log: string): Readiness {
  if (req.role === 'serve') return status?.node_id ? { ready: true } : { ready: false };

  const sig = scanLog(log, req.listen);
  if (sig.bindError !== undefined) {
    return { failed: `cannot listen on ${req.listen.host}:${req.listen.port}: ${sig.bindError}` };
  }
  if (!sig.bound) return { ready: false };
  if (req.protocol === 'udp') return { ready: true };
  const row = status?.services.find((s) => s.name === req.name);
  return row?.transports && row.transports.length > 0 ? { ready: true } : { ready: false, needsProbe: true };
}
