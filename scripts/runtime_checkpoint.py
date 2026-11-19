#!/usr/bin/env python3
"""Verified control-plane checkpoints; never deletes or overwrites live user data."""
import argparse
import contextlib
import datetime
import hashlib
import json
import lzma
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.request

from runtime_checkpoint_storage import backup_files, compress_file, restore_checkpoint, verify_checkpoint, write_json, selected_files
from runtime_checkpoint_snapshot import snapshot_parent, freeze, pin_databases, validate_pin, signature


def database_paths(root):
    config=tomllib.loads((root/'configs/config.toml').read_text())['database']
    paths=[]
    for key in ('sqlite_path','audit_sqlite_path'):
        value=config.get(key)
        if value and value != ':memory:':
            path=Path(value);path=path if path.is_absolute() else root/path
            path=path.resolve();path.relative_to(root)
            if path.is_file():paths.append(path)
    private=Path(config.get('skill_data_root','data/skills'))
    private=private if private.is_absolute() else root/private
    private.resolve().relative_to(root)
    if private.exists():
        for path in private.rglob('*'):
            if path.is_symlink() or not path.is_file():continue
            with path.open('rb') as stream:magic=stream.read(16)
            if magic==b'SQLite format 3\0':paths.append(path)
    return sorted(set(paths))


def running_revision(root):
    config=tomllib.loads((root/'configs/config.toml').read_text())['database']
    path=Path(config['sqlite_path']);path=path if path.is_absolute() else root/path
    with contextlib.closing(sqlite3.connect(path.as_uri()+'?mode=ro',uri=True,timeout=5)) as db:
        key=db.execute("SELECT user_key FROM auth_keys WHERE role='admin' AND enabled=1 LIMIT 1").fetchone()
    if not key:raise RuntimeError('checkpoint_admin_identity_unavailable')
    request=urllib.request.Request('http://127.0.0.1:8787/v1/system/host-summary',headers={'x-agent-key':key[0]})
    with urllib.request.urlopen(request,timeout=10) as response:report=json.load(response)
    version=report['data']['version'];revision=version.get('build_revision')
    if not revision or version.get('source_match')=='different':raise RuntimeError('checkpoint_cohort_mismatch')
    return revision


def controller(mode, unit, action):
    if sys.platform!='linux' or mode not in ('systemd-system','systemd-user'):
        raise RuntimeError('checkpoint_service_manager_unsupported')
    if not unit or not unit.endswith('.service') or '/' in unit or unit.startswith('-'):
        raise RuntimeError('checkpoint_service_invalid')
    command=['systemctl']+(['--user'] if mode=='systemd-user' else [])
    if mode=='systemd-system' and os.geteuid()!=0:command=['sudo','-n']+command
    result=subprocess.run(command+[action,unit],capture_output=True,text=True,timeout=120)
    if action=='is-active':return result.returncode==0
    if result.returncode:raise RuntimeError('checkpoint_service_'+action+'_failed')


def resume(root, mode, unit):
    marker=root/'data/runtime-checkpoint-resume.json'
    if marker.exists():
        if marker.is_symlink():raise RuntimeError('checkpoint_resume_marker_invalid')
        value=json.loads(marker.read_text())
        if value!={'schema_version':1,'service_mode':mode,'service_unit':unit}:raise RuntimeError('checkpoint_resume_marker_invalid')
        controller(mode,unit,'start');marker.unlink()
    return {'status':'resumed_or_already_running'}


def idle(root):
    if (root/'data/nni/work/current.json').exists():raise RuntimeError('checkpoint_distributed_work_active')
    config=tomllib.loads((root/'configs/config.toml').read_text())['database']
    path=Path(config['sqlite_path']);path=path if path.is_absolute() else root/path
    with contextlib.closing(sqlite3.connect(path.as_uri()+'?mode=ro',uri=True,timeout=5)) as db:
        active=db.execute("SELECT COUNT(*) FROM tasks WHERE status NOT IN ('succeeded','failed','timeout','canceled','cancelled')").fetchone()[0]
        if active:raise RuntimeError('checkpoint_foreground_not_idle')


