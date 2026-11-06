import { spawn as nodeSpawn } from 'node:child_process';
import { closeSync, openSync, readFileSync } from 'node:fs';
import { exec } from '@actions/exec';

export interface Launcher {
  spawn(
    exe: string,
    args: string[],
    env: Record<string, string>,
    logPath: string,
  ): Promise<{ pid: number; exitCode(): number | null }>;
  isAlive(pid: number): boolean;
  stop(pid: number, graceMs: number): Promise<'exited' | 'killed' | 'still-alive'>;
}

// The runner exposes every action input (INPUT_*) and saved state (STATE_*) in
// the environment; the tunnel binary must not inherit them (e.g. relay-token).
function inheritedEnv(): NodeJS.ProcessEnv {
  const out: NodeJS.ProcessEnv = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (!/^(INPUT_|STATE_)/i.test(k)) out[k] = v;
  }
  return out;
}

const POLL_MS = 50;
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Process state letter from a `/proc/<pid>/stat` line (field 3, after the last `)`). */
export function procState(stat: string): string | undefined {
  const end = stat.lastIndexOf(')');
  if (end < 0) return undefined;
  return stat.slice(end + 1).trim().split(/\s+/)[0] || undefined;
}

function readProcStat(pid: number): string | undefined {
  try {
    return readFileSync(`/proc/${pid}/stat`, 'utf8');
  } catch {
    return undefined;
  }
}

function signalable(pid: number): boolean {
  try {
    process.kill(pid, 0);
  } catch (e) {
    return (e as NodeJS.ErrnoException).code === 'EPERM';
  }
  return true;
}

export function createLauncher(
  platform: NodeJS.Platform = process.platform,
  readStat: (pid: number) => string | undefined = readProcStat,
): Launcher {
  // A detached tunnel is reparented to PID 1 once the main step exits. In a
  // `container:` job PID 1 does not reap, so a stopped tunnel stays a zombie
  // and kill(pid, 0) still succeeds; on Linux a zombie counts as dead.
  function alive(pid: number): boolean {
    if (!signalable(pid)) return false;
    if (platform === 'linux' && procState(readStat(pid) ?? '') === 'Z') return false;
    return true;
  }

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
    async spawn(exe, args, env, logPath) {
      const fd = openSync(logPath, 'a');
      try {
        const child = nodeSpawn(exe, args, {
          detached: true,
          stdio: ['ignore', fd, fd],
          windowsHide: true,
          env: { ...inheritedEnv(), ...env },
        });
        // Attach before anything else: ENOENT/EACCES surface as an async 'error'.
        const started = await new Promise<number>((resolve, reject) => {
          child.once('error', (e) => reject(new Error(`failed to start ${exe}: ${e.message}`)));
          child.once('spawn', () => resolve(child.pid as number));
        });
        const pid = started;
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
