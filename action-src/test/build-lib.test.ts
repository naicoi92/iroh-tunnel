import { expect, test } from 'bun:test';
import { join, resolve } from 'node:path';
import { generateRootActionYml, normalizeBundle, workspaceVersion } from '../build-lib';

test('workspaceVersion reads [workspace.package] version only', () => {
  const toml = '[package]\nversion = "9.9.9"\n\n[workspace.package]\nedition = "2021"\nversion = "0.5.0"\n';
  expect(workspaceVersion(toml)).toBe('0.5.0');
});

test('workspaceVersion throws when missing', () => {
  expect(() => workspaceVersion('[package]\nversion = "1.0.0"\n')).toThrow();
});

test('generateRootActionYml rewrites main/post and prefixes comment', () => {
  const src = "name: x\nruns:\n  using: node24\n  main: dist/main.js\n  post: dist/post.js\n  post-if: always()\n";
  expect(generateRootActionYml(src)).toBe(
    '# Generated from access/action.yml by action-src/build.ts; do not edit.\n' +
      "name: x\nruns:\n  using: node24\n  main: access/dist/main.js\n  post: access/dist/post.js\n  post-if: always()\n",
  );
});

test('normalizeBundle replaces the absolute action-src path with a relative one', () => {
  const dir = '/home/ci/work/repo/action-src';
  const text = `var __dirname = "${dir}/node_modules/@actions/tool-cache/lib";\nx(__dirname);\n`;
  expect(normalizeBundle(text, dir)).toBe(
    'var __dirname = "action-src/node_modules/@actions/tool-cache/lib";\nx(__dirname);\n',
  );
});

test('normalizeBundle is a no-op when no absolute path is present', () => {
  expect(normalizeBundle('var a = 1;\n', '/x/action-src')).toBe('var a = 1;\n');
});

test('committed bundles contain no absolute build path', async () => {
  const root = resolve(import.meta.dir, '../..');
  for (const f of ['access/dist/main.js', 'access/dist/post.js', 'serve/dist/main.js', 'serve/dist/post.js']) {
    const text = await Bun.file(join(root, f)).text();
    expect(text.includes(root)).toBe(false);
    expect(text).not.toMatch(/var __dirname = "\//);
  }
});
