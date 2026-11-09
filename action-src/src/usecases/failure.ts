import type { Role } from '../domain/inputs';
import { redact } from './redact';

const EXIT_CLASSES: Record<number, string> = {
  1: 'general',
  2: 'config',
  3: 'permission',
  4: 'iroh',
  5: 'service',
  6: 'conflict',
};

export function logTail(text: string, lines = 40): string {
  const all = text.split(/\r?\n/);
  while (all.length > 0 && all[all.length - 1] === '') all.pop();
  return all.slice(-lines).join('\n');
}

export function statusText(status: unknown): string {
  return status === undefined ? 'no status file yet' : JSON.stringify(status);
}

function withLog(head: string, log: string, secrets: readonly string[]): string {
  const tail = redact(logTail(log), secrets);
  return tail === '' ? `${head}\nTunnel log is empty.` : `${head}\nLast lines of the tunnel log:\n${tail}`;
}

export function exitMessage(
  role: Role,
  code: number,
  status: unknown,
  log: string,
  platform: NodeJS.Platform,
  secrets: readonly string[] = [],
): string {
  const cls = EXIT_CLASSES[code];
  let head = `iroh-tunnel-${role} exited before it was ready: exit code ${code}${cls ? ` (${cls})` : ''}. Last status: ${statusText(status)}.`;
  if (code === 6) {
    head += ` Another iroh-tunnel ${role} is already running for this runner user, possibly as a system service or another step of this job.`;
    head += ` Stop that system service or run the step as a different user.`;
    if (platform === 'win32') head += ` Use only one Serve and one Access step per job on Windows.`;
  }
  return withLog(head, log, secrets);
}

export function bindMessage(reason: string, status: unknown, log: string, secrets: readonly string[] = []): string {
  return withLog(`${reason} Last status: ${statusText(status)}.`, log, secrets);
}

export const NO_HOME_RELAY =
  'No home relay was registered; check the relay-url and relay-token inputs and outbound HTTPS access to the relay.';

export function timeoutMessage(
  role: Role,
  status: unknown,
  log: string,
  secrets: readonly string[] = [],
  reason?: string,
): string {
  const why = reason ? ` ${reason}` : '';
  const head = `iroh-tunnel-${role} was not ready within 60 s.${why} Last status: ${statusText(status)}.`;
  return withLog(head, log, secrets);
}
