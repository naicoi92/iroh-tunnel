import type { ActionsIO } from '../src/adapters/io';
import type { Launcher } from '../src/adapters/launcher';
import type { StatusSnapshot } from '../src/adapters/status';
import type { Workspace } from '../src/adapters/workspace';
import type { StartDeps } from '../src/usecases/start';
import type { StopDeps } from '../src/usecases/stop';

export const SECRET_KEY = 'c2VjcmV0LWtleS1iYXNlNjQtZml4dHVyZS0xMjM0NTY3ODkw';
export const NODE = 'a'.repeat(64);

export type World = {
  calls: string[];
  /** every string passed to a non-setSecret IO call */
  out: string[];
  outputs: Record<string, string>;
  state: Record<string, string>;
  inputs: Record<string, string>;
  failed: string[];
  secrets: string[];
  files: Map<string, string>;
  status?: StatusSnapshot;
  exit: number | null;
  spawned: { exe: string; args: string[]; env: Record<string, string>; logPath: string }[];
  probes: number;
  polls: number;
  jobFailed: boolean;
  stopResult: 'exited' | 'killed' | 'still-alive';
  stopCalls: [number, number][];
  removed: string[];
  /** hook run on each clock.sleep (simulates the world changing) */
  onSleep: (w: World, n: number) => void;
  /** hook run when the child is spawned */
  onSpawn: (w: World) => void;
  installed: number;
};

export const WS: Workspace = {
  root: '/tmp/ws',
  stateDir: '/tmp/ws/state',
  homeDir: '/tmp/ws/home',
  configPath: '/tmp/ws/config.toml',
  logPath: '/tmp/ws/tunnel.log',
  async writeConfig() {},
};

export function newWorld(inputs: Record<string, string> = {}): World {
  return {
    calls: [],
    out: [],
    outputs: {},
    state: {},
    inputs,
    failed: [],
    secrets: [],
    files: new Map(),
    exit: null,
    spawned: [],
    probes: 0,
    polls: 0,
    jobFailed: false,
    stopResult: 'exited',
    stopCalls: [],
    removed: [],
    onSleep: () => {},
    onSpawn: () => {},
    installed: 0,
  };
}

export function fakeIO(w: World): ActionsIO {
  const rec = (kind: string) => (m: string) => {
    w.calls.push(`${kind}:${m}`);
    w.out.push(m);
  };
  return {
    input: (n) => w.inputs[n] ?? '',
    setOutput: (k, v) => {
      w.calls.push(`output:${k}`);
      w.out.push(`${k}=${v}`);
      w.outputs[k] = v;
    },
    setSecret: (v) => {
      w.calls.push('setSecret');
      w.secrets.push(v);
    },
    saveState: (k, v) => {
      w.calls.push(`save:${k}`);
      w.out.push(`${k}=${v}`);
      w.state[k] = v;
    },
    getState: (k) => w.state[k] ?? '',
    info: rec('info'),
    warning: rec('warning'),
    group: async (t, fn) => {
      w.calls.push(`group:${t}`);
      w.out.push(t);
      return fn();
    },
    setFailed: (m) => {
      w.calls.push('setFailed');
      w.out.push(m);
      w.failed.push(m);
    },
    jobFailed: () => w.jobFailed,
  };
}

export function fakeLauncher(w: World): Launcher {
  return {
    async spawn(exe, args, env, logPath) {
      w.calls.push('spawn');
      w.spawned.push({ exe, args, env, logPath });
      w.onSpawn(w);
      return { pid: 4242, exitCode: () => w.exit };
    },
    isAlive: () => true,
    async stop(pid, grace) {
      w.calls.push('launcher.stop');
      w.stopCalls.push([pid, grace]);
      return w.stopResult;
    },
  };
}

export function startDeps(w: World, over: Partial<StartDeps> = {}): StartDeps {
  let now = 0;
  let sleeps = 0;
  return {
    io: fakeIO(w),
    installer: {
      async install(role) {
        w.calls.push('install');
        w.installed++;
        return { exePath: `/bin/iroh-tunnel-${role}`, version: '0.6.0' };
      },
    },
    createWorkspace: async () => {
      w.calls.push('createWorkspace');
      return WS;
    },
    launcher: fakeLauncher(w),
    readStatus: async () => {
      w.polls++;
      return w.status;
    },
    readLog: async (p) => w.files.get(p) ?? '',
    probe: async () => {
      w.probes++;
    },
    clock: {
      now: () => now,
      sleep: async (ms) => {
        now += ms;
        sleeps++;
        w.onSleep(w, sleeps);
      },
    },
    platform: 'linux',
    runnerTemp: '/tmp',
    env: {},
    writeFile: async (p, t) => {
      w.files.set(p, t);
      w.calls.push(`writeFile:${p}`);
    },
    ...over,
  };
}

export function stopDeps(w: World, over: Partial<StopDeps> = {}): StopDeps {
  return {
    io: fakeIO(w),
    launcher: fakeLauncher(w),
    readStatus: async () => w.status,
    readLog: async (p) => w.files.get(p) ?? '',
    removeWorkspace: async (r) => {
      w.calls.push('removeWorkspace');
      w.removed.push(r);
    },
    platform: 'linux',
    ...over,
  };
}
