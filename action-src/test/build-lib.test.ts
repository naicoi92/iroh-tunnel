import { expect, test } from 'bun:test';
import { generateRootActionYml, workspaceVersion } from '../build-lib';

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
