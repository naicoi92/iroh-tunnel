export type Declared = {
  access: Set<string>;
  serve: Set<string>;
  /** Inputs with `required: true`; omitted means no requirement check. */
  required?: { access: Set<string>; serve: Set<string> };
};

const PREFIX = 'naicoi92/iroh-tunnel';
const INTERNAL = new Set(['job-status']);

/** Yaml fenced blocks that mention the action. */
export function yamlBlocks(md: string): string[] {
  const out: string[] = [];
  const re = /^```ya?ml[^\n]*\n([\s\S]*?)^```/gm;
  for (let m = re.exec(md); m; m = re.exec(md)) {
    if (m[1]!.includes(PREFIX)) out.push(m[1]!);
  }
  return out;
}

/** `null` means the path is under this repo but names no known action. */
function role(uses: string): 'access' | 'serve' | null | undefined {
  if (uses !== PREFIX && !uses.startsWith(`${PREFIX}/`) && !uses.startsWith(`${PREFIX}@`)) return undefined;
  const rest = uses.slice(PREFIX.length).split('@')[0]!;
  if (rest === '' || rest === '/access') return 'access';
  if (rest === '/serve') return 'serve';
  return null;
}

function walk(v: unknown, f: (o: Record<string, unknown>) => void): void {
  if (Array.isArray(v)) v.forEach((x) => walk(x, f));
  else if (v && typeof v === 'object') {
    f(v as Record<string, unknown>);
    Object.values(v).forEach((x) => walk(x, f));
  }
}

/** Returns one error per undeclared or internal `with:` key, missing required input, unknown action path, or unparsable block. */
export function checkMarkdown(md: string, declared: Declared): string[] {
  const errors: string[] = [];
  yamlBlocks(md).forEach((block, i) => {
    let doc: unknown;
    try {
      doc = Bun.YAML.parse(block);
    } catch (e) {
      errors.push(`yaml block ${i + 1}: cannot parse: ${(e as Error).message}`);
      return;
    }
    walk(doc, (o) => {
      if (typeof o.uses !== 'string') return;
      const r = role(o.uses);
      if (r === undefined) return;
      if (r === null) {
        errors.push(`${o.uses}: unknown action path (expected ${PREFIX}, ${PREFIX}/access or ${PREFIX}/serve)`);
        return;
      }
      const w = o.with && typeof o.with === 'object' ? (o.with as Record<string, unknown>) : {};
      for (const req of declared.required?.[r] ?? []) {
        if (!(req in w)) errors.push(`${o.uses}: required input '${req}' is missing`);
      }
      for (const key of Object.keys(w)) {
        if (INTERNAL.has(key)) errors.push(`${o.uses}: input '${key}' is internal`);
        else if (!declared[r].has(key)) errors.push(`${o.uses}: input '${key}' is not declared in the ${r} action.yml`);
      }
    });
  });
  return errors;
}
