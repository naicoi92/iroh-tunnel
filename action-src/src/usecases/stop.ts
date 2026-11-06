import type { ActionsIO } from '../adapters/io';
import type { Launcher } from '../adapters/launcher';
import type { readStatus } from '../adapters/status';
import { join } from 'node:path';
import type { Role } from '../domain/inputs';
import { logTail } from './failure';
import { redact } from './redact';

export type StopDeps = {
  io: ActionsIO;
  launcher: Launcher;
  readStatus: typeof readStatus;
  readLog: (path: string) => Promise<string>;
  removeWorkspace: (root: string) => Promise<void>;
  platform: NodeJS.Platform;
};

const GRACE_MS = 5000;

const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

export async function stop(deps: StopDeps): Promise<void> {
  const { io } = deps;
  const root = io.getState('workspace');
  if (root === '') return;

  const role = io.getState('role') as Role | '';
  const pidText = io.getState('pid');
  const logPath = join(root, 'tunnel.log');
  const stateDir = join(root, 'state');
  const errors: string[] = [];
  const attempt = async (what: string, fn: () => Promise<void>) => {
    try {
      await fn();
    } catch (e) {
      errors.push(`${what}: ${msg(e)}`);
    }
  };

  // Every step is attempted: a failing diagnostic must not skip the kill or the cleanup.
  if (role !== '') {
    await attempt('reading the final status failed', () =>
      io.group('iroh-tunnel final status', async () => {
        const st = await deps.readStatus(stateDir, role);
        io.info(st ? JSON.stringify(st, null, 2) : 'no status available');
      }),
    );
  }

  const outcome: { result: 'exited' | 'killed' | 'still-alive' } = { result: 'exited' };
  if (pidText !== '') {
    await attempt('stopping the tunnel failed', async () => {
      outcome.result = await deps.launcher.stop(Number(pidText), GRACE_MS);
    });
  }

  if (io.jobFailed()) {
    // The post step is a separate process: re-register the token (action inputs are re-provided to it).
    const token = io.input('relay-token').trim();
    if (token !== '') io.setSecret(token);
    const secrets = token === '' ? [] : [token];
    await attempt('reading the tunnel log failed', () =>
      io.group('iroh-tunnel log', async () => {
        const tail = redact(logTail(await deps.readLog(logPath)), secrets);
        io.info(tail === '' ? 'Tunnel log is empty.' : tail);
      }),
    );
  }

  await attempt('removing the workspace failed', () => deps.removeWorkspace(root));
  if (outcome.result === 'still-alive') {
    errors.push(`iroh-tunnel-${role || 'tunnel'} (pid ${pidText}) did not exit after SIGKILL/taskkill.`);
  }
  if (errors.length > 0) io.setFailed(errors.join('\n'));
}
