import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {BuildFixture, repoRoot} from './build-inputs-fixture.mjs';
import {runPrebuiltCopy} from './prebuilt-ui-fixture.mjs';
import ts from 'typescript';

const documentation = ['README.md', 'README.zh-CN.md', 'docs/architecture/01-fixture.md', 'docs/architecture/01-fixture.zh-CN.md'];

test('shell fingerprint changes for every imported documentation surface', () => {
  const unchanged = [];
  for (const file of documentation) {
    const fixture = new BuildFixture();
    try {
      const before = fixture.shellFingerprint();
      fixture.write(file, '# Updated fixture\n');
      if (before === fixture.shellFingerprint()) unchanged.push(file);
    } finally { fixture.dispose(); }
  }
  assert.deepEqual(unchanged, []);
});

test('Vite UI version changes for every imported documentation surface', async () => {
  const unchanged = [];
  for (const file of documentation) {
    const fixture = new BuildFixture();
    try {
      const before = await fixture.viteVersion();
      fixture.write(file, '# Updated fixture\n');
      if (before === await fixture.viteVersion()) unchanged.push(file);
    } finally { fixture.dispose(); }
  }
  assert.deepEqual(unchanged, []);
});

test('documentation glob additions and deletions invalidate both fingerprints', async () => {
  const fixture = new BuildFixture();
  try {
    const shell = fixture.shellFingerprint();
    const vite = await fixture.viteVersion();
    fixture.write('docs/architecture/02-new.zh-CN.md', '# New fixture\n');
    assert.notEqual(fixture.shellFingerprint(), shell);
    assert.notEqual(await fixture.viteVersion(), vite);
    fs.unlinkSync(path.join(fixture.root, 'docs/architecture/02-new.zh-CN.md'));
    assert.equal(fixture.shellFingerprint(), shell);
    assert.equal(await fixture.viteVersion(), vite);
    fs.unlinkSync(path.join(fixture.root, 'docs/architecture/01-fixture.md'));
    assert.notEqual(fixture.shellFingerprint(), shell);
    assert.notEqual(await fixture.viteVersion(), vite);
  } finally { fixture.dispose(); }
});

test('unimported documentation does not invalidate either fingerprint', async () => {
  const fixture = new BuildFixture();
  try {
    const shell = fixture.shellFingerprint();
    const vite = await fixture.viteVersion();
    for (const file of ['docs/architecture/README.md', 'docs/architecture/1-ignored.md', 'docs/architecture/nested/01-ignored.md', 'docs/unrelated.md']) {
      fixture.write(file, '# Not imported\n');
    }
    assert.equal(fixture.shellFingerprint(), shell);
    assert.equal(await fixture.viteVersion(), vite);
  } finally { fixture.dispose(); }
});

test('unchanged input and UI source/public mutations retain their existing behavior', async () => {
  const fixture = new BuildFixture();
  try {
    const shell = fixture.shellFingerprint();
    const vite = await fixture.viteVersion();
    assert.equal(fixture.shellFingerprint(), shell);
    assert.equal(await fixture.viteVersion(), vite);
    fixture.write('UI/public/fixture.txt', 'changed');
    assert.notEqual(fixture.shellFingerprint(), shell);
    assert.notEqual(await fixture.viteVersion(), vite);
    const publicChanged = fixture.shellFingerprint();
    fixture.write('UI/src/fixture.ts', 'export const fixture = 1;\n');
    assert.notEqual(fixture.shellFingerprint(), publicChanged);
    assert.notEqual(await fixture.viteVersion(), publicChanged.slice(0, 12));
  } finally { fixture.dispose(); }
});

test('selected product identity participates without a path-dependent namespace', async () => {
  const fixture = new BuildFixture();
  try {
    const shell = fixture.shellFingerprint();
    const vite = await fixture.viteVersion();
    fixture.write('configs/product_identity.toml', fs.readFileSync(path.join(repoRoot, 'scripts/fixtures/product_identity/brand-alternate.toml')));
    assert.notEqual(fixture.shellFingerprint(), shell);
    assert.notEqual(await fixture.viteVersion(), vite);
  } finally { fixture.dispose(); }
});

