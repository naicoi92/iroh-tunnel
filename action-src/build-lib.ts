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
