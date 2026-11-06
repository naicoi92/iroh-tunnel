import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export interface Workspace {
  root: string;
  stateDir: string;
  homeDir: string;
  configPath: string;
  logPath: string;
  writeConfig(toml: string): Promise<void>;
}

export async function createWorkspace(runnerTemp: string): Promise<Workspace> {
  await mkdir(runnerTemp, { recursive: true });
  const root = await mkdtemp(join(runnerTemp, 'iroh-tunnel-'));
  const stateDir = join(root, 'state');
  const homeDir = join(root, 'home');
  await mkdir(stateDir);
  await mkdir(homeDir);
  const configPath = join(root, 'config.toml');
  return {
    root,
    stateDir,
    homeDir,
    configPath,
    logPath: join(root, 'tunnel.log'),
    async writeConfig(toml) {
      await writeFile(configPath, toml, { mode: 0o600 });
      if (process.platform !== 'win32') await chmod(configPath, 0o600);
    },
  };
}

export async function removeWorkspace(root: string): Promise<void> {
  await rm(root, { recursive: true, force: true });
}
