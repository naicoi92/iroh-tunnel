import * as toolCache from '@actions/tool-cache';
import { readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createIO } from '../adapters/io';
import { createLauncher } from '../adapters/launcher';
import { probe } from '../adapters/probe';
import { createInstaller } from '../adapters/release';
import { readStatus } from '../adapters/status';
import { isMusl, resolveTarget } from '../adapters/target';
import { createWorkspace, removeWorkspace } from '../adapters/workspace';
import type { StartDeps } from '../usecases/start';
import type { StopDeps } from '../usecases/stop';

const RELEASE_ROOT = 'https://github.com/naicoi92/iroh-tunnel/releases';

export function realStartDeps(): StartDeps {
  const binDir = process.env.IROH_TUNNEL_ACTION_BIN_DIR;
  return {
    io: createIO(),
    installer: createInstaller({
      releaseRoot: RELEASE_ROOT,
      target: resolveTarget(process.platform, process.arch, isMusl()),
      cache: toolCache,
      fetch,
      binDir: binDir ? binDir : undefined,
    }),
    createWorkspace,
    launcher: createLauncher(),
    readStatus,
    readLog: (path) => readFile(path, 'utf8').catch(() => ''),
    probe,
    clock: { now: () => Date.now(), sleep: (ms) => new Promise((r) => setTimeout(r, ms)) },
    platform: process.platform,
    runnerTemp: process.env.RUNNER_TEMP || tmpdir(),
    env: process.env,
    writeFile: (path, text) => writeFile(path, text),
  };
}

export function realStopDeps(): StopDeps {
  return {
    io: createIO(),
    launcher: createLauncher(),
    readStatus,
    readLog: (path) => readFile(path, 'utf8').catch(() => ''),
    removeWorkspace,
    platform: process.platform,
  };
}
