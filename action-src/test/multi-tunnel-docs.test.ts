import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const docs = readFileSync(new URL('../../docs/github-actions.md', import.meta.url), 'utf8');

test('documents concurrent tunnels and legacy Windows pins', () => {
  const example = docs.split('## Two services')[1]?.split('## Own relay')[0] ?? '';
  expect(example).toMatch(/one step per service/i);
  expect(example).toContain('id: postgres');
  expect(example).toContain('id: redis');
  expect(example).toContain('listen: 127.0.0.1:15432');
  expect(example).toContain('listen: 127.0.0.1:16379');
  expect(example).toContain('steps.postgres.outputs.node-id');
  expect(example).toContain('steps.redis.outputs.node-id');
  expect(example).toMatch(/each step[\s\S]*cleanup/i);
  expect(docs).toMatch(/Windows[\s\S]*0\.8\.0[\s\S]*0\.7\.0[\s\S]*one Access[\s\S]*one Serve/i);
  expect(docs).not.toContain('On Windows, use at most one Access step and one Serve step per job.');
});
