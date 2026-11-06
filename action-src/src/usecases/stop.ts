import type { ActionsIO } from '../adapters/io';
import type { Launcher } from '../adapters/launcher';
import type { readStatus } from '../adapters/status';
import { join } from 'node:path';
import type { Role } from '../domain/inputs';
import { logTail } from './failure';

export type StopDeps = {
  io: ActionsIO;
  launcher: Launcher;
  readStatus: typeof readStatus;
  readLog: (path: string) => Promise<string>;
  removeWorkspace: (root: string) => Promise<void>;
  platform: NodeJS.Platform;
};

const GRACE_MS = 5000;

export async function stop(deps: StopDeps): Promise<void> {
  const { io } = deps;
  const root = io.getState('workspace');
  if (root === '') return;

  const role = io.getState('role') as Role | '';
  const pidText = io.getState('pid');
  const logPath = join(root, 'tunnel.log');
  const stateDir = join(root, 'state');

  if (role !== '') {
    await io.group('iroh-tunnel final status', async () => {
      const st = await deps.readStatus(stateDir, role);
      io.info(st ? JSON.stringify(st, null, 2) : 'no status available');
    });
  }

  let result: 'exited' | 'killed' | 'still-alive' = 'exited';
  if (pidText !== '') result = await deps.launcher.stop(Number(pidText), GRACE_MS);

  if (io.jobFailed()) {
    await io.group('iroh-tunnel log', async () => {
      const tail = logTail(await deps.readLog(logPath));
      io.info(tail === '' ? 'Tunnel log is empty.' : tail);
    });
  }

  await deps.removeWorkspace(root);
  if (result === 'still-alive') {
    io.setFailed(`iroh-tunnel-${role || 'tunnel'} (pid ${pidText}) did not exit after SIGKILL/taskkill.`);
  }
}
