#!/usr/bin/env python3
"""Decrypt verified vault records into a NEW directory; never activate a restore."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile

from runtime_vault_cipher import CHUNK_BYTES, VaultCipher, load_key
from runtime_checkpoint_storage import relative, write_json


class LocalReader:
    def __init__(self, root):self.root=Path(root)
    def index(self):return json.loads((self.root/'vault-index.json').read_text())
    def manifest(self, name):return (self.root/'snapshots'/(name+'.enc')).read_bytes()
    @contextlib.contextmanager
    def objects(self, name, identities):
        def read():
            for identity in identities:
                path=self.root/'objects'/identity
                if path.is_symlink() or path.stat().st_size>CHUNK_BYTES+64:raise RuntimeError('vault_restore_object_invalid')
                yield identity,path.read_bytes()
        yield read()


class RemoteReader:
    def __init__(self, transport):self.transport=transport
    def index(self):return self.transport.inventory()
    def manifest(self,name):return self.transport.load_manifest(name)
    @contextlib.contextmanager
    def objects(self,name,identities):
        with tempfile.TemporaryFile() as errors:
            process=subprocess.Popen(self.transport.command('export',name),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=errors)
            try:
                process.stdin.write(json.dumps(identities,separators=(',',':')).encode());process.stdin.close();process.stdin=None
                with tarfile.open(fileobj=process.stdout,mode='r|') as archive:
                    def read():
                        for member in archive:
                            if not member.isfile() or member.size>CHUNK_BYTES+64:raise RuntimeError('vault_restore_transport_invalid')
                            with archive.extractfile(member) as stream:yield member.name,stream.read()
                    yield read()
                process.stdout.close();process.wait(timeout=30)
                if process.returncode:raise RuntimeError('vault_restore_transport_failed')
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:process.wait(timeout=10)
                    except subprocess.TimeoutExpired:process.kill();process.wait()


def restore(reader,key,output,*,prefixes=(),reserve=2*1024**3):
    output=Path(output).absolute()
    if output.exists() or output.is_symlink():raise RuntimeError('vault_restore_output_exists')
    cipher=VaultCipher(key);index=reader.index()
    if not index['snapshots']:raise RuntimeError('vault_restore_snapshot_missing')
    entry=index['snapshots'][0];name=entry['name']
    if Path(name).name!=name or not name.startswith('snapshot-'):raise RuntimeError('vault_restore_snapshot_invalid')
    encrypted=reader.manifest(name)
    if hashlib.sha256(encrypted).hexdigest()!=entry['manifest_sha256']:raise RuntimeError('vault_restore_manifest_digest')
    value=json.loads(cipher.decrypt_manifest(encrypted))
    if value.get('schema_version')!=1 or value.get('kind')!='encrypted_runtime_vault' or value['name']!=name:raise RuntimeError('vault_restore_schema_invalid')
    all_records={}
    for row in value['files']:
        path=str(relative(row['path']))
        if path in all_records:raise RuntimeError('vault_restore_duplicate_path')
        if row.get('kind') not in ('file','sqlite','hardlink','symlink','directory'):raise RuntimeError('vault_restore_record_kind_invalid')
        all_records[path]=row
    prefixes=[str(relative(p)).rstrip('/') for p in prefixes]
    selected={path for path in all_records if not prefixes or any(path==p or path.startswith(p+'/') for p in prefixes)}
    if not selected:raise RuntimeError('vault_restore_selection_empty')
    # A selected hardlink must bring its original into the same isolated output.
    while True:
        added={all_records[path]['target'] for path in selected if all_records[path]['kind']=='hardlink'}-selected
        if not added:break
        if not added.issubset(all_records):raise RuntimeError('vault_restore_hardlink_target_missing')
        selected.update(added)
    records=[row for path,row in all_records.items() if path in selected]
    files=[row for row in records if row['kind'] in ('file','sqlite')]
    required=sum(row['raw_bytes'] for row in files)
    if reserve<0:raise RuntimeError('vault_restore_invalid_reserve')
    parent=output.parent.resolve();parent.mkdir(parents=True,exist_ok=True);output=parent/output.name
    if shutil.disk_usage(parent).free<required+reserve:raise RuntimeError('vault_restore_low_space')
    output.mkdir(mode=0o700)
    try:
        metadata=output.stat()
        if metadata.st_uid!=os.getuid() or metadata.st_mode&0o777!=0o700:raise RuntimeError('vault_restore_output_not_private')
        for row in records:
            if row['kind']=='directory':(output/relative(row['path'])).mkdir(parents=True,exist_ok=True,mode=0o700)
        identities=[identity for row in files for identity in row['chunks']]
        with reader.objects(name,identities) as stream:
            chunks=iter(stream)
            for row in files:
                target=output/relative(row['path']);target.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
                sha=hashlib.sha256();size=0
                with target.open('xb') as destination:
                    for expected in row['chunks']:
                        identity,blob=next(chunks)
                        if identity!=expected or hashlib.sha256(blob).hexdigest()!=value['objects'][identity]:raise RuntimeError('vault_restore_chunk_order_or_digest')
                        raw=cipher.decrypt_object(identity,blob)
                        if shutil.disk_usage(parent).free<reserve+len(raw):raise RuntimeError('vault_restore_storage_pressure')
                        size+=len(raw)
                        if size>row['raw_bytes']:raise RuntimeError('vault_restore_size_mismatch')
                        sha.update(raw);destination.write(raw)
                    destination.flush();os.fsync(destination.fileno())
                if size!=row['raw_bytes'] or sha.hexdigest()!=row['raw_sha256']:raise RuntimeError('vault_restore_file_digest')
                target.chmod(row.get('mode',0o600)&0o777)
                if row['kind']=='sqlite':
                    with contextlib.closing(sqlite3.connect(target.as_uri()+'?mode=ro&immutable=1',uri=True)) as db:
                        if db.execute('PRAGMA quick_check').fetchone()[0]!='ok':raise RuntimeError('vault_restored_database_invalid')
            try:next(chunks)
            except StopIteration:pass
            else:raise RuntimeError('vault_restore_extra_chunks')
        pending=[row for row in records if row['kind']=='hardlink']
        while pending:
            remaining=[]
            for row in pending:
                target=output/relative(row['path']);source=output/relative(row['target'])
                if not source.is_file():remaining.append(row);continue
                target.parent.mkdir(parents=True,exist_ok=True,mode=0o700);os.link(source,target)
            if len(remaining)==len(pending):raise RuntimeError('vault_restore_hardlink_cycle')
            pending=remaining
        for row in records:
            if row['kind']!='symlink':continue
            target=output/relative(row['path']);link=Path(row['target'])
            if link.is_absolute() or not (target.parent/link).resolve().is_relative_to(output):raise RuntimeError('vault_restore_symlink_escape')
            target.parent.mkdir(parents=True,exist_ok=True,mode=0o700);target.symlink_to(link)
        for row in sorted(records,key=lambda row:row['path'].count('/'),reverse=True):
            if row['kind']=='directory':(output/relative(row['path'])).chmod(row.get('mode',0o700)&0o777)
        report={'schema_version':1,'status':'restored_verified','activated':False,'source_revision':value['source_revision'],'files':len(records),'selected_prefixes':prefixes,'external_dependencies':len(value.get('external_dependencies',[])),'manifest_sha256':entry['manifest_sha256']}
        write_json(output/'vault-restore.json',report)
        return report
    except BaseException:
        # Only this invocation's newly created output is eligible for cleanup.
        # Restored source modes can be read-only; make directories traversable
        # without following symlinks before removing the incomplete copy.
        output.chmod(0o700)
        for directory,names,_ in os.walk(output,followlinks=False):
            for name in names:
                child=Path(directory)/name
                if not child.is_symlink():child.chmod(0o700)
        shutil.rmtree(output);raise


def main():
    os.umask(0o077)
    if sys.platform not in ('linux','darwin'):
        print(json.dumps({'status':'unsupported','error_code':'vault_platform_unsupported'}));return 1
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--vault-directory',type=Path);p.add_argument('--host');p.add_argument('--remote-directory');p.add_argument('--key-file',type=Path,required=True);p.add_argument('--output',type=Path);p.add_argument('--verify-only',action='store_true');p.add_argument('--prefix',action='append',default=[]);p.add_argument('--reserve-bytes',type=int,default=2*1024**3);a=p.parse_args()
    try:
        if a.host:
            from runtime_checkpoint_vault import RemoteTransport
            reader=RemoteReader(RemoteTransport(a.host,a.remote_directory,Path(__file__).resolve().parents[1]))
        elif a.vault_directory:reader=LocalReader(a.vault_directory)
        else:raise RuntimeError('vault_restore_source_missing')
        if a.verify_only:
            from runtime_vault_verify import verify
            report=verify(reader,load_key(a.key_file))
        else:
            if not a.output:raise RuntimeError('vault_restore_output_missing')
            report=restore(reader,load_key(a.key_file),a.output,prefixes=a.prefix,reserve=a.reserve_bytes)
        print(json.dumps(report));return 0
    except Exception as error:
        code=str(error) if isinstance(error,RuntimeError) and str(error).startswith('vault_') else 'vault_restore_failed'
        print(json.dumps({'status':'error','error_code':code}));return 1


if __name__=='__main__':raise SystemExit(main())
