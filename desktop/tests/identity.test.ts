import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
test('desktop menu projects selected identity while package, launcher and vault identity stay stable', () => {
  const names = [];
  try {
    for (const fixture of ['brand-primary.toml', 'brand-alternate.toml']) {
      const config = path.resolve(root, '../scripts/fixtures/product_identity', fixture);
      const expected = /^display_name\s*=\s*"([^"]+)"/m.exec(fs.readFileSync(config, 'utf8'))![1];
      const result = spawnSync(process.execPath, ['scripts/build.mjs', '--prepare-only'], {cwd:root, env:{...process.env, APP_PRODUCT_IDENTITY_CONFIG:config}, encoding:'utf8'});
      assert.equal(result.status,0,result.stderr);
      const metadata = JSON.parse(fs.readFileSync(path.join(root,'.build/identity.json'),'utf8'));
      assert.equal(metadata.productName,'agent-desktop');
      const plist = fs.readFileSync(metadata.bundle.macOS.infoPlist, 'utf8');
      assert.ok(plist.includes(`<key>CFBundleDisplayName</key><string>${expected}</string>`));
      assert.ok(plist.includes('_agent-runtime._tcp'));
      const entry = fs.readFileSync(metadata.bundle.linux.deb.desktopTemplate,'utf8');
      assert.ok(entry.includes(`Name=${expected}\n`));assert.ok(entry.includes('Exec=agent-desktop\n'));
      names.push(expected);
    }
    assert.notEqual(names[0],names[1]);
    const bad = spawnSync(process.execPath,['scripts/build.mjs','--prepare-only'],{cwd:root,env:{...process.env,APP_PRODUCT_IDENTITY_CONFIG:path.join(root,'.build/missing-identity.toml')},encoding:'utf8'});
    assert.notEqual(bad.status,0);
  } finally {
    const env = {...process.env}; delete env.APP_PRODUCT_IDENTITY_CONFIG;
    spawnSync(process.execPath,['scripts/build.mjs','--prepare-only'],{cwd:root,env});
  }
});
