import { expect, test } from 'bun:test';
import { createIO, type CoreLike } from '../../src/adapters/io';

function fakeCore(log: string[], state: Record<string, string> = {}, inputs: Record<string, string> = {}): CoreLike {
  return {
    getInput: (n) => inputs[n] ?? '',
    setOutput: (k, v) => log.push(`output:${k}=${v}`),
    setSecret: (v) => log.push(`secret:${v}`),
    saveState: (k, v) => { state[k] = v; log.push(`save:${k}`); },
    getState: (k) => state[k] ?? '',
    info: (m) => log.push(`info:${m}`),
    warning: (m) => log.push(`warn:${m}`),
    startGroup: (t) => log.push(`start:${t}`),
    endGroup: () => log.push('end'),
    setFailed: (m) => log.push(`fail:${m}`),
  };
}

test('setSecret is observed before a later info', () => {
  const log: string[] = [];
  const io = createIO(fakeCore(log));
  io.setSecret('tok');
  io.info('using tok');
  expect(log).toEqual(['secret:tok', 'info:using tok']);
});

test('group wraps output in startGroup/endGroup and returns the value', async () => {
  const log: string[] = [];
  const io = createIO(fakeCore(log));
  const v = await io.group('T', async () => { io.info('x'); return 7; });
  expect(v).toBe(7);
  expect(log).toEqual(['start:T', 'info:x', 'end']);
});

test('group ends even when fn throws', async () => {
  const log: string[] = [];
  const io = createIO(fakeCore(log));
  await expect(io.group('T', async () => { throw new Error('boom'); })).rejects.toThrow('boom');
  expect(log).toEqual(['start:T', 'end']);
});

test('jobFailed: job-status failure/cancelled or missing main-ok', () => {
  const mkio = (status: string, state: Record<string, string>) => createIO(fakeCore([], state, { 'job-status': status }));
  expect(mkio('success', { 'main-ok': '1' }).jobFailed()).toBe(false);
  expect(mkio('failure', { 'main-ok': '1' }).jobFailed()).toBe(true);
  expect(mkio('cancelled', { 'main-ok': '1' }).jobFailed()).toBe(true);
  expect(mkio('success', {}).jobFailed()).toBe(true);
});

test('input returns the trimmed core input', () => {
  expect(createIO(fakeCore([], {}, { name: 'x' })).input('name')).toBe('x');
});
