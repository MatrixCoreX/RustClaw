#!/usr/bin/env python3
"""Copy a verified checkpoint over pinned SSH into an owner-private remote directory."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import subprocess
import sys
import time
import uuid

from runtime_checkpoint_storage import digest, manifest, write_json

PREPARE = r'''
import json,os,pathlib,shutil,sys
root=pathlib.Path(sys.argv[1]);pending=root/sys.argv[2]
if sys.platform not in ('linux','darwin'):raise RuntimeError('checkpoint_remote_platform_unsupported')
if root.resolve()!=root:raise RuntimeError('checkpoint_remote_root_symlink')
root.mkdir(parents=True,exist_ok=True,mode=0o700)
if root.stat().st_uid!=os.getuid() or root.stat().st_mode&0o077:raise RuntimeError('checkpoint_remote_root_not_private')
if shutil.disk_usage(root).free<int(sys.argv[3])+2*1024**3:raise RuntimeError('checkpoint_remote_low_space')
pending.mkdir(mode=0o700)
print(json.dumps({'platform':sys.platform,'ionice':sys.platform=='linux' and bool(shutil.which('ionice')),'pending':str(pending)}))
'''
PUBLISH = r'''
import hashlib,json,os,pathlib,runpy,shutil,signal,sys,time
for signum in (signal.SIGTERM,signal.SIGHUP):signal.signal(signum,lambda *_:sys.exit(143))
root=pathlib.Path(sys.argv[1]);pending=root/sys.argv[2];final=root/sys.argv[3]
module=pending/'runtime_checkpoint_storage.py'
if hashlib.sha256(module.read_bytes()).hexdigest()!=sys.argv[4]:raise RuntimeError('checkpoint_remote_verifier_mismatch')
verifiers=root/'.verifiers';verifiers.mkdir(exist_ok=True,mode=0o700)
verified_module=verifiers/(sys.argv[4]+'.py')
if verified_module.exists():
    if hashlib.sha256(verified_module.read_bytes()).hexdigest()!=sys.argv[4]:raise RuntimeError('checkpoint_remote_verifier_mismatch')
    module.unlink()
else:os.replace(module,verified_module);verified_module.chmod(0o400)
for file in pending.iterdir():
    if not file.is_file() or file.is_symlink():raise RuntimeError('checkpoint_remote_member_invalid')
    with file.open('rb') as stream:os.fsync(stream.fileno())
fd=os.open(pending,os.O_RDONLY);os.fsync(fd);os.close(fd)
ns=runpy.run_path(str(verified_module));ns['verify_checkpoint'](pending)
if ns['digest'](pending/'manifest.json')!=sys.argv[5]:raise RuntimeError('checkpoint_remote_manifest_mismatch')
restored=None;restore=root/('.restore-'+sys.argv[2].removeprefix('.pending-'))
if restore.exists():raise RuntimeError('checkpoint_remote_restore_output_exists')
try:
    if sys.argv[7]=='restore':restored=ns['restore_checkpoint'](pending,restore,reserve_bytes=2*1024**3,databases_only=True)
finally:
    if restore.exists():shutil.rmtree(restore)
if final.exists():
    ns['verify_checkpoint'](final)
    if ns['digest'](final/'manifest.json')!=sys.argv[5]:raise RuntimeError('checkpoint_remote_existing_conflict')
    shutil.rmtree(pending)
else:os.rename(pending,final)
index=root/'checkpoint-index.json';previous=json.loads(index.read_text()) if index.exists() else {'checkpoints':[]}
old=previous.get('checkpoints',[])
entry={'name':final.name,'manifest_sha256':sys.argv[5],'verifier_sha256':sys.argv[4],'source_revision':sys.argv[6],'verified_at_ts':int(time.time())}
ns['write_json'](index,{'schema_version':1,'checkpoints':[entry]})
fd=os.open(root,os.O_RDONLY);os.fsync(fd);os.close(fd)
for row in old:
    name=row.get('name','')
    if name==final.name or not name.startswith('checkpoint-') or pathlib.Path(name).name!=name:continue
    path=root/name
    if path.is_dir() and not path.is_symlink() and ns['digest'](path/'manifest.json')==row['manifest_sha256']:shutil.rmtree(path)
print(json.dumps({'status':'verified','remote_source_revision':sys.argv[6],'verifier_sha256':hashlib.sha256(verified_module.read_bytes()).hexdigest(),'checkpoint_runtime_source':ns['manifest'](final)['source_revision'],'verified_at_ts':entry['verified_at_ts'],'restore_test':restored,'available_bytes':shutil.disk_usage(root).free}))
'''
CLEAN = r'''
import pathlib,shutil,sys
root=pathlib.Path(sys.argv[1]);name=sys.argv[2]
if name.startswith('.pending-') and pathlib.Path(name).name==name:
    path=root/name
    if path.is_dir() and not path.is_symlink():shutil.rmtree(path)
'''


def validate_target(host, directory):
    if not re.fullmatch(r'[A-Za-z0-9_.@:-]+',host) or host.startswith('-'):raise RuntimeError('checkpoint_remote_host_invalid')
    if not re.fullmatch(r'/[A-Za-z0-9_/-]+',directory) or '..' in PurePosixPath(directory).parts or directory=='/':raise RuntimeError('checkpoint_remote_directory_invalid')


def ssh_options():return ['-o','BatchMode=yes','-o','StrictHostKeyChecking=yes','-o','ConnectTimeout=10']


def remote(host, code, *args, low_priority=False, timeout=1800):
    command=('ionice -c 3 nice -n 19 ' if low_priority else 'nice -n 19 ')+shlex.join(['python3','-B','-c',code,*map(str,args)])
    result=subprocess.run(['ssh',*ssh_options(),host,command],capture_output=True,text=True,timeout=timeout)
    if result.returncode:raise RuntimeError('checkpoint_remote_operation_failed')
    return json.loads(result.stdout) if result.stdout.strip() else None


def sync(root, destination, host, remote_directory, restore_test=False):
    validate_target(host,remote_directory)
    checkpoints=[p for p in destination.glob('checkpoint-*') if p.is_dir() and not p.is_symlink() and (p/'manifest.json').is_file()]
    if not checkpoints:raise RuntimeError('checkpoint_remote_local_missing')
    checkpoint=max(checkpoints,key=lambda p:manifest(p)['created_at_ts']);value=manifest(checkpoint)
    expected=[row for row in value['files'] if row['kind']=='sqlite']+[value['control_archive']]
    files=[checkpoint/'manifest.json']
    for row in expected:
        name=row.get('archive') or row['name'];path=checkpoint/name
        if Path(name).name!=name or path.is_symlink() or digest(path)!=row['sha256']:raise RuntimeError('checkpoint_remote_local_digest_mismatch')
        files.append(path)
    verifier=Path(__file__).with_name('runtime_checkpoint_storage.py');module_digest=digest(verifier)
    source=subprocess.run(['git','-C',str(Path(__file__).resolve().parents[1]),'rev-parse','HEAD'],capture_output=True,text=True)
    if source.returncode:
        from runtime_checkpoint import running_revision
        revision=running_revision(root)
    else:
        revision=source.stdout.strip()
        committed=subprocess.check_output(['git','-C',str(Path(__file__).resolve().parents[1]),'show',revision+':scripts/runtime_checkpoint_storage.py'])
        if hashlib.sha256(committed).hexdigest()!=module_digest:raise RuntimeError('checkpoint_remote_verifier_not_committed')
    if not re.fullmatch('[0-9a-f]{40}',revision):raise RuntimeError('checkpoint_remote_source_revision_invalid')
    pending='.pending-'+uuid.uuid4().hex
    try:
        info=remote(host,PREPARE,remote_directory,pending,sum(p.stat().st_size for p in files),timeout=30)
        subprocess.run(['scp','-q',*ssh_options(),*map(str,files),str(verifier),host+':'+remote_directory+'/'+pending+'/'],check=True,timeout=1200,capture_output=True)
        report=remote(host,PUBLISH,remote_directory,pending,checkpoint.name,module_digest,digest(checkpoint/'manifest.json'),revision,'restore' if restore_test else 'verify',low_priority=info['ionice'])
        if report['remote_source_revision']!=revision or report['verifier_sha256']!=module_digest:raise RuntimeError('checkpoint_remote_source_mismatch')
        report.update(created_at_ts=value['created_at_ts'],bytes=sum(p.stat().st_size for p in files),database_count=sum(r['kind']=='sqlite' for r in value['files']))
        return report
    except BaseException:
        try:remote(host,CLEAN,remote_directory,pending,timeout=30)
        except Exception:pass
        raise


def main():
    os.umask(0o077)
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',required=True,type=Path);p.add_argument('--destination',required=True,type=Path);p.add_argument('--host',required=True);p.add_argument('--remote-directory',required=True);p.add_argument('--restore-test',action='store_true');a=p.parse_args()
    status_path=a.root/'data/runtime-checkpoint-status.json'
    try:
        import fcntl
        with (a.destination/'.checkpoint.lock').open('a') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            result=sync(a.root.resolve(),a.destination.resolve(),a.host,a.remote_directory,a.restore_test)
        status=json.loads(status_path.read_text());status.update(status='verified_remote',remote_status='verified',remote_verified_at_ts=result['verified_at_ts'],created_at_ts=result['created_at_ts'],bytes=result['bytes'],database_count=result['database_count']);status.pop('remote_error_code',None);status.pop('error_code',None);write_json(status_path,status)
        print(json.dumps(result));return 0
    except Exception as error:
        code=str(error) if isinstance(error,RuntimeError) and str(error).startswith('checkpoint_') else 'checkpoint_remote_copy_failed'
        status=json.loads(status_path.read_text()) if status_path.exists() else {'schema_version':1,'status':'unavailable'}
        if status.get('status') in ['verified','verified_remote','verified_local_remote_failed']:status['status']='verified_local_remote_failed'
        status.update(remote_status='failed',remote_error_code=code);write_json(status_path,status)
        print(json.dumps({'status':'error','error_code':code}),file=sys.stderr);return 1

if __name__=='__main__':sys.exit(main())
