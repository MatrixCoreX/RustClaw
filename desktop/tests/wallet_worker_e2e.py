"""Exercise the packaged key worker with disposable files and OS credentials.
Run: dbus-run-session -- python3 wallet_worker_e2e.py /path/to/agent-desktop
No production account, network service or real transaction is used.
"""
import hashlib
import json
import os
from pathlib import Path
import select
import struct
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(os.environ.get('DESKTOP_TEST_OUTPUT_DIR', ROOT / 'test-results')) / ('worker-' + uuid.uuid4().hex[:8])
OUT.mkdir(parents=True)
(OUT / 'runtime').mkdir(mode=0o700)
os.environ['XDG_RUNTIME_DIR'] = str(OUT / 'runtime')
os.environ.pop('GNOME_KEYRING_CONTROL', None)
os.environ.update(XDG_DATA_HOME=str(OUT / 'data'), XDG_CONFIG_HOME=str(OUT / 'config'), XDG_CACHE_HOME=str(OUT / 'cache'))
assert os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'isolated D-Bus session required'
subprocess.run(['dbus-update-activation-environment', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_RUNTIME_DIR'], check=True)
subprocess.run(['gnome-keyring-daemon', '--unlock', '--components=secrets'], input=b'test-only-keyring-password', check=True, stdout=subprocess.DEVNULL)
BINARY = str(Path(sys.argv[1]).resolve())
PASSWORD = 'test-only-vault-password'
BACKUP_PASSWORD = 'Jasper!flume7-Pebble4-Orbit9-velvet'
checks = []
children = []

def passed(label):
    checks.append(label)
    print(label, flush=True)

def read_exact(stream, length):
    value = b''
    deadline = time.monotonic() + 60
    while len(value) < length:
        assert select.select([stream], [], [], max(0, deadline - time.monotonic()))[0], 'worker response timeout'
        part = os.read(stream.fileno(), length - len(value))
        assert part, 'worker closed response'
        value += part
    return value

class Worker:
    def __init__(self, directory):
        self.child = subprocess.Popen([BINARY, '--asset-vault-worker'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        children.append(self.child)
        self.sequence = 0
        self.directory = directory
        self.call('open', directory=str(directory))

    def send(self, value):
        data = json.dumps(value).encode()
        self.child.stdin.write(struct.pack('!I', len(data)) + data)

    def call(self, operation, error=None, **fields):
        self.sequence += 1
        self.send({'version': 1, 'id': self.sequence, 'request': {'operation': operation, **fields}})
        size = struct.unpack('!I', read_exact(self.child.stdout, 4))[0]
        assert 0 < size <= 131072
        data = read_exact(self.child.stdout, size)
        reply = json.loads(data)
        assert reply['version'] == 1 and reply['id'] == self.sequence
        assert PASSWORD.encode() not in data and BACKUP_PASSWORD.encode() not in data
        if error:
            assert reply['result'] == {'Err': error}, reply
            return
        assert 'Ok' in reply['result'], reply
        return reply['result']['Ok']

    def close(self):
        self.child.stdin.close()
        self.child.wait(timeout=5)
        assert self.child.stderr.read() == b'', 'worker must not log credentials'

try:
    worker = Worker(OUT / 'vault')
    state = worker.call('status')
    assert not state['initialized'] and state['storage_version'] == 2
    worker.call('initialize', password=PASSWORD)
    first = worker.call('create', name='first')
    second = worker.call('create', name='second')
    assert first['id'] != second['id'] and first['public_key'] != second['public_key']
    status_path = Path(f'/proc/{worker.child.pid}/status')
    status = dict(line.split(':', 1) for line in status_path.read_text().splitlines())
    assert status['NoNewPrivs'].strip() == '1' and status['Seccomp'].strip() == '2'
    assert int(status['VmLck'].split()[0]) > 0
    try:
        descriptor = os.open(f'/proc/{worker.child.pid}/mem', os.O_RDONLY)
    except PermissionError:
        pass
    else:
        os.close(descriptor)
        raise AssertionError('same-user memory reader must be denied')
    passed('packaged worker: OS keyring, separate process, seccomp, no-new-privileges, locked memory and same-user memory-read denial')
    vault_path = worker.directory / 'vault-v1.json'
    original = json.loads(vault_path.read_text())
    assert original['version'] == 2 and len(original['entries']) == 2
    assert all(len(entry['secret']['ciphertext']) == 96 for entry in original['entries'])
    backup = OUT / 'new.backup.json'
    worker.call('backup', id=first['id'], vault_password=PASSWORD, password='aaaaaaaaaaaaaaaa', path=str(backup), error='wallet_backup_password_weak')
    assert not backup.exists() and not worker.call('status')['unlocked']
    start = time.monotonic()
    worker.call('backup', id=first['id'], vault_password=PASSWORD, password=BACKUP_PASSWORD, path=str(backup))
    export_seconds = round(time.monotonic() - start, 3)
    assert not worker.call('status')['unlocked']
    current = json.loads(vault_path.read_text())
    assert current['entries'][1] == original['entries'][1], 'export must not rewrite another account'
    b = json.loads(backup.read_text())
    assert b['format'] == 'asset-account-backup-v2' and b['kdf']['memory_kib'] == 262144
    passed('v2 backup: weak-password rejection, selected-account export, native read-back verification and automatic lock')
    cap = dict(schema_version=1, protocol='asset_owner_v1', ledger_id='worker-test-ledger', node_url='https://ledger.example.test', service='assets', actions=['transfer'])
    intent = dict(kind='transfer', asset='AIC', amount_units='100000000', recipient=second['public_key'], memo='synthetic test only', max_fee_bps=0)
    payload = dict(schema_version=1, protocol='asset_owner_v1', ledger_id=cap['ledger_id'], node_url=cap['node_url'], service='assets', account=first['public_key'], operation_id=str(uuid.uuid4()), challenge_id=str(uuid.uuid4()), nonce='01' * 32, expires_at_unix=int(time.time()) + 120, terms={**intent, 'fee_units': '0'})
    raw = json.dumps(payload, separators=(',', ':'))
    worker.call('sign', id=first['id'], password=PASSWORD, payload=raw, cap=cap, intent={**intent, 'amount_units': '200000000'}, error='wallet_challenge_mismatch')
    signature = worker.call('sign', id=first['id'], password=PASSWORD, payload=raw, cap=cap, intent=intent)
    from wallet_fixture import public_bytes
    from cryptography.hazmat.primitives import hashes
    from cryptography.hazmat.primitives.asymmetric import ec, utils
    signature_bytes = bytes.fromhex(signature)
    assert len(signature_bytes) == 64
    public = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256K1(), public_bytes(first['public_key']))
    public.verify(utils.encode_dss_signature(int.from_bytes(signature_bytes[:32], 'big'), int.from_bytes(signature_bytes[32:], 'big')), raw.encode(), ec.ECDSA(hashes.SHA256()))
    assert not worker.call('status')['unlocked']
    assert (worker.call('sign', id=first['id'], password='wrong-password', payload=raw, cap=cap, intent=intent, error='wallet_unlock_failed')) is None
    passed('worker independently rejects mismatched transaction terms, signs exact bytes with the selected key, requires a fresh password and locks afterward; nothing is submitted')
    replacement = Worker(OUT / 'replacement')
    replacement.call('initialize', password=PASSWORD)
    restored = replacement.call('restore', password=BACKUP_PASSWORD, path=str(backup), name='restored')
    assert restored['public_key'] == first['public_key'] and restored['backed_up']
    assert replacement.call('status')['backup_upgrade_accounts'] == []
    replacement.close()
    passed('v2 backup restores the identical public key into a separate native vault')
    time.sleep(2.1)
    worker.call('unlock', password=PASSWORD)
    before_restart = vault_path.read_bytes()
    worker.close()
    worker = Worker(OUT / 'vault')
    assert not worker.call('status')['unlocked']
    assert vault_path.read_bytes() == before_restart
    worker.call('unlock', password=PASSWORD)
    assert worker.call('status')['unlocked']
    worker.call('lock')
    assert not worker.call('status')['unlocked']
    status = dict(line.split(':', 1) for line in Path(f'/proc/{worker.child.pid}/status').read_text().splitlines())
    assert int(status['VmLck'].split()[0]) == 0
    worker.close()
    passed('worker exit revokes keys and releases the vault lock; reopening preserves encrypted records; explicit lock releases key pages')
    for kind in ['replayed-id', 'unknown-operation', 'extra-field', 'extra-status-field', 'oversized-frame', 'partial-frame']:
        broken = Worker(OUT / ('invalid-' + kind))
        if kind == 'oversized-frame':
            broken.child.stdin.write(struct.pack('!I', 131073))
        elif kind == 'partial-frame':
            broken.child.stdin.write(struct.pack('!I', 10) + b'{')
            broken.child.stdin.close()
        else:
            request = {'operation': 'status'}
            if kind == 'unknown-operation':
                request['operation'] = 'export_private_key'
            if kind == 'extra-status-field':
                request['unexpected'] = True
            if kind == 'extra-field':
                request = {'operation': 'initialize', 'password': PASSWORD, 'unexpected': True}
            broken.send({'version': 1, 'id': 1 if kind == 'replayed-id' else 2, 'request': request})
        assert broken.child.wait(timeout=5) != 0, kind
        assert not (broken.directory / 'vault-v1.json').exists()
    passed('replayed identifiers, unknown operations/fields, oversized and truncated frames terminate without writing vault data')
    # The helper exits abruptly. Kernel parent-death signaling must kill the worker,
    # without depending on GUI cleanup, graceful IPC, or a normal destructor.
    helper = '''import subprocess,os,json,struct,sys
p=subprocess.Popen([sys.argv[1],'--asset-vault-worker'],stdin=subprocess.PIPE,stdout=subprocess.PIPE)
b=json.dumps({'version':1,'id':1,'request':{'operation':'open','directory':sys.argv[2]}}).encode()
p.stdin.write(struct.pack('!I',len(b))+b);p.stdin.flush()
n=struct.unpack('!I',p.stdout.read(4))[0];assert 'Ok' in json.loads(p.stdout.read(n))['result']
print(p.pid,flush=True);os._exit(0)
'''
    parent = subprocess.Popen([sys.executable, '-c', helper, BINARY, str(OUT / 'parent-death')], stdout=subprocess.PIPE, text=True)
    child_pid = int(parent.stdout.readline().strip())
    parent.wait(timeout=5)
    for _ in range(50):
        try:
            state = Path(f'/proc/{child_pid}/stat').read_text().split(') ', 1)[1].split()[0]
        except FileNotFoundError:
            break
        if state == 'Z':
            break
        time.sleep(.1)
    else:
        raise AssertionError('worker survived abrupt parent death')
    passed('abrupt parent death terminates the packaged worker through the kernel')
    (OUT / 'acceptance.json').write_text(json.dumps({'checks': checks, 'passed': len(checks), 'binary_sha256': hashlib.sha256(Path(BINARY).read_bytes()).hexdigest(), 'verified_backup_seconds': export_seconds, 'tested_platform': sys.platform}, ensure_ascii=False, indent=2) + '\n')
    print(OUT, flush=True)
finally:
    for child in children:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=5)