def create(root, destination, *, mode, unit, retain=1, reserve=2*1024**3, offline=False, read_fence_seconds=1800):
    if type(read_fence_seconds) is not int or not 1<=read_fence_seconds<=3600:
        raise RuntimeError('checkpoint_read_fence_budget_invalid')
    if sys.platform not in ('linux','darwin'):raise RuntimeError('checkpoint_platform_unsupported')
    root=root.resolve();destination=destination.resolve()
    if destination==root or root in destination.parents:raise RuntimeError('checkpoint_destination_inside_workspace')
    destination.mkdir(parents=True,exist_ok=True,mode=0o700)
    if destination.stat().st_uid!=os.geteuid() or destination.stat().st_mode & 0o077:
        raise RuntimeError('checkpoint_destination_not_private')
    if not offline and root.stat().st_dev==destination.stat().st_dev:raise RuntimeError('checkpoint_requires_other_filesystem')
    existing=sorted(destination.glob('checkpoint-*'))
    recent=[p for p in existing if p.is_dir() and (p/'manifest.json').is_file()]
    estimate=max([sum(f.stat().st_size for f in p.iterdir() if f.is_file()) for p in recent[-1:]] or [900*1024**2])
    if shutil.disk_usage(destination).free < reserve+estimate:raise RuntimeError('checkpoint_destination_low_space')
    import fcntl
    with (destination/'.checkpoint.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        idle(root)
        stopped=False
        staging=None
        frozen=None
        try:
            databases=database_paths(root);files=selected_files(root)
            required=sum(p.stat().st_size for p in files)
            frozen=Path(tempfile.mkdtemp(prefix='runtime-checkpoint-',dir=snapshot_parent(required))).resolve()
            revision=running_revision(root) if not offline else subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip()
            staging=Path(tempfile.mkdtemp(prefix='.pending-',dir=destination))
            manifest={'schema_version':1,'kind':'runtime_control_checkpoint','created_at_ts':int(time.time()),
                'source_revision':revision,
                'scope':{'included':['runtime_databases','skill_databases','configuration','runtime_binaries','ui','admission','package_pointers'],
                    'excluded':['task_attachment_files','models','immutable_skill_package_payloads','external_connector_storage'],
                    'restore_requires_matching_installed_packages':True},'files':[]}
            paused_at=time.monotonic()
            with contextlib.ExitStack() as database_lifetime:
                try:
                    if not offline:
                        if not controller(mode,unit,'is-active'):raise RuntimeError('checkpoint_service_not_running')
                        write_json(root/'data/runtime-checkpoint-resume.json',{'schema_version':1,'service_mode':mode,'service_unit':unit})
                        stopped=True
                        controller(mode,unit,'stop')
                        idle(root)
                    pins=pin_databases(databases,database_lifetime)
                    freeze(root,frozen,files)
                    fence_deadline=time.monotonic()+read_fence_seconds
                    def guard():
                        if time.monotonic()>fence_deadline:raise RuntimeError('checkpoint_read_fence_timeout')
                        if not offline and shutil.disk_usage(root).free<reserve:raise RuntimeError('checkpoint_source_storage_pressure')
                    def pack(pin):
                        validate_pin(pin)
                        name='database-'+str(len(manifest['files']))+'.xz'
                        record=compress_file(pin['path'],staging/name,guard=guard)
                        if signature(pin['path'])!=pin['signature']:raise RuntimeError('checkpoint_database_file_changed')
                        record.update(path=str(pin['path'].relative_to(root)),archive=name,kind='sqlite')
                        manifest['files'].append(record)
                        pin['db'].close()
                    # Rollback-journal files need their read fences only during this small paused phase.
                    for pin in pins:
                        if pin['mode']!='wal':pack(pin);pin['db'].close()
                finally:
                    if stopped:resume(root,mode,unit);stopped=False
                manifest['runtime_pause_seconds']=round(time.monotonic()-paused_at,3) if not offline else 0
                if offline and revision!=subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip():
                    raise RuntimeError('checkpoint_source_changed')
                # At end-mark zero the reader prevents main-file checkpoint writes, not WAL commits.
                for pin in pins:
                    if pin['mode']=='wal':pack(pin)
                backup_files(frozen,staging,manifest)
                write_json(staging/'manifest.json',manifest)
                verify_checkpoint(staging)
            stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
            final=destination/('checkpoint-'+stamp+'-'+staging.name.removeprefix('.pending-'))
            os.rename(staging,final);staging=None
            fd=os.open(destination,os.O_RDONLY);os.fsync(fd);os.close(fd)
        finally:
            if staging is not None:shutil.rmtree(staging)
            if frozen is not None:shutil.rmtree(frozen)
            if stopped:resume(root,mode,unit)
        # Never retire an older checkpoint until a complete, verified new one exists.
        for old in sorted(recent,reverse=True)[max(0,retain-1):]:
            verify_checkpoint(old);shutil.rmtree(old)
        return {'schema_version':1,'status':'verified','checkpoint':str(final),'created_at_ts':manifest['created_at_ts'],
            'bytes':sum(p.stat().st_size for p in final.iterdir() if p.is_file()),'database_count':sum(r['kind']=='sqlite' for r in manifest['files']),
            'scope':manifest['scope'],'runtime_pause_seconds':manifest['runtime_pause_seconds']}


def main():
    os.umask(0o077)
    signal.signal(signal.SIGTERM,lambda *_:sys.exit(143))
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest='action',required=True)
    make=sub.add_parser('create');make.add_argument('--root',required=True,type=Path);make.add_argument('--destination',required=True,type=Path)
    make.add_argument('--service-mode',default='systemd-system');make.add_argument('--service-unit',default='agent-runtime.service')
    make.add_argument('--retain',type=int,default=1);make.add_argument('--reserve-bytes',type=int,default=2*1024**3)
    make.add_argument('--read-fence-seconds',type=int,default=1800,help='Bound database read fences to 1..3600 seconds; storage pressure still aborts immediately.')
    make.add_argument('--offline',action='store_true',help='Operator guarantees all non-database state is offline; SQLite writers are still fenced.')
    check=sub.add_parser('verify');check.add_argument('--checkpoint',required=True,type=Path)
    restore=sub.add_parser('restore');restore.add_argument('--checkpoint',required=True,type=Path);restore.add_argument('--output',required=True,type=Path);restore.add_argument('--reserve-bytes',type=int,default=128*1024**2);restore.add_argument('--databases-only',action='store_true')
    recovery=sub.add_parser('resume');recovery.add_argument('--root',required=True,type=Path);recovery.add_argument('--service-mode',default='systemd-system');recovery.add_argument('--service-unit',default='agent-runtime.service')
    args=parser.parse_args()
    try:
        if args.action=='create':
            if args.retain<1 or args.reserve_bytes<0:raise RuntimeError('checkpoint_invalid_limits')
            result=create(args.root,args.destination,mode=args.service_mode,unit=args.service_unit,retain=args.retain,reserve=args.reserve_bytes,offline=args.offline,read_fence_seconds=args.read_fence_seconds)
            write_json(args.root/'data/runtime-checkpoint-status.json',result)
        elif args.action=='resume':result=resume(args.root,args.service_mode,args.service_unit)
        elif args.action=='verify':result=verify_checkpoint(args.checkpoint)
        else:result=restore_checkpoint(args.checkpoint,args.output,reserve_bytes=args.reserve_bytes,databases_only=args.databases_only)
        print(json.dumps(result))
    except Exception as error:
        code=str(error) if isinstance(error,RuntimeError) and str(error).startswith('checkpoint_') else 'checkpoint_operation_failed'
        if args.action=='create':write_json(args.root/'data/runtime-checkpoint-status.json',{'schema_version':1,'status':'failed','error_code':code,'attempted_at_ts':int(time.time())})
        print(json.dumps({'status':'error','error_code':code}),file=sys.stderr);return 1
    return 0

if __name__=='__main__':sys.exit(main())
