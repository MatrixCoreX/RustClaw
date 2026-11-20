#!/usr/bin/env python3
"""Conservative retention policy and lossless relocation of cold rollback archives."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from runtime_checkpoint_storage import digest, write_json

POLICY={
    'task_files':'explicit_user_delete_only',
    'models':'explicit_user_delete_only',
    'installed_packages':'admission_and_active_leases',
    'validation_evidence':'preserve',
    'rollback_archives':'preserve_or_verified_relocation',
    'local_checkpoints':{'verified_versions':1,'retire_after_verified_replacement':True},
    'encrypted_vault':{'default_versions':3,'maximum_versions':8,'retire_only_indexed_unreferenced_objects':True},
    'previous_run_logs':{'verified_compressed_runs_per_component':20},
}


def signature(path):
    st=path.stat();return (st.st_dev,st.st_ino,st.st_size,st.st_mtime_ns)


def filesystem_id(path):return path.stat().st_dev


def active_references(path):
    owner=path.stat().st_uid
    if sys.platform=='linux':
        target=path.resolve();references=[]
        for proc in Path('/proc').iterdir():
            if not proc.name.isdecimal():continue
            try:
                if proc.stat().st_uid!=owner:continue
                for descriptor in (proc/'fd').iterdir():
                    try:
                        value=Path(os.readlink(descriptor))
                        if value.is_absolute() and value.resolve()==target:references.append(int(proc.name))
                    except FileNotFoundError:continue
                    except PermissionError:raise RuntimeError('storage_reference_audit_unavailable')
            except FileNotFoundError:continue
            except PermissionError:raise RuntimeError('storage_reference_audit_unavailable')
        return references
    if sys.platform=='darwin':
        if not shutil.which('lsof'):raise RuntimeError('storage_reference_audit_unavailable')
        result=subprocess.run(['lsof','-t','-a','-u',str(owner),'--',str(path)],capture_output=True,text=True,timeout=15)
        if result.returncode not in (0,1) or result.stderr.strip():raise RuntimeError('storage_reference_audit_unavailable')
        return [int(value) for value in result.stdout.split()]
    raise RuntimeError('storage_relocation_platform_unsupported')


def relocate(root,source,destination,*,reserve=2*1024**3,apply=False):
    if sys.platform not in ('linux','darwin'):raise RuntimeError('storage_relocation_platform_unsupported')
    root=Path(root).resolve();source=Path(source).absolute();destination=Path(destination).absolute()
    if reserve<0:raise RuntimeError('storage_reserve_invalid')
    # These immutable deployment archives have no editable user work. Models,
    # task attachments, databases, skill packages and logs are never relocated
    # implicitly or selected through an arbitrary glob.
    if source.is_symlink() or not source.is_file() or not source.resolve().is_relative_to(root/'.release-backups') or source.name!='files.tar.gz':
        raise RuntimeError('storage_relocation_scope_invalid')
    before=signature(source);meta=source.stat()
    if meta.st_nlink!=1:raise RuntimeError('storage_relocation_hardlinked_source')
    if active_references(source):raise RuntimeError('storage_archive_active')
    if destination.is_symlink() or destination.resolve().is_relative_to(root):raise RuntimeError('storage_destination_invalid')
    probe=destination
    while not probe.exists():probe=probe.parent
    if destination.exists() and (destination.stat().st_uid!=meta.st_uid or destination.stat().st_mode&0o077):raise RuntimeError('storage_destination_not_private')
    if filesystem_id(source)==filesystem_id(probe):raise RuntimeError('storage_requires_other_filesystem')
    sha=digest(source);target=destination/('archive-'+sha+'.tar.gz')
    if target.is_symlink():raise RuntimeError('storage_destination_symlink')
    reusable=target.is_file() and target.stat().st_size==meta.st_size and digest(target)==sha
    if target.exists() and not reusable:raise RuntimeError('storage_destination_conflict')
    if not reusable and shutil.disk_usage(probe).free<meta.st_size+reserve:raise RuntimeError('storage_destination_low_space')
    report={'schema_version':1,'mode':'apply' if apply else 'preview','source':str(source),'destination':str(target),'bytes':meta.st_size,'sha256':sha,'preserves_original_path':True}
    if not apply:return report
    destination.mkdir(parents=True,exist_ok=True,mode=0o700)
    if destination.is_symlink() or destination.stat().st_uid!=meta.st_uid or destination.stat().st_mode&0o077:raise RuntimeError('storage_destination_not_private')
    temp=None;link=None
    try:
        if not reusable:
            fd,name=tempfile.mkstemp(prefix='.relocating-',dir=destination);os.close(fd);temp=Path(name)
            with source.open('rb') as original,temp.open('wb') as output:
                while data:=original.read(1024**2):
                    if shutil.disk_usage(destination).free<reserve+len(data):raise RuntimeError('storage_destination_low_space')
                    output.write(data)
                output.flush();os.fsync(output.fileno())
            shutil.copystat(source,temp)
            if digest(temp)!=sha or signature(source)!=before:raise RuntimeError('storage_source_changed')
            os.replace(temp,target);temp=None
            fd=os.open(destination,os.O_RDONLY)
            try:os.fsync(fd)
            finally:os.close(fd)
        if signature(source)!=before or active_references(source):raise RuntimeError('storage_source_changed_or_active')
        fd,name=tempfile.mkstemp(prefix='.archive-link-',dir=source.parent);os.close(fd);link=Path(name);link.unlink();link.symlink_to(target)
        os.replace(link,source);link=None
        fd=os.open(source.parent,os.O_RDONLY)
        try:os.fsync(fd)
        finally:os.close(fd)
        if digest(source)!=sha:raise RuntimeError('storage_relocation_verification_failed')
        report.update(status='relocated_verified',completed_at_ts=int(time.time()))
        journal=root/'data/storage-relocations.json';previous=json.loads(journal.read_text()) if journal.exists() else {'schema_version':1,'archives':[]}
        previous['archives'].append(report);write_json(journal,previous)
        return report
    finally:
        for path in (temp,link):
            if path is not None and (path.exists() or path.is_symlink()):path.unlink()


def main():
    os.umask(0o077)
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest='action',required=True)
    sub.add_parser('policy')
    move=sub.add_parser('relocate');move.add_argument('--root',type=Path,required=True);move.add_argument('--source',type=Path,required=True);move.add_argument('--destination',type=Path,required=True);move.add_argument('--reserve-bytes',type=int,default=2*1024**3);move.add_argument('--apply',action='store_true')
    args=p.parse_args()
    try:
        value={'schema_version':1,'policy':POLICY} if args.action=='policy' else relocate(args.root,args.source,args.destination,reserve=args.reserve_bytes,apply=args.apply)
        print(json.dumps(value));return 0
    except Exception as error:
        code=str(error) if isinstance(error,RuntimeError) and str(error).startswith('storage_') else 'storage_lifecycle_failed'
        print(json.dumps({'status':'error','error_code':code}));return 1


if __name__=='__main__':raise SystemExit(main())
