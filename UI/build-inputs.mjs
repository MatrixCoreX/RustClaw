import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const fixedFiles = [
  'package.json', 'package-lock.json', 'index.html',
  'tsconfig.json', 'tsconfig.app.json', 'tsconfig.node.json',
  'vite.config.ts', 'vite.config.js', 'tailwind.config.js', 'tailwind.config.ts',
  'postcss.config.js', 'postcss.config.cjs', 'build-inputs.mjs', 'build-fingerprint.mjs',
];

/** @param {string} uiDirectory @param {string} identityConfigPath */
export function computeUiBuildFingerprint(uiDirectory, identityConfigPath) {
  const uiRoot = path.resolve(uiDirectory);
  const files = new Map();
  const add = absolute => {
    if (fs.existsSync(absolute) && fs.statSync(absolute).isFile()) {
      files.set(path.relative(uiRoot, absolute).split(path.sep).join('/'), absolute);
    }
  };
  for (const relative of fixedFiles) add(path.join(uiRoot, relative));
  for (const directory of ['src', 'public']) {
    const pending = [path.join(uiRoot, directory)];
    while (pending.length > 0) {
      const current = pending.pop();
      if (!fs.existsSync(current)) continue;
      for (const entry of fs.readdirSync(current, {withFileTypes: true})) {
        const absolute = path.join(current, entry.name);
        if (entry.isDirectory()) pending.push(absolute);
        else if (entry.isFile()) add(absolute);
      }
    }
  }
  for (const relative of ['../README.md', '../README.zh-CN.md']) {
    add(path.resolve(uiRoot, relative));
  }
  const architecture = path.resolve(uiRoot, '../docs/architecture');
  if (fs.existsSync(architecture)) {
    for (const name of fs.readdirSync(architecture)) {
      if (/^[0-9]{2}-.*\.md$/.test(name)) add(path.join(architecture, name));
    }
  }
  // A selected identity contributes its bytes, never its machine-local pathname.
  files.set('product_identity.toml', path.resolve(identityConfigPath));
  const digest = crypto.createHash('sha256').update('ui-build-inputs-v1\0');
  for (const label of [...files.keys()].sort()) {
    digest.update(label).update('\0').update(fs.readFileSync(files.get(label))).update('\0');
  }
  return digest.digest('hex');
}
