export const GENERATED_HEADER =
  '# Generated from access/action.yml by action-src/build.ts; do not edit.\n';

/** Extract `version` from the `[workspace.package]` table of a Cargo.toml. */
export function workspaceVersion(cargoToml: string): string {
  const m = /^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"/m.exec(cargoToml);
  if (!m) throw new Error('version not found in [workspace.package] of Cargo.toml');
  return m[1]!;
}

/** Derive the root action.yml from the Access metadata (main/post lines only). */
export function generateRootActionYml(accessYml: string): string {
  const out = accessYml
    .replace(/^(\s*)main:\s*dist\/main\.js\s*$/m, '$1main: access/dist/main.js')
    .replace(/^(\s*)post:\s*dist\/post\.js\s*$/m, '$1post: access/dist/post.js');
  if (out === accessYml) throw new Error('access/action.yml has no main/post lines to rewrite');
  return GENERATED_HEADER + out;
}

/**
 * Make a bundle independent of the checkout path. Bun injects
 * `var __dirname = "<abs>/action-src/node_modules/..."` for CJS deps that use
 * `__dirname`; rewrite the absolute `action-src` prefix to a fixed relative one.
 */
export function normalizeBundle(text: string, actionSrcDir: string): string {
  // Cover the POSIX form, the raw Windows form and the JSON-escaped Windows form.
  const forms = new Set([actionSrcDir, actionSrcDir.replace(/\\/g, '\\\\')]);
  let out = text;
  for (const f of forms) out = out.split(f).join('action-src');
  const leftover = /var __dirname = "(\/|[A-Za-z]:)/.exec(out);
  if (leftover) {
    throw new Error('bundle still contains an absolute __dirname path after normalization');
  }
  return out;
}
