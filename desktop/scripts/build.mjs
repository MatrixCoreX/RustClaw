import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import {buildWindowsWorker} from './windows-worker.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const identityPath = process.env.APP_PRODUCT_IDENTITY_CONFIG
  ? path.resolve(process.env.APP_PRODUCT_IDENTITY_CONFIG) : path.resolve(root, '../configs/product_identity.toml');
// Reuse the repository's schema validator and projection. Arguments never pass through a shell.
const identity = spawnSync('cargo', ['run', '--quiet', '--locked', '--manifest-path', path.join(root, 'scripts/identity/Cargo.toml')], {
  cwd: path.resolve(root, '..'),
  env: {...process.env, APP_PRODUCT_IDENTITY_CONFIG: identityPath,
    CARGO_TARGET_DIR: path.resolve(process.env.CARGO_TARGET_DIR || path.join(root, 'target'), 'identity')}, encoding: 'utf8',
});
if (identity.status !== 0) { console.error(identity.stderr || identity.error); process.exit(1); }
const {display_name: name} = JSON.parse(identity.stdout);
fs.mkdirSync(path.resolve(root, '.build'), {recursive: true});
const configPath = path.resolve(root, '.build/identity.json');
const desktopTemplate = path.resolve(root, '.build/desktop-entry.desktop');
// Package and launcher identity are canonical; only the visible menu label is branded.
const displayName = name.replaceAll('\\', '\\\\').replaceAll('\n', '\\n').replaceAll('\r', '\\r');
fs.writeFileSync(desktopTemplate, `[Desktop Entry]\nType=Application\nName=${displayName}\nComment=Secure client for your agent devices\nExec=agent-desktop\nIcon=agent-desktop\nTerminal=false\nCategories=Utility;\nStartupWMClass=agent-desktop\n`);
const escapeXml = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;').replaceAll("'", '&apos;');
const infoPlist = path.join(root, '.build/Info.plist');
const plist = fs.readFileSync(path.join(root, 'platforms/macos/Info.plist'), 'utf8')
  .replace('<!-- PRODUCT_DISPLAY_NAME -->', `<key>CFBundleDisplayName</key><string>${escapeXml(name)}</string>`);
fs.writeFileSync(infoPlist, plist);
fs.writeFileSync(configPath, JSON.stringify({productName: 'agent-desktop', bundle:{
  linux:{deb:{desktopTemplate}}, macOS:{infoPlist},
}}));
if (process.argv.includes('--prepare-only')) process.exit(0);
const dev = process.argv.includes('--dev');
if(process.platform==='win32') {
  const config=JSON.parse(fs.readFileSync(configPath,'utf8'));
  config.bundle.externalBin=buildWindowsWorker(root,process.argv.slice(2));
  fs.writeFileSync(configPath,JSON.stringify(config));
}
const result = spawnSync(process.execPath, [path.resolve(root, 'node_modules/@tauri-apps/cli/tauri.js'), 'build', '--config', configPath, ...(dev ? ['--debug', '--no-bundle'] : []), ...process.argv.slice(2).filter(a => a !== '--dev')], {
  cwd: root, stdio: 'inherit', env: {...process.env, APP_PRODUCT_IDENTITY_CONFIG: identityPath},
});
if (result.error) console.error(result.error);
process.exit(result.status ?? 1);
