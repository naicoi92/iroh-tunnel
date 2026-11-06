import { describe, expect, test } from 'bun:test';
import { checkMarkdown, type Declared } from '../scripts/check-docs-lib';

const declared: Declared = {
  access: new Set(['name', 'node-id', 'listen', 'protocol']),
  serve: new Set(['name', 'target']),
};
const fence = (body: string) => '```yaml\n' + body + '\n```\n';

describe('checkMarkdown', () => {
  test('accepts declared inputs for access and serve', () => {
    const md =
      fence(
        'steps:\n  - uses: naicoi92/iroh-tunnel@v0\n    with:\n      name: db\n      node-id: abc\n      listen: 127.0.0.1:1',
      ) + fence('steps:\n  - uses: naicoi92/iroh-tunnel/serve@v0\n    with:\n      name: db\n      target: x:1');
    expect(checkMarkdown(md, declared)).toEqual([]);
  });

  test('rejects an undeclared key', () => {
    const md = fence('steps:\n  - uses: naicoi92/iroh-tunnel/serve@v0\n    with:\n      name: db\n      listen: x');
    const errs = checkMarkdown(md, declared);
    expect(errs).toHaveLength(1);
    expect(errs[0]).toContain('listen');
  });

  test('rejects job-status even when declared', () => {
    const d: Declared = { access: new Set(['name', 'job-status']), serve: new Set() };
    const md = fence('- uses: naicoi92/iroh-tunnel/access@v0\n  with:\n    job-status: success');
    expect(checkMarkdown(md, d)).toHaveLength(1);
  });

  test('ignores blocks without the action and other uses', () => {
    const md = fence('- uses: actions/checkout@v7\n  with:\n    bogus: 1');
    expect(checkMarkdown(md, declared)).toEqual([]);
  });

  test('reports unparsable yaml', () => {
    const md = fence('uses: naicoi92/iroh-tunnel@v0\n  with: [');
    expect(checkMarkdown(md, declared)).toHaveLength(1);
  });
});
