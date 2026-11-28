#!/usr/bin/env python3
"""Durable phased backups: busy deferral, bounded retries and remote-only recovery."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import signal
import sqlite3
import sys
import time
import tomllib
import urllib.request

import runtime_checkpoint as checkpoint
from runtime_checkpoint_storage import write_json
from runtime_backup_local import checked_local, publish_local_status, recover_published_checkpoint
from runtime_backup_capacity import capacity_report, verified_source_revision

BUSY={'vault_remote_busy','checkpoint_foreground_not_idle','checkpoint_distributed_work_active','checkpoint_already_running'}
CAPACITY={'checkpoint_capacity_unavailable','checkpoint_destination_low_space','checkpoint_source_storage_pressure'}
PERMANENT={'vault_key_invalid','vault_key_not_private','vault_crypto_dependency_missing','vault_scope_invalid','vault_retention_invalid','vault_existing_object_corrupt','vault_attachments_exceed_capacity','checkpoint_cohort_mismatch','vault_receiver_not_committed'}


def sample_inventory(root):
    config=tomllib.loads((root/'configs/config.toml').read_text())['database']
    path=Path(config['sqlite_path']);path=path if path.is_absolute() else root/path
    with contextlib.closing(sqlite3.connect(path.as_uri()+'?mode=ro',uri=True,timeout=5)) as db:
        key=db.execute("SELECT user_key FROM auth_keys WHERE role='admin' AND enabled=1 LIMIT 1").fetchone()
    if not key:raise RuntimeError('backup_inventory_identity_unavailable')
    request=urllib.request.Request('http://127.0.0.1:8787/v1/system/storage-scan',data=b'{}',headers={'x-agent-key':key[0],'Content-Type':'application/json'})
    with urllib.request.urlopen(request,timeout=15) as response:
        if not json.load(response).get('ok'):raise RuntimeError('backup_inventory_unavailable')


def create_local(settings):
    result=checkpoint.create(settings['root'],settings['destination'],mode=settings['service_mode'],unit=settings['service_unit'],retain=1,reserve=settings['reserve_bytes'],read_fence_seconds=settings['read_fence_seconds'])
    write_json(settings['root']/'data/runtime-checkpoint-status.json',result)
    return result


def send_remote(settings,local):
    root=settings['root']
    publish_local_status(settings,local)
    if settings.get('vault_key_file'):
        from runtime_checkpoint_vault import RemoteTransport,build_snapshot
        from runtime_vault_cipher import load_key
        transport=RemoteTransport(settings['remote_host'],settings['remote_directory'].rstrip('/')+'/vault',Path(__file__).resolve().parents[1])
        report=build_snapshot(root,Path(local['checkpoint']),transport,load_key(settings['vault_key_file']),settings['scopes'],settings['retain_versions'])
        write_json(root/'data/runtime-vault-status.json',report)
    else:
        from runtime_checkpoint_remote import sync
        report=sync(root,settings['destination'],settings['remote_host'],settings['remote_directory'])
    if report.get('status') not in ('verified','verified_encrypted'):raise RuntimeError('backup_remote_ack_invalid')
    status=json.loads((root/'data/runtime-checkpoint-status.json').read_text())
    if status['checkpoint']!=local['checkpoint']:raise RuntimeError('backup_local_checkpoint_changed')
    status.update(status='verified_remote',remote_status='verified',remote_verified_at_ts=int(time.time()))
    status.pop('remote_error_code',None);write_json(root/'data/runtime-checkpoint-status.json',status)
    return report


def read_state(path):
    if not path.exists():return {}
    if path.is_symlink() or path.stat().st_size>65536:raise RuntimeError('backup_run_state_invalid')
    value=json.loads(path.read_text())
    if value.get('schema_version')!=1:raise RuntimeError('backup_run_state_invalid')
    return value


def run_cycle(settings,*,retry_only=False):
    root=settings['root'];path=root/'data/runtime-backup-schedule.json';now=int(time.time())
    state=read_state(path)
    policy={k:str(v) if isinstance(v,Path) else v for k,v in settings.items()}
    digest=hashlib.sha256(json.dumps(policy,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    if not state:state={'schema_version':1,'phase':'idle','attempt':0,'cycle_started_at_ts':now,'policy_digest':digest}
    changed=state.get('policy_digest') not in (None,digest)
    capacity=capacity_report(root,settings['destination'],settings['reserve_bytes'])
    try:current_revision=checkpoint.running_revision(root)
    except Exception:current_revision=None;capacity['cohort_probe_error_code']='checkpoint_cohort_unavailable'
    verified_revision=verified_source_revision(root,settings['destination'])
    current_known=isinstance(current_revision,str) and len(current_revision)==40 and all(c in '0123456789abcdef' for c in current_revision)
    capacity.update(current_source_revision=current_revision if current_known else None,
                    verified_source_revision=verified_revision,
                    current_source_covered=(current_revision==verified_revision) if current_known and verified_revision else None)
    write_json(root/'data/runtime-backup-capacity.json',capacity)
    cohort_recovered = (
        state.get('phase') == 'attention'
        and state.get('error_code') == 'checkpoint_cohort_mismatch'
        and current_known
    )
    changed = changed or cohort_recovered or (
        state.get('phase') == 'complete' and capacity['current_source_covered'] is False
    )
    # A small hourly telemetry sample supports capacity estimates. Failure of a
    # console sample must never suppress a backup or trigger a model request.
    if now-state.get('last_inventory_at_ts',0)>=3600:
        try:sample_inventory(root);state['last_inventory_at_ts']=now
        except Exception:state['inventory_error_code']='backup_inventory_unavailable'
    if retry_only and not changed:
        if state.get('phase') in (None,'idle','complete','attention'):
            if state:write_json(path,state)
            return {'status':'not_due','phase':state.get('phase','idle')}
        if state.get('next_retry_at_ts',0)>now:return {'status':'not_due','phase':state['phase']}
    if changed or (not retry_only and (state.get('phase') in ('idle','complete','attention') or now-state.get('cycle_started_at_ts',0)>=86400)):
        state={'schema_version':1,'cycle_started_at_ts':now,'phase':'local','attempt':0,'policy_digest':digest,'last_inventory_at_ts':state.get('last_inventory_at_ts',0)}
    if now-state['cycle_started_at_ts']>=86400:
        state.update(phase='attention',error_code='backup_retry_deadline_expired',notice_key='backup.attention_required');state.pop('next_retry_at_ts',None);write_json(path,state)
        return {'status':'attention_required','error_code':state['error_code']}
    phase=state.get('resume_phase',state['phase'])
    if phase not in ('local','remote'):raise RuntimeError('backup_run_phase_invalid')
    if phase=='local' and capacity['status']!='ready':
        code='checkpoint_capacity_unavailable' if capacity['status']=='unavailable' else ('checkpoint_destination_low_space' if capacity['destination_shortfall_bytes'] else 'checkpoint_source_storage_pressure')
        state.update(phase='deferred',resume_phase='local',error_code=code,notice_key='backup.capacity_deferred',next_retry_at_ts=now+900)
        write_json(path,state)
        return {'status':'retry_scheduled','phase':'deferred','error_code':code,'attempt':state['attempt']}
    try:
        if phase=='remote':
            try:local=checked_local(settings,state)
            except (FileNotFoundError,KeyError):phase='local'
            except RuntimeError as error:
                if str(error) in ('backup_run_cohort_changed','backup_run_checkpoint_stale'):phase='local'
                else:raise
        if phase=='local':
            state.update(phase='local',resume_phase='local');state.pop('next_retry_at_ts',None);write_json(path,state)
            local=recover_published_checkpoint(settings,state)
            if local is None:local=create_local(settings)
            state.update(phase='remote',resume_phase='remote',checkpoint=local['checkpoint']);write_json(path,state)
            phase='remote'
        if settings.get('remote_host'):
            state.update(phase='remote',resume_phase='remote');write_json(path,state)
            send_remote(settings,local)
        state.update(phase='complete',completed_at_ts=int(time.time()),notice_key='backup.verified')
        for key in ('error_code','next_retry_at_ts','resume_phase'):state.pop(key,None)
        write_json(path,state)
        return {'status':'verified','phase':'complete','attempt':state['attempt']}
    except Exception as error:
        code=str(error) if isinstance(error,RuntimeError) and str(error).startswith(('checkpoint_','vault_','backup_')) else 'backup_operation_failed'
        if isinstance(error,BlockingIOError):code='checkpoint_already_running'
        state.update(error_code=code,resume_phase=phase)
        if code=='vault_pinned_package_missing':state['resume_phase']='local'
        if code in BUSY or code in CAPACITY:
            state.update(phase='deferred',notice_key='backup.capacity_deferred' if code in CAPACITY else 'backup.busy',next_retry_at_ts=int(time.time())+900)
        else:
            state['attempt']=state.get('attempt',0)+1
            if code in PERMANENT or state['attempt']>=settings['retry_limit']:
                state.update(phase='attention',notice_key='backup.attention_required');state.pop('next_retry_at_ts',None)
            else:
                delay=min(10800,900*(2**(state['attempt']-1)))
                state.update(phase='retry_pending',notice_key='backup.retry_pending',next_retry_at_ts=int(time.time())+delay)
        write_json(path,state)
        return {'status':'attention_required' if state['phase']=='attention' else 'retry_scheduled','phase':state['phase'],'error_code':code,'attempt':state['attempt']}


def main():
    os.umask(0o077)
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root',type=Path,required=True);p.add_argument('--destination',type=Path,required=True)
    p.add_argument('--remote-host');p.add_argument('--remote-directory');p.add_argument('--vault-key-file',type=Path)
    p.add_argument('--scopes',nargs='*',default=['packages','models']);p.add_argument('--retain-versions',type=int,default=3)
    p.add_argument('--retry-limit',type=int,default=6);p.add_argument('--retry-only',action='store_true')
    p.add_argument('--service-mode',default='systemd-system');p.add_argument('--service-unit',default='agent-runtime.service')
    p.add_argument('--reserve-bytes',type=int,default=2*1024**3);p.add_argument('--read-fence-seconds',type=int,default=1800)
    args=p.parse_args()
    if sys.platform not in ('linux','darwin'):
        print(json.dumps({'status':'unsupported','error_code':'backup_platform_unsupported'}));return 1
    import fcntl
    for signum in (signal.SIGTERM,signal.SIGINT,signal.SIGHUP):signal.signal(signum,lambda *_:sys.exit(143))
    try:
        settings=vars(args).copy();retry=settings.pop('retry_only')
        settings['root']=settings['root'].resolve();settings['destination']=settings['destination'].resolve()
        if not 1<=settings['retry_limit']<=12 or not 1<=settings['retain_versions']<=8:raise RuntimeError('backup_limits_invalid')
        if settings['reserve_bytes']<0 or not 1<=settings['read_fence_seconds']<=3600:raise RuntimeError('backup_limits_invalid')
        from runtime_vault_payloads import SCOPES
        if not set(settings['scopes']).issubset(SCOPES):raise RuntimeError('backup_scopes_invalid')
        if bool(settings['remote_host'])!=bool(settings['remote_directory']) or (settings['vault_key_file'] and not settings['remote_host']):raise RuntimeError('backup_remote_target_missing')
        if settings['remote_host']:
            from runtime_checkpoint_remote import validate_target
            validate_target(settings['remote_host'],settings['remote_directory'])
        data=settings['root']/'data';data.mkdir(exist_ok=True)
        with (data/'.backup-schedule.lock').open('a') as lock:
            try:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            except BlockingIOError:
                print(json.dumps({'status':'already_running'}));return 0
            report=run_cycle(settings,retry_only=retry)
        print(json.dumps(report));return int(report['status']=='attention_required')
    except Exception as error:
        code=str(error) if isinstance(error,RuntimeError) and str(error).startswith(('backup_','vault_','checkpoint_')) else 'backup_run_failed'
        print(json.dumps({'status':'error','error_code':code}));return 1


if __name__=='__main__':raise SystemExit(main())
