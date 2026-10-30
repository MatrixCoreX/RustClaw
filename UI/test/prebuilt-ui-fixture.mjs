import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {repoRoot} from './build-inputs-fixture.mjs';

export function runPrebuiltCopy(fixture, configured) {
  const bin = path.join(fixture.root, 'bin');
  const brew = path.join(fixture.root, 'brew');
  const destination = path.join(fixture.root, 'deployed');
  const calls = path.join(fixture.root, 'commands.log');
  fs.mkdirSync(bin);
  const executable = (name, content) => {
    fs.writeFileSync(path.join(bin, name), `#!/bin/sh\n${content}\n`, {mode: 0o755});
  };
  executable('uname', 'if [ "$1" = "-s" ]; then echo Darwin; else echo x86_64; fi');
  executable('sw_vers', 'echo 13.0');
  executable('brew', 'if [ "$1" = "--prefix" ]; then printf "%s\\n" "$FIXTURE_BREW"; elif [ "$1 $2 $3" = "services restart nginx" ]; then echo brew_restart >> "$FIXTURE_CALLS"; else exit 97; fi');
  executable('nginx', 'echo nginx_check >> "$FIXTURE_CALLS"');
  for (const name of ['node', 'npm', 'sudo']) {
    executable(name, `echo forbidden_${name} >> "$FIXTURE_CALLS"; exit 97`);
  }
  fixture.write('safe-discovery.sh', `command() {
  case "$*" in
    "-v systemctl"|"-v service"|"-v rc-service"|"-v apt-get"|"-v dnf"|"-v yum") return 1 ;;
    *) builtin command "$@" ;;
  esac
}\n`);
  for (const name of ['build-ui-nginx.sh', 'scripts/shell_compat.sh', 'scripts/product_identity.sh']) {
    fixture.write(name, fs.readFileSync(path.join(repoRoot, name)));
  }
  fs.unlinkSync(path.join(fixture.ui, 'node_modules'));
  for (const name of ['src', 'public', 'package.json', 'package-lock.json', 'vite.config.ts', 'build-inputs.mjs', 'build-fingerprint.mjs']) {
    fs.rmSync(path.join(fixture.ui, name), {recursive: true, force: true});
  }
  fixture.write('UI/dist/index.html', 'fixture-prebuilt');
  fs.mkdirSync(path.join(brew, 'etc/nginx/servers'), {recursive: true});
  fixture.write('brew/etc/nginx/nginx.conf', 'events {}\nhttp {\n}\n');
  if (configured) {
    const generated = fixture.helpFunction('printf "CONFIG_BEGIN\\n"; write_nginx_config "unused" "$FINGERPRINT_FIXTURE/deployed" "http://127.0.0.1:8788"; printf "CONFIG_END\\n"');
    const content = generated.split('CONFIG_BEGIN\n')[1]?.split('CONFIG_END\n')[0];
    assert.ok(content);
    fixture.write('brew/etc/nginx/servers/agent-runtime-ui.conf', content);
  }
  const env = {...fixture.environment(), PATH: `${bin}${path.delimiter}${process.env.PATH}`,
    HOME: path.join(fixture.root, 'home'), CARGO_HOME: path.join(fixture.root, 'cargo'),
    BASH_ENV: path.join(fixture.root, 'safe-discovery.sh'), FIXTURE_BREW: brew, FIXTURE_CALLS: calls};
  execFileSync('bash', [path.join(fixture.root, 'build-ui-nginx.sh'), '--copy-if-configured'],
    {env, encoding: 'utf8', stdio: 'pipe', timeout: 30_000});
  const log = fs.existsSync(calls) ? fs.readFileSync(calls, 'utf8') : '';
  assert.ok(!log.includes('forbidden_'), log);
  if (configured) {
    assert.equal(fs.readFileSync(path.join(destination, 'index.html'), 'utf8'), 'fixture-prebuilt');
    assert.ok(log.includes('brew_restart'));
  } else {
    assert.equal(fs.existsSync(destination), false);
    assert.equal(log, '');
  }
}
