"""Refresh only the independent HTTPS config, retaining its certificate and HTTP service."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import platform
import re
import ssl
import subprocess
import time
from urllib.parse import urlsplit
from templates import nginx_config


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser()
    for name in ['state', 'nginx', 'mime-types', 'ui-root', 'expected-config-sha256', 'source-commit']:
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        raise SystemExit('platform_unsupported: macos_required')
    source = Path(__file__).resolve().parent
    if not re.fullmatch('[a-f0-9]{40}', args.source_commit) or (source / 'source-commit.txt').read_text().strip() != args.source_commit:
        raise SystemExit('source_commit_mismatch')
    state = Path(args.state).resolve()
    config_path = state / 'nginx.conf'
    original = config_path.read_bytes()
    if hashlib.sha256(original).hexdigest() != args.expected_config_sha256:
        raise SystemExit('existing_configuration_changed')
    pid = int((state / 'nginx.pid').read_text())
    command = run(['/bin/ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout
    if str(config_path) not in command:
        raise SystemExit('independent_https_process_required')
    deployment = json.loads((state / 'deployment.json').read_text())
    url = urlsplit(deployment['https_origin'])
    config = nginx_config(state, Path(args.ui_root).resolve(), Path(args.mime_types).resolve(),
                          url.hostname, deployment['hostname'], deployment['lan_allow'], url.port)
    os.umask(0o077)
    backup = state / ('nginx.conf.before-' + args.expected_config_sha256)
    with backup.open('xb') as file:
        file.write(original)
    candidate = state / ('nginx.conf.candidate-' + args.source_commit)
    with candidate.open('x') as file:
        file.write(config)
    run([args.nginx, '-t', '-p', str(state) + '/', '-c', candidate])
    os.replace(candidate, config_path)
    try:
        run([args.nginx, '-p', str(state) + '/', '-c', config_path, '-s', 'reload'])
        context = ssl.create_default_context(cafile=str(state / 'ca.crt'))
        for _ in range(20):
            conn = http.client.HTTPSConnection(url.hostname, url.port, context=context, timeout=3)
            try:
                conn.request('GET', '/webd/session', headers={'Origin': deployment['https_origin']})
                response = conn.getresponse()
                payload = response.read(4097)
                if response.status == 200 and len(payload) <= 4096 and json.loads(payload).get('ok') is True:
                    break
            finally:
                conn.close()
            time.sleep(.2)
        else:
            raise RuntimeError('https_origin_validation_failed')
    except Exception:
        candidate.write_bytes(original)
        os.replace(candidate, config_path)
        run([args.nginx, '-p', str(state) + '/', '-c', config_path, '-s', 'reload'])
        raise
    deployment.setdefault('initial_source_commit', deployment['source_commit'])
    deployment.update(source_commit=args.source_commit, previous_config_sha256=args.expected_config_sha256,
                      config_sha256=hashlib.sha256(config.encode()).hexdigest(), origin_with_port_verified=True)
    (state / 'deployment.json').write_text(json.dumps(deployment, indent=2) + '\n')
    print(json.dumps(deployment, indent=2))


if __name__ == '__main__':
    main()
