"""Recover only private, verified checkpoints published by the current backup cycle."""
import json
import os
from pathlib import Path

import runtime_checkpoint as checkpoint
from runtime_checkpoint_storage import manifest, verify_checkpoint, write_json

READY={'verified','verified_remote','verified_local_remote_failed'}


def read_status(root):
    path=root/'data/runtime-checkpoint-status.json'
    if not path.exists():return {}
    if path.is_symlink() or path.stat().st_size>65536:raise RuntimeError('backup_local_status_invalid')
    return json.loads(path.read_text())


def private_checkpoint(settings,path):
    destination=settings['destination'];path=Path(path)
    for directory in (destination,path):
        metadata=directory.stat()
        if directory.is_symlink() or not directory.is_dir() or metadata.st_uid!=os.geteuid() or metadata.st_mode&0o077:raise RuntimeError('backup_run_checkpoint_invalid')
    if path.parent.resolve()!=destination.resolve() or not path.name.startswith('checkpoint-'):raise RuntimeError('backup_run_checkpoint_invalid')
    return path


def checked_local(settings,state):
    path=private_checkpoint(settings,state['checkpoint']);value=manifest(path)
    if value['created_at_ts']<state['cycle_started_at_ts']:raise RuntimeError('backup_run_checkpoint_stale')
    if value['source_revision']!=checkpoint.running_revision(settings['root']):raise RuntimeError('backup_run_cohort_changed')
    return {'checkpoint':str(path),'created_at_ts':value['created_at_ts']}


def publish_local_status(settings,local):
    path=private_checkpoint(settings,local['checkpoint']);current=read_status(settings['root'])
    if current.get('checkpoint')==str(path) and current.get('status') in READY:return
    if current.get('checkpoint') and Path(current['checkpoint']).exists():
        previous=private_checkpoint(settings,current['checkpoint'])
        if manifest(previous)['created_at_ts']>=local['created_at_ts']:raise RuntimeError('backup_local_checkpoint_changed')
    # Recovery never declares a directory verified from its name or journal alone.
    verify_checkpoint(path);value=manifest(path)
    result={'schema_version':1,'status':'verified','checkpoint':str(path),'created_at_ts':value['created_at_ts'],
            'bytes':sum(p.stat().st_size for p in path.iterdir() if p.is_file()),'database_count':sum(row['kind']=='sqlite' for row in value['files']),
            'scope':value['scope'],'runtime_pause_seconds':value['runtime_pause_seconds']}
    write_json(settings['root']/'data/runtime-checkpoint-status.json',result)


def recover_published_checkpoint(settings,state):
    current=read_status(settings['root'])
    candidates=[]
    if current.get('status') in READY and current.get('checkpoint'):candidates.append(Path(current['checkpoint']))
    candidates.extend(sorted(settings['destination'].glob('checkpoint-*'),reverse=True))
    seen=set()
    for path in candidates:
        if path in seen:continue
        seen.add(path)
        try:local=checked_local(settings,{**state,'checkpoint':str(path)})
        except FileNotFoundError:continue
        except RuntimeError as error:
            if str(error) in ('backup_run_checkpoint_stale','backup_run_cohort_changed'):continue
            raise
        publish_local_status(settings,local)
        return local
    return None
