import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { build } from '../build';

const repoRoot = resolve(import.meta.dir, '../..');
const FILES = [
  'access/dist/main.js',
  'access/dist/post.js',
  'serve/dist/main.js',
  'serve/dist/post.js',
  'action.yml',
];

const tmp = await mkdtemp(join(tmpdir(), 'check-dist-'));
const diffs: string[] = [];
try {
  await build(tmp);
  for (const f of FILES) {
    const [a, b] = await Promise.all([
      readFile(join(tmp, f)).catch(() => undefined),
      readFile(join(repoRoot, f)).catch(() => undefined),
    ]);
    if (!a || !b || !a.equals(b)) diffs.push(f);
  }
} finally {
  await rm(tmp, { recursive: true, force: true });
}
if (diffs.length > 0) {
  console.error('generated files out of date (run `bun run build` in action-src/):');
  for (const f of diffs) console.error(`  ${f}`);
  process.exit(1);
}
