import { spawn as nodeSpawn } from 'node:child_process';
import { closeSync, openSync } from 'node:fs';
import { exec } from '@actions/exec';

export interface Launcher {
  spawn(
    exe: string,
    args: string[],
    env: Record<string, string>,
    logPath: string,
  ): { pid: number; exitCode(): number | null };
  isAlive(pid: number): boolean;
  stop(pid: number, graceMs: number): Promise<'exited' | 'killed' | 'still-alive'>;
}

const POLL_MS = 50;
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
  } catch (e) {
    return (e as NodeJS.ErrnoException).code === 'EPERM';
  }
  return true;
}

export function createLauncher(platform: NodeJS.Platform = process.platform): Launcher {
  // Children spawned by this process; a reaped child leaves no zombie, so
  // kill(pid, 0) reports it dead as soon as it has exited.
  const exits = new Map<number, number | null>();

  async function waitDead(pid: number, ms: number): Promise<boolean> {
    const end = Date.now() + ms;
    while (alive(pid)) {
      if (Date.now() >= end) return false;
      await sleep(POLL_MS);
    }
    return true;
  }

  function signal(pid: number, sig: NodeJS.Signals): void {
    try {
      process.kill(pid, sig);
    } catch {
      /* already gone */
    }
  }

  return {
    spawn(exe, args, env, logPath) {
      const fd = openSync(logPath, 'a');
      try {
        const child = nodeSpawn(exe, args, {
          detached: true,
          stdio: ['ignore', fd, fd],
          windowsHide: true,
          env: { ...process.env, ...env },
        });
        const pid = child.pid;
        if (pid === undefined) throw new Error(`failed to start ${exe}`);
        exits.set(pid, null);
        child.on('exit', (code, sig) => exits.set(pid, code ?? (sig ? 128 : null)));
        child.on('error', () => exits.set(pid, 127));
        child.unref();
        return { pid, exitCode: () => exits.get(pid) ?? null };
      } finally {
        closeSync(fd);
      }
    },

    isAlive: alive,

    async stop(pid, graceMs) {
      if (!alive(pid)) return 'exited';
      if (platform === 'win32') {
        signal(pid, 'SIGTERM');
        if (await waitDead(pid, graceMs)) return 'exited';
        await exec('taskkill', ['/PID', String(pid), '/T', '/F'], { ignoreReturnCode: true });
        return (await waitDead(pid, graceMs)) ? 'killed' : 'still-alive';
      }
      signal(pid, 'SIGTERM');
      if (await waitDead(pid, graceMs)) return 'exited';
      signal(pid, 'SIGKILL');
      return (await waitDead(pid, graceMs)) ? 'killed' : 'still-alive';
    },
  };
}
