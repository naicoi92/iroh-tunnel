// Manual smoke run: simulates the Actions runner and drives the real bundles with the
// debug binaries (needs the internet for the n0 public relays).
//   cargo build --locked --bin iroh-tunnel-serve --bin iroh-tunnel-access
//   bun run scripts/local-smoke.ts
import { spawn, spawnSync } from 'node:child_process';
import { createSocket } from 'node:dgram';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { createServer, connect, type AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const repo = resolve(import.meta.dir, '../..');
const binDir = join(repo, 'target/debug');

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(`SMOKE FAILED: ${msg}`);
}

/** Parse the heredoc form written by @actions/core 3: name<<delim\nvalue\ndelim. */
function parseCommandFile(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const m = /^(.+?)<<(ghadelimiter_.+)$/.exec(lines[i]!);
    if (!m) continue;
    const value: string[] = [];
    for (i++; i < lines.length && lines[i] !== m[2]; i++) value.push(lines[i]!);
    out[m[1]!] = value.join('\n');
  }
  return out;
}

type Step = { script: string; inputs: Record<string, string>; state?: Record<string, string> };

async function runStep(dir: string, label: string, s: Step) {
  const output = join(dir, `${label}.output`);
  const stateFile = join(dir, `${label}.state`);
  await writeFile(output, '');
  await writeFile(stateFile, '');
  const env: Record<string, string> = {
    ...(process.env as Record<string, string>),
    RUNNER_TEMP: join(dir, 'runner-temp'),
    GITHUB_OUTPUT: output,
    GITHUB_STATE: stateFile,
    IROH_TUNNEL_ACTION_BIN_DIR: binDir,
  };
  for (const [k, v] of Object.entries(s.inputs)) env[`INPUT_${k.replace(/ /g, '_').toUpperCase()}`] = v;
  for (const [k, v] of Object.entries(s.state ?? {})) env[`STATE_${k}`] = v;
  const r = await new Promise<{ code: number | null; out: string }>((done) => {
    const p = spawn('node', [join(repo, s.script)], { env, stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    p.stdout.on('data', (d) => (out += d));
    p.stderr.on('data', (d) => (out += d));
    p.on('close', (code) => done({ code, out }));
  });
  console.log(`--- ${label} (exit ${r.code})\n${r.out.trimEnd()}`);
  assert(r.code === 0 && !r.out.includes('::error::'), `${label} failed`);
  return {
    outputs: parseCommandFile(await readFile(output, 'utf8')),
    state: parseCommandFile(await readFile(stateFile, 'utf8')),
    log: r.out,
  };
}

const freePort = () =>
  new Promise<number>((res) => {
    const s = createServer().listen(0, '127.0.0.1', () => {
      const p = (s.address() as AddressInfo).port;
      s.close(() => res(p));
    });
  });

function tcpRoundTrip(port: number, payload: string) {
  return new Promise<string>((res, rej) => {
    const c = connect(port, '127.0.0.1', () => c.write(payload));
    c.once('data', (d) => {
      res(d.toString());
      c.destroy();
    });
    c.once('error', rej);
    setTimeout(() => rej(new Error('tcp round trip timed out')), 15_000);
  });
}

function udpRoundTrip(port: number, payload: string) {
  return new Promise<string>((res, rej) => {
    const sock = createSocket('udp4');
    sock.bind(0, '127.0.0.1');
    sock.once('message', (m) => {
      res(m.toString());
      sock.close();
    });
    const send = () => sock.send(payload, port, '127.0.0.1');
    send();
    const retry = setInterval(send, 1000);
    setTimeout(() => {
      clearInterval(retry);
      rej(new Error('udp round trip timed out'));
    }, 40_000).unref();
    sock.once('close', () => clearInterval(retry));
  });
}

const dir = await mkdtemp(join(tmpdir(), 'iroh-smoke-'));
const tcpEcho = createServer((c) => c.pipe(c));
await new Promise<void>((r) => tcpEcho.listen(0, '127.0.0.1', r));
const udpEcho = createSocket('udp4');
udpEcho.on('message', (m, r) => udpEcho.send(m, r.port, r.address));
await new Promise<void>((r) => udpEcho.bind(0, '127.0.0.1', r));
const tcpTarget = (tcpEcho.address() as AddressInfo).port;
const udpTarget = udpEcho.address().port;

const posts: Step[] = [];
try {
  for (const proto of ['tcp', 'udp'] as const) {
    const name = `smoke-${proto}`;
    const target = proto === 'tcp' ? tcpTarget : udpTarget;
    const serve = await runStep(dir, `serve-${proto}-main`, {
      script: 'serve/dist/main.js',
      inputs: { name, target: `127.0.0.1:${target}`, protocol: proto, 'job-status': 'success' },
    });
    const nodeId = serve.outputs['node-id'];
    assert(nodeId, 'serve did not set node-id');
    posts.push({ script: 'serve/dist/post.js', inputs: { 'job-status': 'success' }, state: serve.state });

    const listen = await freePort();
    const access = await runStep(dir, `access-${proto}-main`, {
      script: 'access/dist/main.js',
      inputs: { name, 'node-id': nodeId, listen: `127.0.0.1:${listen}`, protocol: proto, 'job-status': 'success' },
    });
    assert(access.outputs['node-id'], 'access did not set node-id');
    posts.push({ script: 'access/dist/post.js', inputs: { 'job-status': 'success' }, state: access.state });

    const mode = ((await stat(join(access.state.workspace!, 'config.toml'))).mode & 0o777).toString(8);
    assert(mode === '600', `config.toml mode is ${mode}, expected 600`);
    console.log(`config.toml mode after the binary saved its key: ${mode}`);

    const payload = `hello-${proto}-${Date.now()}`;
    const echoed = proto === 'tcp' ? await tcpRoundTrip(listen, payload) : await udpRoundTrip(listen, payload);
    assert(echoed === payload, `${proto} round trip: sent ${payload}, got ${echoed}`);
    console.log(`${proto} round trip OK: ${echoed}`);
  }
} catch (e) {
  for (const p of posts) {
    const log = await readFile(join(p.state!.workspace!, 'tunnel.log'), 'utf8').catch(() => '');
    console.error(`--- tunnel.log (${p.script})\n${log.slice(-3000)}`);
  }
  throw e;
} finally {
  let stopped = 0;
  for (const p of posts.reverse()) {
    const r = await runStep(dir, `post-${stopped++}`, p).catch((e) => {
      console.error(String(e));
      return undefined;
    });
    void r;
  }
  tcpEcho.close();
  udpEcho.close();
  await new Promise((r) => setTimeout(r, 300));
}

const left = spawnSync('pgrep', ['-fl', 'iroh-tunnel-']).stdout.toString().trim();
assert(left === '', `iroh-tunnel processes remain:\n${left}`);
for (const p of posts) assert((await stat(p.state!.workspace!).catch(() => null)) === null, 'workspace not removed');
await rm(dir, { recursive: true, force: true });
console.log('SMOKE OK: round trips equal, both posts ran, no iroh-tunnel- process left');
