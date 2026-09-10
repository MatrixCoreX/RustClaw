import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { normalizeWebdCsrfToken } from '../../UI/src/lib/webd-csrf';

test('desktop login regression cases agree with the real browser and server CSRF contract', () => {
  const cases = JSON.parse(readFileSync(new URL('./fixtures/webd-csrf.json', import.meta.url), 'utf8'));
  const server = readFileSync(new URL('../../crates/webd/src/main.rs', import.meta.url), 'utf8');
  const size = Number(/const WEBD_CSRF_TOKEN_HEX_BYTES:\s*usize\s*=\s*(\d+)/.exec(server)![1]);
  for (const {token, valid} of cases) {
    assert.equal(normalizeWebdCsrfToken(token) !== null, valid);
    if (valid) assert.equal(token.length, size);
  }
});
