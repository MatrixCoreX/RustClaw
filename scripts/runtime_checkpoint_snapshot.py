"""Pin fully checkpointed SQLite main files; resume WAL writers before compression."""
import contextlib
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import tempfile


def snapshot_parent(required):
    candidates=[Path(tempfile.gettempdir())]
    if sys.platform=='linux' and Path('/dev/shm').is_dir():candidates.insert(0,Path('/dev/shm'))
    for parent in candidates:
        margin=max(65536,min(128*1024**2,required//4+65536))
        if shutil.disk_usage(parent).free<required+margin:continue
        memory_backed=parent==Path('/dev/shm')
        if sys.platform=='linux':
            mounts=[]
            for line in Path('/proc/self/mountinfo').read_text().splitlines():
                before,after=line.split(' - ',1);point=Path(before.split()[4])
                if parent.resolve().is_relative_to(point):mounts.append((len(point.parts),after.split()[0]))
            memory_backed=bool(mounts and max(mounts)[1]=='tmpfs')
        if memory_backed:
            try:
                entries=dict(line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())
                available=int(entries['MemAvailable'].strip().split()[0])*1024
            except (OSError,ValueError,KeyError):continue
            if available<required+1024**3:continue
        # statvfs cannot see per-user tmpfs quotas. Reserve before stopping a service.
        fd,name=tempfile.mkstemp(prefix='checkpoint-reservation-',dir=parent)
        try:
            if hasattr(os,'posix_fallocate'):os.posix_fallocate(fd,0,required+margin)
            else:
                remaining=required+margin
                while remaining:
                    count=min(remaining,1024*1024);os.write(fd,b'\0'*count);remaining-=count
            return parent
        except OSError as error:
            if error.errno not in (28,122):raise
        finally:os.close(fd);os.unlink(name)
    raise RuntimeError('checkpoint_snapshot_space_or_quota_unavailable')


def signature(path):
    metadata=path.stat()
    return metadata.st_dev,metadata.st_ino,metadata.st_size,metadata.st_mtime_ns


def pin_databases(databases, lifetime):
    pins=[]
    for path in databases:
        db=sqlite3.connect(str(path),timeout=10);lifetime.callback(db.close)
        mode=db.execute('PRAGMA journal_mode').fetchone()[0]
        result=db.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
        if result and result[0]!=0:raise RuntimeError('checkpoint_database_busy')
        db.execute('BEGIN')
        db.execute('SELECT count(*) FROM sqlite_master').fetchone()  # Establish the read lock.
        if mode=='wal':
            wal=Path(str(path)+'-wal')
            # A new commit before our read fence means the main file alone is not the snapshot.
            if wal.exists() and wal.stat().st_size:raise RuntimeError('checkpoint_database_changed_during_pin')
        pins.append({'path':path,'db':db,'mode':mode,'signature':signature(path)})
    return pins


def validate_pin(pin):
    if signature(pin['path'])!=pin['signature']:raise RuntimeError('checkpoint_database_file_changed')
    if pin['db'].execute('PRAGMA quick_check').fetchone()[0]!='ok':raise RuntimeError('checkpoint_database_invalid')


def freeze(root, output, files):
    for source in files:
        relative=source.relative_to(root)
        if not source.resolve().is_relative_to(root) or source.is_symlink():raise RuntimeError('checkpoint_source_escaped')
        target=output/relative;target.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
        shutil.copyfile(source,target);target.chmod(source.stat().st_mode&0o700)
