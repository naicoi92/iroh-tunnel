import type { StatusSnapshot } from '../adapters/status';
import { scanLog } from '../adapters/logscan';
import type { TunnelRequest } from '../domain/inputs';

export type Readiness =
  | { ready: true }
  | { ready: false; needsProbe?: boolean; awaiting?: 'home_relay' }
  | { failed: string };

/**
 * Serve: the status file has a node id and a home relay.
 * Access: the local listener is bound; TCP also needs a transport in the status row.
 */
export function isReady(req: TunnelRequest, status: StatusSnapshot | undefined, log: string): Readiness {
  if (req.role === 'serve') {
    if (!status?.node_id) return { ready: false };
    return status.home_relay ? { ready: true } : { ready: false, awaiting: 'home_relay' };
  }

  const sig = scanLog(log, req.listen);
  if (sig.bindError !== undefined) {
    return { failed: `input \`listen\` (${req.listen.host}:${req.listen.port}): cannot bind: ${sig.bindError}.` };
  }
  if (!sig.bound) return { ready: false };
  if (req.protocol === 'udp') return { ready: true };
  const row = status?.services.find((s) => s.name === req.name);
  return row?.transports && row.transports.length > 0 ? { ready: true } : { ready: false, needsProbe: true };
}
