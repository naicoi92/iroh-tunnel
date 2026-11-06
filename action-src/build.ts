import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { generateRootActionYml, normalizeBundle, workspaceVersion } from './build-lib';

/** First release that ships the actions. */
const MIN_VERSION = '0.6.0';

const repoRoot = resolve(import.meta.dir, '..');

const ENTRIES = [
  { entry: 'src/entry/access-main.ts', out: 'access/dist/main.js' },
  { entry: 'src/entry/access-post.ts', out: 'access/dist/post.js' },
  { entry: 'src/entry/serve-main.ts', out: 'serve/dist/main.js' },
  { entry: 'src/entry/serve-post.ts', out: 'serve/dist/post.js' },
];

export async function build(outRoot: string = repoRoot): Promise<void> {
  const buildVersion = workspaceVersion(await readFile(join(repoRoot, 'Cargo.toml'), 'utf8'));
  for (const { entry, out } of ENTRIES) {
    const result = await Bun.build({
      entrypoints: [join(import.meta.dir, entry)],
      target: 'node',
      format: 'esm',
      minify: false,
      define: {
        __BUILD_VERSION__: JSON.stringify(buildVersion),
        __MIN_VERSION__: JSON.stringify(MIN_VERSION),
      },
    });
    if (!result.success || result.outputs.length !== 1) {
      throw new Error(`bundling ${entry} failed:\n${result.logs.join('\n')}`);
    }
    const dest = join(outRoot, out);
    await mkdir(dirname(dest), { recursive: true });
    await writeFile(dest, normalizeBundle(await result.outputs[0]!.text(), import.meta.dir));
  }
  const accessYml = await readFile(join(repoRoot, 'access/action.yml'), 'utf8');
  await writeFile(join(outRoot, 'action.yml'), generateRootActionYml(accessYml));
}

if (import.meta.main) {
  const i = process.argv.indexOf('--out');
  const out = i >= 0 ? process.argv[i + 1] : undefined;
  if (i >= 0 && !out) throw new Error('--out requires a directory');
  await build(out ? resolve(out) : repoRoot);
}
