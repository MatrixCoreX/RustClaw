"""Private checkpoint codecs and restore-to-new-directory validation."""
import contextlib
import hashlib
import json
import lzma
import os
from pathlib import Path
import shutil
import sqlite3
import tarfile
import tempfile


def write_json(path, value):
    path=Path(path);path.parent.mkdir(parents=True,exist_ok=True)
    fd,name=tempfile.mkstemp(prefix='.'+path.name,dir=path.parent)
    try:
        with os.fdopen(fd,'w') as stream:
            json.dump(value,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
        os.replace(name,path)
    finally:
        if os.path.exists(name):os.unlink(name)


def digest(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def compress_file(source, output):
    raw=hashlib.sha256();size=0
    with source.open('rb') as stream, lzma.open(output,'xb',preset=3) as compressed:
        while chunk:=stream.read(1024*1024):compressed.write(chunk);raw.update(chunk);size+=len(chunk)
    with output.open('rb') as stream:os.fsync(stream.fileno())
    return {'raw_bytes':size,'raw_sha256':raw.hexdigest(),'bytes':output.stat().st_size,'sha256':digest(output)}


def selected_files(root):
    roots=[root/'configs',root/'data/skills/.runtime-admission',root/'target/release',root/'UI/dist',
        root/'.agent-runtime/credentials',root/'.agent-runtime/git']
    files=[]
    for base in roots:
        if not base.exists():continue
        for directory,dirs,names in os.walk(base,followlinks=False):
            dirs[:]=[name for name in dirs if not (Path(directory)/name).is_symlink()]
            for name in names:
                path=Path(directory)/name
                if path.is_file() and not path.is_symlink():files.append(path)
    for path in (root/'data/skill-packages').glob('*/current.json'):
        if not path.is_symlink():files.append(path)
    for name in ['.release-tag','.release-manifest-digest']:
        if (root/name).is_file():files.append(root/name)
    return sorted(set(files))


def backup_files(root, staging, manifest):
    output=staging/'control-files.tar.xz'
    with tarfile.open(output,'x:xz',preset=3,format=tarfile.PAX_FORMAT,dereference=True) as archive:
        for path in selected_files(root):
            if not path.resolve().is_relative_to(root):raise RuntimeError('checkpoint_source_escaped')
            archive.add(path,arcname=str(path.relative_to(root)),recursive=False)
            manifest['files'].append({'kind':'file','path':str(path.relative_to(root)),'raw_bytes':path.stat().st_size,
                'raw_sha256':digest(path),'mode':path.stat().st_mode&0o777})
    manifest['control_archive']={'name':output.name,'bytes':output.stat().st_size,'sha256':digest(output)}
    with output.open('rb') as stream:os.fsync(stream.fileno())


def relative(value):
    path=Path(value)
    if path.is_absolute() or '..' in path.parts or not path.parts:raise RuntimeError('checkpoint_path_invalid')
    return path


def manifest(checkpoint):
    checkpoint=Path(checkpoint)
    if checkpoint.is_symlink():raise RuntimeError('checkpoint_root_symlink')
    value=json.loads((checkpoint/'manifest.json').read_text())
    if value.get('schema_version')!=1 or value.get('kind')!='runtime_control_checkpoint':raise RuntimeError('checkpoint_schema_invalid')
    paths=[row['path'] for row in value['files']]
    if len(paths)!=len(set(paths)):raise RuntimeError('checkpoint_duplicate_path')
    for path in paths:relative(path)
    return value


def verify_checkpoint(checkpoint):
    checkpoint=Path(checkpoint);value=manifest(checkpoint)
    for row in [r for r in value['files'] if r['kind']=='sqlite']:
        path=checkpoint/relative(row['archive'])
        if path.is_symlink() or digest(path)!=row['sha256']:raise RuntimeError('checkpoint_archive_digest_mismatch')
        size=0;raw=hashlib.sha256()
        with lzma.open(path,'rb') as stream:
            while chunk:=stream.read(1024*1024):
                size+=len(chunk)
                if size>row['raw_bytes']:raise RuntimeError('checkpoint_database_size_mismatch')
                raw.update(chunk)
        if size!=row['raw_bytes'] or raw.hexdigest()!=row['raw_sha256']:raise RuntimeError('checkpoint_database_digest_mismatch')
    control=value['control_archive'];path=checkpoint/relative(control['name'])
    if path.is_symlink() or digest(path)!=control['sha256']:raise RuntimeError('checkpoint_archive_digest_mismatch')
    expected={r['path']:r for r in value['files'] if r['kind']=='file'};seen=set()
    with tarfile.open(path,'r:xz') as archive:
        for entry in archive:
            relative(entry.name)
            if not entry.isfile() or entry.name not in expected or entry.name in seen:raise RuntimeError('checkpoint_member_invalid')
            row=expected[entry.name]
            if entry.size!=row['raw_bytes']:raise RuntimeError('checkpoint_member_size_mismatch')
            with archive.extractfile(entry) as stream:observed=hashlib.file_digest(stream,'sha256').hexdigest()
            if observed!=row['raw_sha256']:raise RuntimeError('checkpoint_member_digest_mismatch')
            seen.add(entry.name)
    if seen!=set(expected):raise RuntimeError('checkpoint_member_missing')
    return {'schema_version':1,'status':'verified','source_revision':value['source_revision'],'files':len(value['files'])}


def restore_checkpoint(checkpoint, output):
    checkpoint=Path(checkpoint);output=Path(output);verify_checkpoint(checkpoint);value=manifest(checkpoint)
    if output.exists() or output.is_symlink():raise RuntimeError('checkpoint_output_exists')
    required=sum(r['raw_bytes'] for r in value['files'])
    parent=output.parent.resolve();parent.mkdir(parents=True,exist_ok=True)
    if shutil.disk_usage(parent).free<required+128*1024**2:raise RuntimeError('checkpoint_restore_low_space')
    output.mkdir(mode=0o700)
    try:
        with tarfile.open(checkpoint/value['control_archive']['name'],'r:xz') as archive:
            for entry in archive:
                target=output/relative(entry.name);target.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
                with archive.extractfile(entry) as stream,target.open('xb') as dest:shutil.copyfileobj(stream,dest)
                target.chmod(entry.mode&0o700)
        for row in value['files']:
            target=output/relative(row['path'])
            if row['kind']=='sqlite':
                target.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
                with lzma.open(checkpoint/relative(row['archive']),'rb') as stream,target.open('xb') as dest:shutil.copyfileobj(stream,dest)
                with contextlib.closing(sqlite3.connect(target.as_uri()+'?mode=ro',uri=True)) as db:
                    if db.execute('PRAGMA quick_check').fetchone()[0]!='ok':raise RuntimeError('checkpoint_restored_database_invalid')
            if digest(target)!=row['raw_sha256']:raise RuntimeError('checkpoint_restore_digest_mismatch')
        write_json(output/'checkpoint-restore.json',{'source_revision':value['source_revision'],'scope':value['scope'],'activated':False})
        return {'schema_version':1,'status':'restored_verified','output':str(output),'files':len(value['files']),'activated':False}
    except BaseException:
        shutil.rmtree(output);raise
