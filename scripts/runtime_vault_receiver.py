"""Keyless remote vault receiver. This committed file is streamed read-only over SSH."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import sys
import tarfile
import uuid

RESERVE = 2 * 1024**3
HEX = re.compile(r'[0-9a-f]{64}')
MAX_MEMBER = 65 * 1024**2


def digest(path):
    with path.open('rb') as stream: return hashlib.file_digest(stream, 'sha256').hexdigest()


def private_root(value):
    root = Path(value)
    if root.resolve() != root: raise RuntimeError('vault_remote_root_symlink')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    meta = root.stat()
    if meta.st_uid != os.getuid() or meta.st_mode & 0o077: raise RuntimeError('vault_remote_root_not_private')
    return root


def index(root):
    path = root / 'vault-index.json'
    if path.is_symlink(): raise RuntimeError('vault_remote_index_symlink')
    value = json.loads(path.read_text()) if path.exists() else {'schema_version':1,'snapshots':[]}
    if value.get('schema_version') != 1 or len(value['snapshots']) > 8: raise RuntimeError('vault_remote_index_invalid')
    for row in value['snapshots']:
        if not re.fullmatch(r'snapshot-[A-Za-z0-9-]+',row['name']) or not HEX.fullmatch(row['manifest_sha256']):raise RuntimeError('vault_remote_index_invalid')
    return value


def inventory(root):
    if (root / 'objects').is_symlink() or (root / 'snapshots').is_symlink(): raise RuntimeError('vault_remote_directory_symlink')
    value = index(root); objects = {}
    for row in value['snapshots']:
        snapshot = root / 'snapshots' / (row['name'] + '.enc')
        if snapshot.is_symlink() or digest(snapshot) != row['manifest_sha256']: raise RuntimeError('vault_remote_manifest_invalid')
        for identity, sha in row['objects'].items():
            if not HEX.fullmatch(identity) or not HEX.fullmatch(sha): raise RuntimeError('vault_remote_object_id_invalid')
            path = root / 'objects' / identity
            if path.is_symlink() or not path.is_file() or digest(path) != sha: raise RuntimeError('vault_remote_existing_object_invalid')
            objects[identity] = sha
    snapshots = [{key:row[key] for key in ('name','created_at_ts','source_revision','manifest_sha256','checkpoint_digest','policy_digest')} for row in value['snapshots']]
    return {'schema_version':1,'objects':objects,'snapshots':snapshots,'available_bytes':shutil.disk_usage(root).free,'retained_versions':len(value['snapshots'])}


def atomic_json(path, value):
    temporary = path.with_name('.' + path.name + '-' + uuid.uuid4().hex)
    with temporary.open('x') as stream:
        json.dump(value, stream, separators=(',', ':')); stream.flush(); os.fsync(stream.fileno())
    os.replace(temporary, path)
    fd = os.open(path.parent, os.O_RDONLY)
    try: os.fsync(fd)
    finally: os.close(fd)


def retire(root, rows, live):
    names={row['name'] for row in live};objects={identity for row in live for identity in row['objects']}
    for row in rows:
        if row['name'] in names:continue
        if not re.fullmatch(r'snapshot-[A-Za-z0-9-]+',row['name']):raise RuntimeError('vault_transaction_invalid')
        path=root/'snapshots'/(row['name']+'.enc')
        if path.is_file() and not path.is_symlink() and digest(path)==row['manifest_sha256']:path.unlink()
        for identity,sha in row['objects'].items():
            if not HEX.fullmatch(identity) or not HEX.fullmatch(sha):raise RuntimeError('vault_transaction_invalid')
            path=root/'objects'/identity
            if identity not in objects and path.is_file() and not path.is_symlink() and digest(path)==sha:path.unlink()


def recover(root):
    journal=root/'.vault-transaction.json'
    if journal.exists():
        if journal.is_symlink() or journal.stat().st_size>MAX_MEMBER:raise RuntimeError('vault_transaction_invalid')
        value=json.loads(journal.read_text())
        if value.get('schema_version')!=1:raise RuntimeError('vault_transaction_invalid')
        live=index(root)['snapshots']
        if any(row['name']==value['name'] and row['manifest_sha256']==value['manifest_sha256'] for row in live):
            retire(root,value['previous'],live)
        else:
            retire(root,[{'name':value['name'],'manifest_sha256':value['manifest_sha256'],'objects':value['created']}],live)
        journal.unlink()
    for pending in root.glob('.pending-*'):
        marker=pending/'.vault-owner.json'
        if not re.fullmatch(r'\.pending-[0-9a-f]{32}',pending.name) or pending.is_symlink() or not marker.is_file() or marker.is_symlink():continue
        if marker.stat().st_size<=1024 and json.loads(marker.read_text())=={'schema_version':1,'kind':'runtime_vault_pending'}:shutil.rmtree(pending)


def receive(root, stream):
    pending = root / ('.pending-' + uuid.uuid4().hex)
    pending.mkdir(mode=0o700)
    atomic_json(pending/'.vault-owner.json',{'schema_version':1,'kind':'runtime_vault_pending'})
    moved = []; committed = False; publication = None
    try:
        seen = set()
        with tarfile.open(fileobj=stream, mode='r|') as archive:
            for member in archive:
                if not member.isfile() or Path(member.name).name != member.name or member.name in seen:
                    raise RuntimeError('vault_transport_member_invalid')
                if member.name not in ('manifest.enc','transfer.json') and not HEX.fullmatch(member.name):
                    raise RuntimeError('vault_transport_member_invalid')
                if member.size > MAX_MEMBER or member.size < 0 or len(seen) >= 160002:
                    raise RuntimeError('vault_transport_limit')
                seen.add(member.name)
                if shutil.disk_usage(root).free < RESERVE + member.size: raise RuntimeError('vault_remote_low_space')
                with archive.extractfile(member) as source, (pending / member.name).open('xb') as output:
                    copied = 0
                    while chunk := source.read(1024**2):
                        if shutil.disk_usage(root).free < RESERVE + len(chunk): raise RuntimeError('vault_remote_low_space')
                        copied += len(chunk); output.write(chunk)
                    if copied != member.size: raise RuntimeError('vault_transport_truncated')
                    output.flush(); os.fsync(output.fileno())
        transfer = json.loads((pending / 'transfer.json').read_text())
        if transfer.get('schema_version') != 1: raise RuntimeError('vault_transfer_schema_invalid')
        keep = transfer['retain_versions']; row = transfer['snapshot']; name = row['name']
        if not isinstance(keep, int) or not 1 <= keep <= 8: raise RuntimeError('vault_retention_invalid')
        if not re.fullmatch(r'snapshot-[A-Za-z0-9-]+', name): raise RuntimeError('vault_snapshot_name_invalid')
        if not HEX.fullmatch(row['manifest_sha256']) or digest(pending / 'manifest.enc') != row['manifest_sha256']:
            raise RuntimeError('vault_manifest_digest_mismatch')
        if not re.fullmatch('[0-9a-f]{40}', row['source_revision']): raise RuntimeError('vault_source_revision_invalid')
        objects = root / 'objects'; snapshots = root / 'snapshots'
        for path in (objects, snapshots):
            if path.is_symlink(): raise RuntimeError('vault_remote_directory_symlink')
            path.mkdir(exist_ok=True, mode=0o700)
        expected_new = set(seen) - {'transfer.json','manifest.enc'}
        if not expected_new.issubset(row['objects']): raise RuntimeError('vault_unreferenced_upload')
        for identity, sha in row['objects'].items():
            if not HEX.fullmatch(identity) or not HEX.fullmatch(sha): raise RuntimeError('vault_remote_object_id_invalid')
            candidate = pending / identity if identity in expected_new else objects / identity
            if candidate.is_symlink() or not candidate.is_file() or digest(candidate) != sha:
                raise RuntimeError('vault_object_digest_mismatch')
        previous = index(root)
        if any(old['name'] == name for old in previous['snapshots']): raise RuntimeError('vault_snapshot_already_exists')
        created={identity:row['objects'][identity] for identity in expected_new if not (objects/identity).exists()}
        atomic_json(root/'.vault-transaction.json',{'schema_version':1,'name':name,'manifest_sha256':row['manifest_sha256'],'created':created,'previous':previous['snapshots']})
        for identity in expected_new:
            target = objects / identity
            if target.exists():
                if target.is_symlink() or digest(target) != row['objects'][identity]: raise RuntimeError('vault_object_conflict')
            else:
                os.rename(pending / identity, target); moved.append(target)
        manifest = snapshots / (name + '.enc')
        if manifest.exists() or manifest.is_symlink(): raise RuntimeError('vault_snapshot_conflict')
        os.rename(pending / 'manifest.enc', manifest); moved.append(manifest)
        retained = [row, *previous['snapshots']][:keep]
        publication=(name,row['manifest_sha256'])
        for directory in (objects, snapshots):
            fd = os.open(directory, os.O_RDONLY)
            try: os.fsync(fd)
            finally: os.close(fd)
        atomic_json(root / 'vault-index.json', {'schema_version':1,'snapshots':retained})
        committed = True
        retire(root,previous['snapshots'],retained)
        (root/'.vault-transaction.json').unlink()
        return {'status':'verified_encrypted','encrypted':True,'retained_versions':len(retained),'manifest_sha256':row['manifest_sha256'],'source_revision':row['source_revision'],'new_objects':len(expected_new),'available_bytes':shutil.disk_usage(root).free}
    finally:
        uncertain=False
        if not committed and publication:
            try:committed=any(row['name']==publication[0] and row['manifest_sha256']==publication[1] for row in index(root)['snapshots'])
            except Exception:uncertain=True
        # A lost acknowledgement or failed directory fsync can happen after the
        # atomic index replacement. Never roll back objects referenced by it.
        if not committed and not uncertain:
            for path in moved:
                if path.is_file() and not path.is_symlink(): path.unlink()
        shutil.rmtree(pending)


def main():
    os.umask(0o077)
    if sys.platform not in ('linux','darwin'): raise RuntimeError('vault_remote_platform_unsupported')
    import fcntl
    os.nice(15)
    for signum in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT):signal.signal(signum,lambda *_:sys.exit(143))
    root = private_root(sys.argv[2])
    with (root / '.vault.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            sys.exit(os.EX_TEMPFAIL)
        recover(root)
        if sys.argv[1] == 'inventory': value = inventory(root)
        elif sys.argv[1] == 'receive': value = receive(root, sys.stdin.buffer)
        elif sys.argv[1] in ('manifest','export'):
            name = sys.argv[3]
            if not re.fullmatch(r'snapshot-[A-Za-z0-9-]+',name): raise RuntimeError('vault_snapshot_name_invalid')
            known = next((row for row in index(root)['snapshots'] if row['name']==name),None)
            if not known: raise RuntimeError('vault_snapshot_missing')
            path = root / 'snapshots' / (name + '.enc')
            if path.is_symlink() or digest(path)!=known['manifest_sha256']: raise RuntimeError('vault_remote_manifest_invalid')
            if sys.argv[1]=='manifest':sys.stdout.buffer.write(path.read_bytes()); return
            request=sys.stdin.buffer.read(MAX_MEMBER+1)
            if len(request)>MAX_MEMBER:raise RuntimeError('vault_export_request_too_large')
            identities=json.loads(request)
            if not isinstance(identities,list) or len(identities)>160000:raise RuntimeError('vault_export_request_invalid')
            with tarfile.open(fileobj=sys.stdout.buffer,mode='w|',dereference=True) as archive:
                for identity in identities:
                    if not isinstance(identity,str) or not HEX.fullmatch(identity) or identity not in known['objects']:raise RuntimeError('vault_export_object_not_in_snapshot')
                    obj=root/'objects'/identity
                    if obj.is_symlink() or obj.stat().st_size>4*1024**2+64 or digest(obj)!=known['objects'][identity]:raise RuntimeError('vault_export_object_invalid')
                    archive.add(obj,arcname=identity,recursive=False)
            return
        else: raise RuntimeError('vault_remote_operation_invalid')
    print(json.dumps(value, separators=(',', ':')))


if __name__ == '__main__': main()
