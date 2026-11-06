import { InputError } from '../domain/errors';
import { renderConfig } from '../domain/config';
import { parseAccess, parseServe } from '../domain/inputs';
import { bindMessage, exitMessage, timeoutMessage } from './failure';
import { isReady } from './readiness';
import type { ActionsIO } from '../adapters/io';
import type { Installer } from '../adapters/release';
import type { Launcher } from '../adapters/launcher';
import type { probe } from '../adapters/probe';
import type { readStatus } from '../adapters/status';
import type { Workspace } from '../adapters/workspace';
import type { Role, TunnelRequest } from '../domain/inputs';

export type Clock = { now(): number; sleep(ms: number): Promise<void> };

export type StartDeps = {
  io: ActionsIO;
  installer: Installer;
  createWorkspace: (runnerTemp: string) => Promise<Workspace>;
  launcher: Launcher;
  readStatus: typeof readStatus;
  readLog: (path: string) => Promise<string>;
  probe: typeof probe;
  clock: Clock;
  platform: NodeJS.Platform;
  runnerTemp: string;
  env: Record<string, string | undefined>;
  writeFile: (path: string, text: string) => Promise<void>;
};


const POLL_MS = 500;
const MAX_POLLS = 120;
const DEADLINE_MS = 60_000;
const PROBE_TIMEOUT_MS = 2000;

class StartFailure extends Error {}

async function fail(deps: StartDeps, message: string): Promise<void> {
  deps.io.setFailed(message);
  const file = deps.env.IROH_TUNNEL_ACTION_ERROR_FILE;
  if (file) {
    try {
      await deps.writeFile(file, message);
    } catch (e) {
      deps.io.warning(`could not write ${file}: ${(e as Error).message}`);
    }
  }
}

async function run(req: TunnelRequest, deps: StartDeps): Promise<void> {
  const { io, launcher } = deps;
  const ws = await deps.createWorkspace(deps.runnerTemp);
  io.saveState('workspace', ws.root);

  const { exePath, version } = await deps.installer.install(req.role, req.version);
  io.info(`using iroh-tunnel-${req.role} ${version}`);

  await ws.writeConfig(renderConfig(req));
  const env: Record<string, string> = { IROH_TUNNEL_STATE_DIR: ws.stateDir, NO_COLOR: '1' };
  if (deps.platform !== 'win32') env.HOME = ws.homeDir;
  const child = await launcher.spawn(exePath, ['run', '-c', ws.configPath], env, ws.logPath);
  io.saveState('pid', String(child.pid));
  io.saveState('role', req.role);

  const secrets = req.relayToken === undefined ? [] : [req.relayToken];
  const deadline = deps.clock.now() + DEADLINE_MS;
  let probed = false;
  let lastStatus: Awaited<ReturnType<StartDeps['readStatus']>>;
  for (let i = 0; i < MAX_POLLS && deps.clock.now() < deadline; i++) {
    const code = child.exitCode();
    if (code !== null) {
      lastStatus = await deps.readStatus(ws.stateDir, req.role).catch(() => undefined);
      throw new StartFailure(exitMessage(req.role, code, lastStatus, await deps.readLog(ws.logPath), deps.platform, secrets));
    }

    lastStatus = await deps.readStatus(ws.stateDir, req.role);
    const log = await deps.readLog(ws.logPath);
    const r = isReady(req, lastStatus, log);
    if ('failed' in r) throw new StartFailure(bindMessage(r.failed, lastStatus, log, secrets));
    // The output is this node's own id, which only the status file knows.
    if (r.ready && lastStatus?.node_id) {
      if (req.role === 'access' && req.protocol === 'udp') {
        io.info('UDP listener is bound; peer connection not checked');
      }
      const nodeId = lastStatus.node_id;
      io.setOutput('node-id', nodeId);
      io.saveState('main-ok', '1');
      io.info(`iroh-tunnel-${req.role} is ready (node-id ${nodeId})`);
      return;
    }
    if (!r.ready && r.needsProbe && !probed && req.role === 'access') {
      probed = true;
      try {
        await deps.probe(req.listen, PROBE_TIMEOUT_MS);
      } catch (e) {
        io.info(`probe of ${req.listen.host}:${req.listen.port} failed: ${(e as Error).message}`);
      }
    }
    await deps.clock.sleep(POLL_MS);
  }
  throw new StartFailure(timeoutMessage(req.role, lastStatus, await deps.readLog(ws.logPath), secrets));
}

export async function start(req: TunnelRequest, deps: StartDeps): Promise<void> {
  // Mask first: nothing below may log before the token is registered.
  if (req.relayToken !== undefined) deps.io.setSecret(req.relayToken);
  try {
    await run(req, deps);
  } catch (e) {
    await fail(deps, (e as Error).message);
  }
}

export async function runMain(role: Role, deps: StartDeps): Promise<void> {
  const raw = (n: string) => deps.io.input(n);
  let req: TunnelRequest;
  try {
    req = role === 'access' ? parseAccess(raw) : parseServe(raw);
  } catch (e) {
    if (e instanceof InputError) {
      await fail(deps, `${e.message} (input: ${e.input})`);
      return;
    }
    throw e;
  }
  await start(req, deps);
}
