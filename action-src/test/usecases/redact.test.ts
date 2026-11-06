import { expect, test } from 'bun:test';
import { redact } from '../../src/usecases/redact';

test('redacts secret_key assignments in any spacing', () => {
  expect(redact('a\nsecret_key = "abc=="\nb', [])).toBe('a\nsecret_key = "***"\nb');
  expect(redact('secret_key="x"', [])).toBe('secret_key = "***"');
});

test('redacts relay_token assignments', () => {
  expect(redact('relay_token = "t"', [])).toBe('relay_token = "***"');
});

test('redacts registered secret values, all occurrences', () => {
  expect(redact('tok-123 and tok-123', ['tok-123'])).toBe('*** and ***');
});

test('ignores empty secrets and leaves other text alone', () => {
  expect(redact('plain', [''])).toBe('plain');
});

test('redacts quoted, single-quoted, unquoted and colon forms', () => {
  for (const v of ['"v1"', "'v1'", 'v1']) {
    for (const sep of ['=', ' = ', ': ', ':']) {
      expect(redact(`relay_token${sep}${v}`, [])).toBe('relay_token = "***"');
      expect(redact(`secret_key${sep}${v}`, [])).toBe('secret_key = "***"');
    }
  }
});
