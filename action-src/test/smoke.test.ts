import { expect, test } from 'bun:test';
import { BUILD_VERSION } from '../src/version';

test('BUILD_VERSION is a string', () => {
  expect(BUILD_VERSION).toBeString();
});
