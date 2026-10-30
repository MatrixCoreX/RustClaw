import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {loadConfigFromFile} from 'vite';

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

export class BuildFixture {
  constructor() {
    this.root = fs.mkdtempSync(path.join(os.tmpdir(), 'ui build inputs '));
    this.ui = path.join(this.root, 'UI');
    this.identity = path.join(this.root, 'configs/product_identity.toml');
    this.write('UI/src/components/AiLearningPage.tsx', fs.readFileSync(path.join(repoRoot, 'UI/src/components/AiLearningPage.tsx')));
    this.write('UI/public/fixture.txt', 'public');
    for (const name of ['package.json', 'package-lock.json', 'index.html', 'tsconfig.json', 'vite.config.ts', 'build-inputs.mjs', 'build-fingerprint.mjs']) {
      const source = path.join(repoRoot, 'UI', name);
      if (fs.existsSync(source)) this.write(`UI/${name}`, fs.readFileSync(source));
    }
    for (const name of ['README.md', 'README.zh-CN.md', 'docs/architecture/01-fixture.md', 'docs/architecture/01-fixture.zh-CN.md']) {
      this.write(name, '# Fixture\n\nBefore.\n');
    }
    this.write('configs/product_identity.toml', fs.readFileSync(path.join(repoRoot, 'scripts/fixtures/product_identity/brand-primary.toml')));
    fs.symlinkSync(path.join(repoRoot, 'UI/node_modules'), path.join(this.ui, 'node_modules'), 'dir');
  }

  write(relative, bytes) {
    const target = path.join(this.root, relative);
    fs.mkdirSync(path.dirname(target), {recursive: true});
    fs.writeFileSync(target, bytes);
  }

  dispose() { fs.rmSync(this.root, {recursive: true, force: true}); }

  environment() {
    return {...process.env, APP_PRODUCT_IDENTITY_CONFIG: this.identity, FINGERPRINT_FIXTURE: this.root,
      FINGERPRINT_SOURCE: path.join(repoRoot, 'build-ui-nginx.sh')};
  }

  helpFunction(body) {
    const code = `trap '${body}' EXIT\nsource "$FINGERPRINT_SOURCE" --help\n`;
    return execFileSync('bash', ['-c', code], {env: this.environment(), encoding: 'utf8', stdio: 'pipe', timeout: 30_000});
  }

  shellFingerprint() {
    const output = this.helpFunction('SCRIPT_DIR="$FINGERPRINT_FIXTURE"; UI_DIR="$SCRIPT_DIR/UI"; printf "FINGERPRINT=%s\\n" "$(compute_ui_build_fingerprint)"');
    const matches = output.split('\n').filter(line => line.startsWith('FINGERPRINT='));
    if (matches.length !== 1 || !/^FINGERPRINT=[a-f0-9]{64}$/.test(matches[0])) {
      throw new Error('fixture_fingerprint_result_invalid');
    }
    return matches[0].slice('FINGERPRINT='.length);
  }

  async viteVersion() {
    const previousCwd = process.cwd();
    const previousIdentity = process.env.APP_PRODUCT_IDENTITY_CONFIG;
    try {
      process.chdir(this.ui);
      process.env.APP_PRODUCT_IDENTITY_CONFIG = this.identity;
      const loaded = await loadConfigFromFile({command: 'build', mode: 'production'}, path.join(this.ui, 'vite.config.ts'), this.ui, 'silent');
      if (!loaded) throw new Error('fixture_vite_config_missing');
      return JSON.parse(loaded.config.define.__APP_UI_VERSION__);
    } finally {
      process.chdir(previousCwd);
      if (previousIdentity === undefined) delete process.env.APP_PRODUCT_IDENTITY_CONFIG;
      else process.env.APP_PRODUCT_IDENTITY_CONFIG = previousIdentity;
    }
  }
}
