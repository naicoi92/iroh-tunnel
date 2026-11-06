import type { Role } from '../domain/inputs';

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

function withLog(head: string, log: string): string {
  const tail = logTail(log);
  return tail === '' ? `${head}\nTunnel log is empty.` : `${head}\nLast lines of the tunnel log:\n${tail}`;
}

export function exitMessage(role: Role, code: number, log: string, platform: NodeJS.Platform): string {
  const cls = EXIT_CLASSES[code];
  let head = `iroh-tunnel-${role} exited before it was ready: exit code ${code}${cls ? ` (${cls})` : ''}.`;
  if (code === 6) {
    head += ` Another iroh-tunnel ${role} is already running for this runner user, possibly as a system service.`;
    head += ` Stop that system service or run the step as a different user.`;
    if (platform === 'win32') head += ` Use only one Serve and one Access step per job on Windows.`;
  }
  return withLog(head, log);
}

export function bindMessage(reason: string, log: string): string {
  return withLog(reason, log);
}

export function timeoutMessage(role: Role, status: unknown, log: string): string {
  const st = status === undefined ? 'no status file yet' : JSON.stringify(status);
  return withLog(`iroh-tunnel-${role} was not ready within 60 s. Last status: ${st}.`, log);
}
