"""Freeze a control-plane cohort before resuming the service; compress afterwards."""
import contextlib
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import tempfile


def snapshot_parent(required):
    candidates=[Path(tempfile.gettempdir())]
    if sys.platform=='linux' and Path('/dev/shm').is_dir():candidates.append(Path('/dev/shm'))
    for parent in candidates:
        if shutil.disk_usage(parent).free<required+128*1024**2:continue
        if parent==Path('/dev/shm'):
            try:
                entries=dict(line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())
                available=int(entries['MemAvailable'].strip().split()[0])*1024
            except (OSError,ValueError,KeyError):continue
            if available<required+1024**3:continue
        return parent
    raise RuntimeError('checkpoint_snapshot_space_unavailable')


def freeze(root, output, databases, files):
    with contextlib.ExitStack() as stack:
        for path in databases:
            db=sqlite3.connect(str(path),timeout=10);stack.callback(db.close)
            result=db.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
            if result and result[0]!=0:raise RuntimeError('checkpoint_database_busy')
            db.execute('BEGIN IMMEDIATE')
            if db.execute('PRAGMA quick_check').fetchone()[0]!='ok':raise RuntimeError('checkpoint_database_invalid')
        for source in databases+files:
            relative=source.relative_to(root)
            if not source.resolve().is_relative_to(root) or source.is_symlink():raise RuntimeError('checkpoint_source_escaped')
            target=output/relative;target.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
            shutil.copyfile(source,target);target.chmod(source.stat().st_mode&0o700)
    return [str(p.relative_to(root)) for p in databases]
