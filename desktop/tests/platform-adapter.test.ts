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
    assert.ok(output?.includes('desktopFetch as runtimeApiFetch'));
    assert.ok(!output?.includes('from "./lib/runtime-api-transport"'));
    assert.equal((source.match(/\bruntimeApiFetch\(/g) ?? []).length, 7);
    assert.equal((output?.match(/\bfetch\(/g) ?? []).length, 0);
    assert.ok(output?.includes('desktopLogout(); return;'));
    assert.ok(output?.includes('/frontend/wallet/pages.tsx'));
    assert.ok(!output?.includes('import("./components/AssetsPage")'));
    assert.ok(!output?.includes('import("./components/BancorPage")'));
    assert.ok(!output?.includes('window.localStorage'));
  }
});

test('shared UI adaptation rejects a new raw fetch or an unreviewed transport call', () => {
  const transform = sharedUiAdapter(root).transform as (source: string, id: string) => {code: string} | undefined;
  const filename = path.resolve(root, '../UI/src/App.tsx');
  const source = fs.readFileSync(filename, 'utf8');
  for (const suffix of ['\nfetch("/v1/unreviewed");', '\nruntimeApiFetch("/v1/unreviewed");']) {
    assert.throws(() => transform(source + suffix, filename), /Shared UI fetch inventory changed/);
  }
});

test('shared account pages preserve layout while local transfer uses native confirmation', () => {
  const transform = sharedUiAdapter(root).transform as (source: string, id: string) => {code: string} | undefined;
  for (const name of ['AssetsPage', 'BancorPage', 'AssetTransferDialog']) {
    const filename = path.resolve(root, `../UI/src/components/${name}.tsx`);
    const source = fs.readFileSync(filename, 'utf8');
    // Git can already check out CRLF on Windows. Construct each line-ending
    // variant explicitly; replacing bare LF would create invalid CRCRLF input.
    for (const input of [source.replaceAll(/\r?\n/g, '\n'), source.replaceAll(/\r?\n/g, '\r\n')]) {
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

test('AiAPP downloads keep native delivery while previews keep the shared endpoint', () => {
  const transform = sharedUiAdapter(root).transform as (source: string, id: string) => {code: string} | undefined;
  const filename = path.resolve(root, '../UI/src/components/AippPage.tsx');
  const source = fs.readFileSync(filename, 'utf8');
  const output = transform(source, filename)?.code ?? '';
  assert.ok(output.includes('<DesktopAipp key={`${selectedApp.skill_name}:${requestScope}`}'));
  assert.throws(() => transform(source.replace('apiFetch={apiFetch} requestScope={requestScope} />', 'apiFetch={apiFetch} />'), filename), /Shared UI contract changed/);
  assert.ok(output.includes('if (!open) { await desktopDownload(artifact.download_url, artifact.filename); return; }'));
  assert.ok(output.includes('open && artifact.preview_url ? artifact.preview_url : artifact.download_url'));

  const viewerFilename = path.resolve(root, '../UI/src/components/AippImageViewer.tsx');
  const viewerSource = fs.readFileSync(viewerFilename, 'utf8');
  const viewerOutput = transform(viewerSource, viewerFilename)?.code ?? '';
  assert.ok(viewerOutput.includes('if (mode === "single") { await desktopDownload(image.downloadUrl, image.filename); setDownloadState("idle"); return; }'));
  assert.ok(viewerOutput.includes('await downloadAippGallery(gallery, apiFetchRef.current, controller.signal, setCompleted)'));
});
