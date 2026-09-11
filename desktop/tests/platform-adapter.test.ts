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

test('shared account pages preserve layout while local transfer uses native confirmation', () => {
  const transform = sharedUiAdapter(root).transform as (source: string, id: string) => {code: string} | undefined;
  for (const name of ['AssetsPage', 'BancorPage', 'AssetTransferDialog']) {
    const filename = path.resolve(root, `../UI/src/components/${name}.tsx`);
    const source = fs.readFileSync(filename, 'utf8');
    for (const input of [source, source.replaceAll('\n', '\r\n')]) {
      const result = transform(input, filename)?.code ?? '';
      assert.ok(result.includes('useDesktopAssetAccount'));
      if (name === 'AssetTransferDialog') {
        assert.ok(result.includes('if (desktopAccount) onClose(); else setCompleted(true);'));
        assert.ok(result.includes('!desktopAccount && nniPrivateKeyOperationsAllowed()'));
        assert.ok(result.includes('NativeTransferAuthorization'));
      } else {
        assert.ok(result.includes('LocalAccountHistory'));
        assert.equal((result.match(/<AccountSelector /g) ?? []).length, 1);
      }
    }
  }
});
