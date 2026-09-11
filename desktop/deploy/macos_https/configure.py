"""Install a user-owned Mac HTTPS listener and Bonjour service from committed source."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import socket
import subprocess
import time
from templates import nginx_config, validate_network


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser()
    for name in ['address', 'hostname', 'subnet', 'nginx', 'openssl', 'mime-types', 'ui-root', 'state', 'source-commit']:
        parser.add_argument('--' + name, required=True)
    parser.add_argument("--port", type=int, default=8443)
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        raise SystemExit('platform_unsupported: macos_required')
    validate_network(args.address, args.hostname, args.subnet)
    if not re.fullmatch('[a-f0-9]{40}', args.source_commit):
        raise SystemExit('exact_source_commit_required')
    source = Path(__file__).resolve().parent
    if (source / 'source-commit.txt').read_text().strip() != args.source_commit:
        raise SystemExit('source_commit_mismatch')
    state = Path(args.state).resolve()
    ui = Path(args.ui_root).resolve()
    for executable in [args.nginx, args.openssl, '/usr/bin/dns-sd']:
        if not os.access(executable, os.X_OK):
            raise SystemExit('required_executable_missing: ' + executable)
    if not (ui / 'index.html').is_file() or not Path(args.mime_types).is_file():
        raise SystemExit('existing_ui_or_mime_types_missing')
    config = nginx_config(state, ui, Path(args.mime_types), args.address, args.hostname, args.subnet, args.port)
    labels = ['org.agent-runtime.desktop-https', 'org.agent-runtime.desktop-discovery']
    agents = Path.home() / 'Library/LaunchAgents'
    paths = [agents / (label + '.plist') for label in labels]
    if state.exists() or any(path.exists() for path in paths):
        raise SystemExit('existing_https_configuration_requires_review')
    # Check the exact LAN binding and current user's permission before creating files.
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind((args.address, args.port))
    os.umask(0o077)
    state.mkdir(parents=True, mode=0o700)
    for name in ['client', 'proxy', 'fastcgi', 'uwsgi', 'scgi']:
        (state / 'tmp' / name).mkdir(parents=True)
    def openssl(*values):
        run([args.openssl, *values], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    openssl('genpkey', '-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:3072', '-out', state / 'ca.key')
    openssl('req', '-new', '-x509', '-sha256', '-days', '3650', '-key', state / 'ca.key',
            '-out', state / 'ca.crt', '-subj', '/CN=LAN Device CA',
            '-addext', 'basicConstraints=critical,CA:TRUE,pathlen:0',
            '-addext', 'keyUsage=critical,keyCertSign,cRLSign')
    openssl('genpkey', '-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:3072', '-out', state / 'server.key')
    openssl('req', '-new', '-key', state / 'server.key', '-out', state / 'server.csr',
            '-subj', '/CN=' + args.hostname)
    extensions = state / 'server.ext'
    extensions.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n'
                          'extendedKeyUsage=serverAuth\nsubjectAltName=DNS:' + args.hostname
                          + ',IP:' + args.address + ',IP:127.0.0.1,IP:::1\n')
    openssl('x509', '-req', '-sha256', '-days', '397', '-in', state / 'server.csr',
            '-CA', state / 'ca.crt', '-CAkey', state / 'ca.key', '-CAcreateserial',
            '-out', state / 'server.crt', '-extfile', extensions)
    openssl('verify', '-CAfile', state / 'ca.crt', '-verify_ip', args.address, state / 'server.crt')
    config_path = state / 'nginx.conf'
    config_path.write_text(config)
    run([args.nginx, '-t', '-p', str(state) + '/', '-c', config_path])
    programs = [
        [args.nginx, '-p', str(state) + '/', '-c', str(config_path), '-g', 'daemon off;'],
        ['/usr/bin/dns-sd', '-R', args.hostname.removesuffix('.local'), '_agent-runtime._tcp', 'local.',
         str(args.port), 'txtvers=1', 'scheme=https', 'api=webd-v1', 'tls=private_ca'],
    ]
    agents.mkdir(parents=True, exist_ok=True)
    started = []
    try:
        for label, path, program in zip(labels, paths, programs):
            plist = {'Label': label, 'ProgramArguments': program, 'RunAtLoad': True, 'KeepAlive': True,
                     'WorkingDirectory': str(state), 'Umask': 0o077,
                     'StandardOutPath': str(state / (label + '.log')),
                     'StandardErrorPath': str(state / (label + '.error.log'))}
            path.write_bytes(plistlib.dumps(plist))
            run(['/bin/launchctl', 'bootstrap', 'gui/' + str(os.getuid()), path])
            started.append(label)
        for _ in range(30):
            try:
                with socket.create_connection((args.address, args.port), timeout=1):
                    break
            except OSError:
                time.sleep(.2)
        else:
            raise RuntimeError('https_listener_not_ready')
        for label in labels:
            run(['/bin/launchctl', 'print', 'gui/' + str(os.getuid()) + '/' + label], stdout=subprocess.DEVNULL)
    except Exception:
        for label in reversed(started):
            subprocess.run(['/bin/launchctl', 'bootout', 'gui/' + str(os.getuid()) + '/' + label], check=False)
        raise
    ca_der = run([args.openssl, 'x509', '-in', state / 'ca.crt', '-outform', 'DER'], capture_output=True).stdout
    evidence = {'source_commit': args.source_commit, 'https_origin': 'https://' + args.address + ':' + str(args.port),
                'hostname': args.hostname, 'lan_allow': args.subnet, 'state': str(state),
                'ca_sha256': hashlib.sha256(ca_der).hexdigest().upper(), 'launch_agents': labels,
                'http_configuration_changed': False, 'http_process_reloaded': False}
    (state / 'deployment.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps(evidence, indent=2))


if __name__ == '__main__':
    main()
