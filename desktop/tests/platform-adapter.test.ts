import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { sharedUiAdapter } from '../scripts/shared-ui-adapter';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
test('shared UI adaptation is active for native and Vite-normalized Windows paths', () => {
  const plugin = sharedUiAdapter(root);
  const transform = plugin.transform as (source: string, id: string) => {code: string} | undefined;
  const filename = path.resolve(root, '../UI/src/App.tsx');
  const source = fs.readFileSync(filename, 'utf8');
  for (const id of [filename, filename.replaceAll('\\', '/')]) {
    const output = transform(source.replaceAll(/\r?\n/g, '\r\n'), id)?.code;
    assert.ok(output?.includes('desktopFetch as fetch'));
    assert.ok(output?.includes('desktopLogout(); return;'));
    assert.ok(!output?.includes('window.localStorage'));
  }
});
