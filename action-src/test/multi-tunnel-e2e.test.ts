import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dir, '../..');
const workflow = readFileSync(resolve(root, '.github/workflows/e2e.yml'), 'utf8');
type Step = { id?: string; name?: string; uses?: string; if?: string; 'continue-on-error'?: boolean; with?: Record<string, string>; env?: Record<string, string>; run?: string };
type Workflow = { jobs: { e2e: { if?: string; strategy: { matrix: { include: { name: string; os: string }[]; exclude?: unknown[] } }; steps: Step[] } } };

function checkWorkflow(doc: Workflow) {
  const job = doc.jobs.e2e;
  expect(job.if).toBeUndefined();
  expect(job.strategy.matrix.exclude ?? []).toEqual([]);
  expect(job.strategy.matrix.include).toEqual([
    { name: 'linux', os: 'ubuntu-latest' },
    { name: 'macos', os: 'macos-15' },
    { name: 'windows', os: 'windows-latest' },
  ]);
  const steps = job.steps;
  const byId = (id: string) => {
    const matches = steps.filter((s) => s.id === id);
    expect(matches).toHaveLength(1);
    const step = matches[0]!;
    expect(step.if).toBeUndefined();
    expect(step['continue-on-error']).not.toBe(true);
    return step;
  };
  const server = byId('echo_servers');
  expect(server.run).toContain('echo-server.ts 18080 target-one');
  expect(server.run).toContain('echo-server.ts 18081 target-two');
  expect(server.run).toContain('wait-port.ts 127.0.0.1:18080');
  expect(server.run).toContain('wait-port.ts 127.0.0.1:18081');
  const serve1 = byId('serve1');
  const serve2 = byId('serve2');
  const access1 = byId('access1');
  const access2 = byId('access2');
  expect(serve1.uses).toBe('./serve');
  expect(serve2.uses).toBe('./serve');
  expect(serve1.with).toEqual({ name: 'echo1', target: '127.0.0.1:18080' });
  expect(serve2.with).toEqual({ name: 'echo2', target: '127.0.0.1:18081' });
  expect(access1.uses).toBe('./');
  expect(access2.uses).toBe('./access');
  expect(access1.with).toEqual({ name: 'echo1', 'node-id': '${{ steps.serve1.outputs.node-id }}', listen: '127.0.0.1:19080' });
  expect(access2.with).toEqual({ name: 'echo2', 'node-id': '${{ steps.serve2.outputs.node-id }}', listen: '127.0.0.1:19081' });
  const verify = byId('verify_tunnels');
  expect(verify.env).toEqual({
    SERVE1: '${{ steps.serve1.outputs.node-id }}', SERVE2: '${{ steps.serve2.outputs.node-id }}',
    ACCESS1: '${{ steps.access1.outputs.node-id }}', ACCESS2: '${{ steps.access2.outputs.node-id }}',
  });
  expect(verify.run).toContain('set -euo pipefail');
  expect(verify.run).toContain('node_ids=("$SERVE1" "$SERVE2" "$ACCESS1" "$ACCESS2")');
  expect(verify.run).toContain('for ((i=0; i<4; i++)); do');
  expect(verify.run).toContain('for ((j=i+1; j<4; j++)); do');
  expect(verify.run).toContain('[ -n "${node_ids[$i]}" ]');
  expect(verify.run).toContain('[ "${node_ids[$i]}" != "${node_ids[$j]}" ]');
  expect(verify.run).toContain('roundtrip.ts 127.0.0.1:19080 "root-$RUNNER_OS" target-one');
  expect(verify.run).toContain('roundtrip.ts 127.0.0.1:19081 "alias-$RUNNER_OS" target-two');
  expect(steps.indexOf(server)).toBeLessThan(steps.indexOf(serve1));
  expect(steps.indexOf(server)).toBeLessThan(steps.indexOf(serve2));
  expect(steps.indexOf(serve1)).toBeLessThan(steps.indexOf(access1));
  expect(steps.indexOf(serve2)).toBeLessThan(steps.indexOf(access2));
  for (const step of [serve1, serve2, access1, access2]) expect(steps.indexOf(step)).toBeLessThan(steps.indexOf(verify));
}

const parseWorkflow = () => Bun.YAML.parse(workflow) as Workflow;

describe('multi-tunnel e2e', () => {
  test('workflow wires both tunnel pairs across all platforms', () => {
    checkWorkflow(parseWorkflow());
  });

  test('workflow contract rejects missing steps, inputs and Windows exclusion', () => {
    const missing = parseWorkflow();
    missing.jobs.e2e.steps = missing.jobs.e2e.steps.filter((s) => s.id !== 'access2');
    expect(() => checkWorkflow(missing)).toThrow();
    const noNodeId = parseWorkflow();
    delete noNodeId.jobs.e2e.steps.find((s) => s.id === 'access1')!.with!['node-id'];
    expect(() => checkWorkflow(noNodeId)).toThrow();
    const windowsExcluded = parseWorkflow();
    windowsExcluded.jobs.e2e.steps.find((s) => s.id === 'access2')!.if = "matrix.name != 'windows'";
    expect(() => checkWorkflow(windowsExcluded)).toThrow();
    const noWindows = parseWorkflow();
    noWindows.jobs.e2e.strategy.matrix.include = noWindows.jobs.e2e.strategy.matrix.include.filter((m) => m.name !== 'windows');
    expect(() => checkWorkflow(noWindows)).toThrow();
    const missingOutput = parseWorkflow();
    delete missingOutput.jobs.e2e.steps.find((s) => s.id === 'verify_tunnels')!.env!.ACCESS2;
    expect(() => checkWorkflow(missingOutput)).toThrow();
  });

  test('detects a round trip to the wrong target', async () => {
    const server = Bun.spawn([process.execPath, resolve(root, 'action-src/scripts/echo-server.ts'), '0', 'target-two'], { stdout: 'pipe', stderr: 'pipe' });
    try {
      const reader = server.stdout.getReader();
      const { value } = await reader.read();
      reader.releaseLock();
      const output = new TextDecoder().decode(value);
      const port = output.match(/127\.0\.0\.1:(\d+)/)?.[1];
      expect(port).toBeDefined();
      const run = async (target: string) => {
        const proc = Bun.spawn([process.execPath, resolve(root, 'action-src/scripts/roundtrip.ts'), `127.0.0.1:${port}`, 'probe', target], { stdout: 'pipe', stderr: 'pipe' });
        return { code: await proc.exited, stderr: await new Response(proc.stderr).text() };
      };
      expect((await run('target-two')).code).toBe(0);
      const wrong = await run('target-one');
      expect(wrong.code).not.toBe(0);
      expect(wrong.stderr).toContain('target-one');
      expect(wrong.stderr).toContain('target-two');
    } finally {
      server.kill();
      await server.exited;
    }
  });
});
