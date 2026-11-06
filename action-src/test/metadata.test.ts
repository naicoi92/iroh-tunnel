import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const root = resolve(import.meta.dir, '../..');
const read = (p: string) => readFileSync(join(root, p), 'utf8');
type Meta = {
  name: string;
  description: string;
  branding: { icon: string; color: string };
  inputs: Record<string, { required?: boolean; default?: string; description?: string }>;
  outputs: Record<string, unknown>;
  runs: Record<string, string>;
};
const parse = (p: string) => Bun.YAML.parse(read(p)) as Meta;

const SHARED = ['protocol', 'relay-url', 'relay-token', 'version'];
const ACCESS = ['name', 'node-id', 'listen', ...SHARED, 'job-status'];
const SERVE = ['name', 'target', ...SHARED, 'job-status'];

describe('action metadata', () => {
  const access = parse('access/action.yml');
  const serve = parse('serve/action.yml');
  const rootMeta = parse('action.yml');

  test('root differs from access only in main/post and the generated comment', () => {
    const lines = (s: string) => s.split('\n');
    const [first, ...rest] = lines(read('action.yml'));
    expect(first).toBe('# Generated from access/action.yml by action-src/build.ts; do not edit.');
    const a = lines(read('access/action.yml'));
    expect(rest.length).toBe(a.length);
    const diff = rest.flatMap((l, i) => (l === a[i] ? [] : [[a[i], l]]));
    expect(diff).toEqual([
      [expect.stringMatching(/main: dist\/main\.js$/), expect.stringMatching(/main: access\/dist\/main\.js$/)],
      [expect.stringMatching(/post: dist\/post\.js$/), expect.stringMatching(/post: access\/dist\/post\.js$/)],
    ]);
    expect(rootMeta.runs.main).toBe('access/dist/main.js');
    expect(rootMeta.runs.post).toBe('access/dist/post.js');
  });

  test('access declares exactly the Access inputs and the node-id output', () => {
    expect(Object.keys(access.inputs)).toEqual(ACCESS);
    expect(Object.keys(access.outputs)).toEqual(['node-id']);
    for (const n of ['name', 'node-id', 'listen']) expect(access.inputs[n]!.required).toBe(true);
    expect(access.inputs.protocol!.default).toBe('tcp');
    for (const n of ['relay-url', 'relay-token', 'version']) expect(access.inputs[n]!.default).toBe('');
    expect(access.inputs['job-status']!.default).toBe('${{ job.status }}');
    expect(access.inputs['job-status']!.description).toBe('Internal; do not set.');
    expect(access.inputs['relay-token']!.description).toMatch(/secret/i);
    expect(access.inputs).not.toHaveProperty('target');
  });

  test('serve declares exactly the Serve inputs and the node-id output', () => {
    expect(Object.keys(serve.inputs)).toEqual(SERVE);
    expect(Object.keys(serve.outputs)).toEqual(['node-id']);
    for (const n of ['name', 'target']) expect(serve.inputs[n]!.required).toBe(true);
    expect(serve.inputs['job-status']!.default).toBe('${{ job.status }}');
    expect(serve.inputs['job-status']!.description).toBe('Internal; do not set.');
    expect(serve.inputs).not.toHaveProperty('listen');
    expect(serve.inputs).not.toHaveProperty('node-id');
  });

  test('runtime, naming and branding', () => {
    for (const m of [access, serve, rootMeta]) {
      expect(m.runs.using).toBe('node24');
      expect(m.runs['post-if']).toBe('always()');
      expect(m.description).not.toBe('');
      expect(m.branding).toEqual({ icon: 'link', color: 'blue' });
    }
    expect(rootMeta.name).toBe('iroh-tunnel');
    expect(access.name).toBe('iroh-tunnel');
    expect(serve.name).toBe('iroh-tunnel-serve');
    expect(serve.runs.main).toBe('dist/main.js');
    expect(serve.runs.post).toBe('dist/post.js');
  });
});