test('shell cache and Vite UI version use the same complete fingerprint', async () => {
  const fixture = new BuildFixture();
  try {
    assert.equal(await fixture.viteVersion(), fixture.shellFingerprint().slice(0, 12));
    fixture.write('README.md', '# Changed\n');
    assert.equal(await fixture.viteVersion(), fixture.shellFingerprint().slice(0, 12));
  } finally { fixture.dispose(); }
});

test('identical workspaces and selected-identity paths do not change fingerprints', async () => {
  const first = new BuildFixture();
  const second = new BuildFixture();
  try {
    assert.equal(first.shellFingerprint(), second.shellFingerprint());
    assert.equal(await first.viteVersion(), await second.viteVersion());
    const before = second.shellFingerprint();
    second.write('configs/same-identity.toml', fs.readFileSync(second.identity));
    second.identity = path.join(second.root, 'configs/same-identity.toml');
    assert.equal(second.shellFingerprint(), before);
    assert.equal(await second.viteVersion(), before.slice(0, 12));
  } finally { first.dispose(); second.dispose(); }
});

test('renaming a numbered document invalidates both fingerprints even with identical bytes', async () => {
  const fixture = new BuildFixture();
  try {
    const before = fixture.shellFingerprint();
    const vite = await fixture.viteVersion();
    fs.renameSync(path.join(fixture.root, 'docs/architecture/01-fixture.md'), path.join(fixture.root, 'docs/architecture/03-fixture.md'));
    assert.notEqual(fixture.shellFingerprint(), before);
    assert.notEqual(await fixture.viteVersion(), vite);
  } finally { fixture.dispose(); }
});

test('build configuration files participate in both consumers', async () => {
  const fixture = new BuildFixture();
  try {
    const before = fixture.shellFingerprint();
    const vite = await fixture.viteVersion();
    fixture.write('UI/postcss.config.cjs', 'module.exports = {};\n');
    assert.notEqual(fixture.shellFingerprint(), before);
    assert.notEqual(await fixture.viteVersion(), vite);
  } finally { fixture.dispose(); }
});

test('missing selected identity never yields a fallback fingerprint', async () => {
  const fixture = new BuildFixture();
  try {
    fs.unlinkSync(fixture.identity);
    assert.throws(() => fixture.shellFingerprint());
    await assert.rejects(() => fixture.viteVersion());
  } finally { fixture.dispose(); }
});

test('learning-page raw documentation imports remain inside the audited build-input contract', () => {
  const sourcePath = path.join(repoRoot, 'UI/src/components/AiLearningPage.tsx');
  const source = ts.createSourceFile(sourcePath, fs.readFileSync(sourcePath, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const readmes = [];
  const globs = [];
  const relative = value => path.posix.normalize(path.posix.join('src/components', value));
  const visit = node => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) {
      const value = node.moduleSpecifier.text;
      const [file, query] = value.split('?', 2);
      if (file.endsWith('.md')) {
        assert.equal(query, 'raw');
        readmes.push(relative(file));
      }
    }
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
      && node.expression.name.text === 'glob' && ts.isMetaProperty(node.expression.expression)) {
      assert.ok(ts.isStringLiteral(node.arguments[0]), 'document_glob_shape_requires_audit');
      globs.push(relative(node.arguments[0].text));
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.deepEqual(readmes.sort(), ['../README.md', '../README.zh-CN.md']);
  assert.deepEqual(globs, ['../docs/architecture/[0-9][0-9]-*.md']);
});

test('prebuilt UI copy with no configured nginx does not require Node, npm or source inputs', () => {
  const fixture = new BuildFixture();
  try { runPrebuiltCopy(fixture, false); }
  finally { fixture.dispose(); }
});

test('configured prebuilt UI copy avoids Node and uses only fixture nginx paths and commands', () => {
  const fixture = new BuildFixture();
  try { runPrebuiltCopy(fixture, true); }
  finally { fixture.dispose(); }
});
