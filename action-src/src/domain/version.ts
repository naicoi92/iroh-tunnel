export type VersionSpec =
  | { kind: 'default' }
  | { kind: 'latest' }
  | { kind: 'exact'; version: string };

const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/;

export function parseVersion(s: string): VersionSpec {
  if (s === '') return { kind: 'default' };
  if (s === 'latest') return { kind: 'latest' };
  const v = s.startsWith('v') ? s.slice(1) : s;
  if (!SEMVER.test(v)) throw new Error(`invalid version "${s}"; expected latest or X.Y.Z[-pre]`);
  return { kind: 'exact', version: v };
}

function cmpNum(a: number, b: number): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function cmpPre(a: string, b: string): number {
  const x = a.split('.');
  const y = b.split('.');
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    const p = x[i]!;
    const q = y[i]!;
    const pn = /^\d+$/.test(p);
    const qn = /^\d+$/.test(q);
    if (pn && qn) {
      const c = cmpNum(Number(p), Number(q));
      if (c) return c;
    } else if (pn !== qn) {
      return pn ? -1 : 1;
    } else if (p !== q) {
      return p < q ? -1 : 1;
    }
  }
  return cmpNum(x.length, y.length);
}

export function compareVersions(a: string, b: string): number {
  const pa = SEMVER.exec(a);
  const pb = SEMVER.exec(b);
  if (!pa || !pb) throw new Error(`invalid version "${pa ? b : a}"`);
  for (let i = 1; i <= 3; i++) {
    const c = cmpNum(Number(pa[i]), Number(pb[i]));
    if (c) return c;
  }
  const ra = pa[4];
  const rb = pb[4];
  if (ra === undefined && rb === undefined) return 0;
  if (ra === undefined) return 1;
  if (rb === undefined) return -1;
  return cmpPre(ra, rb);
}
