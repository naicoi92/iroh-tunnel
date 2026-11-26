import { execFile, spawn as nodeSpawn } from 'node:child_process';
import { promisify } from 'node:util';
import { closeSync, openSync, readFileSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { exec } from '@actions/exec';

export type StopResult = 'exited' | 'killed' | 'still-alive' | 'identity-mismatch' | 'identity-unavailable';

export interface Launcher {
  spawn(
    exe: string,
    args: string[],
    env: Record<string, string>,
    logPath: string,
  ): Promise<{ pid: number; exitCode(): number | null }>;
  isAlive(pid: number): boolean;
  identity(pid: number): Promise<string | undefined>;
  stop(pid: number, graceMs: number, identity: string): Promise<StopResult>;
  stopAction(root: string, graceMs: number): Promise<StopResult>;
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

const execFileAsync = promisify(execFile);

async function processIdentity(platform: NodeJS.Platform, pid: number): Promise<string | undefined> {
  try {
    if (platform === 'linux') {
      const stat = readProcStat(pid);
      const end = stat?.lastIndexOf(')') ?? -1;
      // Field 22 (start time in clock ticks), relative to field 3 after comm.
      const start = end < 0 ? undefined : stat?.slice(end + 1).trim().split(/\s+/)[19];
      return start && /^\d+$/.test(start) ? `linux:${start}` : undefined;
    }
    if (platform === 'darwin') {
      // ps exposes start time only to the second; this cannot distinguish a
      // same-PID replacement started within that same second.
      const { stdout } = await execFileAsync('ps', ['-p', String(pid), '-o', 'lstart='], { timeout: 5000 });
      const start = stdout.trim();
      return start ? `darwin:${start}` : undefined;
    }
    if (platform === 'win32') {
      const { stdout } = await execFileAsync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command',
        `(Get-Process -Id ${pid} -ErrorAction Stop).StartTime.ToUniversalTime().Ticks`], { timeout: 5000, windowsHide: true });
      const start = stdout.trim();
      return /^\d+$/.test(start) ? `win32:${start}` : undefined;
    }
  } catch { /* an exited or inaccessible process has no usable identity */ }
  return undefined;
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
  readIdentity: (pid: number) => Promise<string | undefined> = (pid) => processIdentity(platform, pid),
  sendSignal: (pid: number, sig: NodeJS.Signals) => void = (pid, sig) => { process.kill(pid, sig); },
  forceWindows: (pid: number) => Promise<unknown> = (pid) => exec('taskkill', ['/PID', String(pid), '/F'], { ignoreReturnCode: true }),
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
      sendSignal(pid, sig);
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
    identity: readIdentity,

    async stopAction(root, graceMs) {
      const complete = async () => (await readFile(join(root, 'shutdown-complete'), 'utf8').catch(() => '')) === 'complete';
      if (await complete()) return 'exited';
      await writeFile(join(root, 'shutdown-request'), 'shutdown', { mode: 0o600 });
      const end = Date.now() + graceMs;
      while (Date.now() < end) {
        if (await complete()) return 'exited';
        await sleep(POLL_MS);
      }
      return (await complete()) ? 'exited' : 'still-alive';
    },

    async stop(pid, graceMs, identity) {
      if (!Number.isSafeInteger(pid) || pid <= 0) return 'identity-unavailable';
      const check = async (): Promise<StopResult | undefined> => {
        if (!alive(pid)) return 'exited';
        if (!identity) return 'identity-unavailable';
        const current = await readIdentity(pid);
        if (!current) return alive(pid) ? 'identity-unavailable' : 'exited';
        if (current !== identity) return 'identity-mismatch';
        return undefined;
      };
      // Identity lookup and signaling are not atomic with Node's PID APIs.
      const first = await check();
      if (first) return first;
      signal(pid, 'SIGTERM');
      if (await waitDead(pid, graceMs)) return 'exited';
      // Check again before escalation: the original may have exited while waiting.
      const second = await check();
      if (second) return second;
      if (platform === 'win32') {
        await forceWindows(pid);
      } else {
        signal(pid, 'SIGKILL');
      }
      return (await waitDead(pid, graceMs)) ? 'killed' : 'still-alive';
    },
  };
}
