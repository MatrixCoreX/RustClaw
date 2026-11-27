#!/usr/bin/env python3
"""Incremental encrypted payload snapshots, transported as one bounded TAR stream."""
import argparse
import contextlib
import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid

from runtime_checkpoint_remote import ssh_options, validate_target
from runtime_checkpoint_storage import digest, manifest, write_json
from runtime_vault_cipher import CHUNK_BYTES, MAX_MANIFEST_BYTES, VaultCipher, load_key
from runtime_vault_payloads import enumerate_payloads


class RemoteTransport:
    def __init__(self, host, directory, source_root):
        validate_target(host,directory)
        self.host=host; self.directory=directory
        module=Path(__file__).with_name('runtime_vault_receiver.py')
        self.code=module.read_text()
        revision=subprocess.run(['git','-C',str(source_root),'rev-parse','HEAD'],capture_output=True,text=True)
        if revision.returncode==0:
            try:committed=subprocess.check_output(['git','-C',str(source_root),'show',revision.stdout.strip()+':scripts/runtime_vault_receiver.py'],stderr=subprocess.DEVNULL)
            except subprocess.CalledProcessError as error:raise RuntimeError('vault_receiver_not_committed') from error
            if committed!=module.read_bytes(): raise RuntimeError('vault_receiver_not_committed')

    def command(self, operation, *args):
        return ['ssh',*ssh_options(),self.host,shlex.join(['python3','-I','-B','-c',self.code,operation,self.directory,*args])]

    @staticmethod
    def check_remote_exit(operation, status):
        if status == os.EX_TEMPFAIL:
            raise RuntimeError('vault_remote_busy')
        if status == 255:
            raise RuntimeError('vault_remote_transport_unavailable')
        if status:
            raise RuntimeError(f'vault_remote_{operation}_failed')

    def read_remote(self, operation, timeout, limit, *args):
        try:
            result = subprocess.run(self.command(operation, *args), capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired as error:
            raise RuntimeError(f'vault_remote_{operation}_timeout') from error
        self.check_remote_exit(operation, result.returncode)
        if len(result.stdout) > limit:
            raise RuntimeError(f'vault_remote_{operation}_too_large')
        return result.stdout

    def inventory(self):
        return json.loads(self.read_remote('inventory', 300, MAX_MANIFEST_BYTES))

    def load_manifest(self, name):
        return self.read_remote('manifest', 120, MAX_MANIFEST_BYTES + 32, name)

    @contextlib.contextmanager
    def writer(self):
        with tempfile.TemporaryFile() as errors:
            process=subprocess.Popen(self.command('receive'),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=errors)
            try:
                yield process.stdin
                process.stdin.close(); process.stdin=None
                stdout,_=process.communicate(timeout=1800)
                self.check_remote_exit('receive', process.returncode)
                self.report=json.loads(stdout)
            except BrokenPipeError as error:
                try:
                    status = process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                    raise RuntimeError('vault_remote_receive_timeout') from error
                self.check_remote_exit('receive', status)
                raise RuntimeError('vault_remote_receive_failed') from error
            except BaseException:
                if process.poll() is None: process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
                raise


def add_member(archive,name,data):
    member=tarfile.TarInfo(name);member.size=len(data);member.mode=0o600
    archive.addfile(member,io.BytesIO(data))


def build_snapshot(root,checkpoint,transport,key,scopes=('packages','models'),retain=3):
    if not isinstance(retain,int) or not 1<=retain<=8: raise RuntimeError('vault_retention_invalid')
    root=Path(root).resolve();checkpoint=Path(checkpoint)
    source=manifest(checkpoint);checkpoint_digest=digest(checkpoint/'manifest.json')
    cipher=VaultCipher(key);scopes=sorted(set(scopes))
    control=checkpoint/source['control_archive']['name']
    if digest(control)!=source['control_archive']['sha256']:raise RuntimeError('vault_checkpoint_corrupt')
    pinned={}
    if 'packages' in scopes:
        with tarfile.open(control,'r|xz') as archive:
            for member in archive:
                parts=Path(member.name).parts
                if len(parts)==4 and parts[:2]==('data','skill-packages') and parts[-1]=='current.json':
                    if not member.isfile() or member.size>65536:raise RuntimeError('vault_package_pointer_invalid')
                    with archive.extractfile(member) as stream:pinned[member.name]=json.load(stream)
    payloads=list(enumerate_payloads(root,scopes,pinned))
    policy_digest=hashlib.sha256(json.dumps({'schema_version':1,'scopes':scopes,'retain':retain},sort_keys=True).encode()).hexdigest()
    remote=transport.inventory();trusted={};matching=None
    for row in remote.get('snapshots',[]):
        encrypted=transport.load_manifest(row['name'])
        if hashlib.sha256(encrypted).hexdigest()!=row['manifest_sha256']: raise RuntimeError('vault_manifest_digest_mismatch')
        previous=json.loads(cipher.decrypt_manifest(encrypted))
        if previous.get('schema_version')!=1: raise RuntimeError('vault_manifest_schema_invalid')
        for identity,sha in previous['objects'].items():
            if remote['objects'].get(identity)!=sha: raise RuntimeError('vault_existing_object_corrupt')
            trusted[identity]=sha
        if previous['checkpoint_digest']==checkpoint_digest and previous['policy_digest']==policy_digest:
            matching=(previous,row)
    if matching:
        previous,row=matching
        return {'status':'verified_encrypted','encrypted':True,'retained_versions':remote['retained_versions'],'verified_at_ts':int(time.time()),'payload_scopes':scopes,'uploaded_bytes':0,'reused_bytes':sum(r.get('raw_bytes',0) for r in previous['files']),'file_count':len(previous['files']),'manifest_sha256':row['manifest_sha256'],'idempotent_replay':True}
    # Attachments are usually already compressed. Refuse clearly impossible
    # selections before reading payloads or disturbing any older remote copy.
    if 'attachments' in scopes:
        attachments=sum(r.get('raw_bytes',0) for r in payloads if r['path'].startswith('.agent-runtime/artifacts/'))
        if attachments>remote['available_bytes']-2*1024**3: raise RuntimeError('vault_attachments_exceed_capacity')
    name='snapshot-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-'+uuid.uuid4().hex[:12]
    value={'schema_version':1,'kind':'encrypted_runtime_vault','name':name,'created_at_ts':int(time.time()),'source_revision':source['source_revision'],'checkpoint_digest':checkpoint_digest,'policy_digest':policy_digest,'payload_scopes':scopes,'files':[],'objects':{},'external_dependencies':[]}
    counters={'uploaded_bytes':0,'reused_bytes':0};new=set()
    with transport.writer() as output,tarfile.open(fileobj=output,mode='w|',bufsize=1024**2) as archive:
        def pack(stream,row):
            sha=hashlib.sha256();size=0;chunks=[]
            while raw:=stream.read(CHUNK_BYTES):
                sha.update(raw);size+=len(raw);identity=cipher.object_id(raw)
                if identity in trusted:
                    blob_sha=trusted[identity];counters['reused_bytes']+=len(raw)
                elif identity in value['objects']:
                    blob_sha=value['objects'][identity];counters['reused_bytes']+=len(raw)
                else:
                    checked,blob=cipher.encrypt_object(raw)
                    if checked!=identity: raise RuntimeError('vault_object_identity_changed')
                    blob_sha=hashlib.sha256(blob).hexdigest()
                    add_member(archive,identity,blob);new.add(identity);counters['uploaded_bytes']+=len(blob)
                value['objects'][identity]=blob_sha;chunks.append(identity)
            if row.get('raw_bytes',size)!=size or row.get('raw_sha256',sha.hexdigest())!=sha.hexdigest(): raise RuntimeError('vault_source_digest_mismatch')
            row={**row,'raw_bytes':size,'raw_sha256':sha.hexdigest(),'chunks':chunks}
            row.pop('signature',None);value['files'].append(row)
        expected={r['path']:r for r in source['files'] if r['kind']=='file'};seen=set()
        control=checkpoint/source['control_archive']['name']
        if digest(control)!=source['control_archive']['sha256']: raise RuntimeError('vault_checkpoint_corrupt')
        with tarfile.open(control,'r|xz') as files:
            for member in files:
                if not member.isfile() or member.name not in expected or member.name in seen: raise RuntimeError('vault_control_member_invalid')
                seen.add(member.name)
                with files.extractfile(member) as stream:pack(stream,{**expected[member.name],'kind':'file'})
        if seen!=set(expected): raise RuntimeError('vault_control_member_missing')
        for row in source['files']:
            if row['kind']!='sqlite':continue
            path=checkpoint/row['archive']
            if digest(path)!=row['sha256']: raise RuntimeError('vault_checkpoint_corrupt')
            with lzma.open(path,'rb') as stream:pack(stream,{'path':row['path'],'kind':'sqlite','raw_bytes':row['raw_bytes'],'raw_sha256':row['raw_sha256'],'mode':0o600})
        known={r['path'] for r in value['files']}
        for row in payloads:
            if row['path'] in known:continue
            known.add(row['path'])
            if row['kind']=='external_dependency':value['external_dependencies'].append(row);continue
            if row['kind']!='file':value['files'].append(row);continue
            path=root/row['path'];before=row['signature']
            with path.open('rb') as stream:pack(stream,row)
            meta=path.stat()
            if path.is_symlink() or (meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns)!=before: raise RuntimeError('vault_payload_changed')
        plaintext=json.dumps(value,separators=(',',':')).encode()
        encrypted=cipher.encrypt_manifest(plaintext)
        add_member(archive,'manifest.enc',encrypted)
        row={k:value[k] for k in ('name','created_at_ts','source_revision','checkpoint_digest','policy_digest','payload_scopes')}
        row.update(manifest_sha256=hashlib.sha256(encrypted).hexdigest(),objects=value['objects'],file_count=len(value['files']))
        add_member(archive,'transfer.json',json.dumps({'schema_version':1,'retain_versions':retain,'snapshot':row},separators=(',',':')).encode())
    report=transport.report
    if report['status']!='verified_encrypted' or report['manifest_sha256']!=row['manifest_sha256'] or report['source_revision']!=source['source_revision']: raise RuntimeError('vault_remote_ack_mismatch')
    return {**report,**counters,'verified_at_ts':int(time.time()),'payload_scopes':scopes,'file_count':len(value['files']),'external_dependency_count':len(value['external_dependencies'])}


def main():
    os.umask(0o077)
    if sys.platform not in ('linux','darwin'):
        print(json.dumps({'status':'unsupported','error_code':'vault_platform_unsupported'}));return 1
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=Path,required=True);p.add_argument('--checkpoint',type=Path,required=True);p.add_argument('--key-file',type=Path,required=True);p.add_argument('--host',required=True);p.add_argument('--remote-directory',required=True);p.add_argument('--scopes',nargs='*',default=['packages','models']);p.add_argument('--retain',type=int,default=3);a=p.parse_args()
    try:
        key=load_key(a.key_file)
        transport=RemoteTransport(a.host,a.remote_directory,Path(__file__).resolve().parents[1])
        report=build_snapshot(a.root,a.checkpoint,transport,key,a.scopes,a.retain)
        write_json(a.root/'data/runtime-vault-status.json',report);print(json.dumps(report));return 0
    except Exception as error:
        code=str(error) if isinstance(error,RuntimeError) and str(error).startswith('vault_') else 'vault_sync_failed'
        print(json.dumps({'status':'error','error_code':code}));return 1


if __name__=='__main__':raise SystemExit(main())
