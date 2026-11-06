import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { checkMarkdown, type Declared } from './check-docs-lib';

const root = resolve(import.meta.dir, '../..');

async function inputs(file: string): Promise<Set<string>> {
  const meta = Bun.YAML.parse(await readFile(join(root, file), 'utf8')) as { inputs: Record<string, unknown> };
  return new Set(Object.keys(meta.inputs));
}

const declared: Declared = { access: await inputs('access/action.yml'), serve: await inputs('serve/action.yml') };
const files = ['docs/github-actions.md', 'README.md'];
const errors: string[] = [];
for (const f of files) {
  for (const e of checkMarkdown(await readFile(join(root, f), 'utf8'), declared)) errors.push(`${f}: ${e}`);
}
if (errors.length) {
  console.error(errors.join('\n'));
  process.exit(1);
}
console.log(`check-docs: ${files.length} files ok`);
