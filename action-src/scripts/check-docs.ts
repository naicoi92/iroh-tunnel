import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { checkMarkdown, type Declared } from './check-docs-lib';

const root = resolve(import.meta.dir, '../..');

type Meta = { inputs: Record<string, { required?: boolean }> };

async function meta(file: string): Promise<Meta> {
  return Bun.YAML.parse(await readFile(join(root, file), 'utf8')) as Meta;
}
const names = (m: Meta) => new Set(Object.keys(m.inputs));
const required = (m: Meta) => new Set(Object.entries(m.inputs).filter(([, v]) => v?.required === true).map(([k]) => k));

const access = await meta('access/action.yml');
const serve = await meta('serve/action.yml');
const declared: Declared = {
  access: names(access),
  serve: names(serve),
  required: { access: required(access), serve: required(serve) },
};
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
