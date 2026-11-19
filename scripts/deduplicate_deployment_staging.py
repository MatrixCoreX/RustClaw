#!/usr/bin/env python3
"""Lossless deduplication of old staging files. Paths and bytes are retained."""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import tempfile

from clean_dev_artifacts import require_idle


def deduplicate(root,apply=False):
    staging=(root/'.deploy-staging').resolve()
    if staging!=root.resolve()/'.deploy-staging':raise RuntimeError('cleanup_staging_symlink')
    grouped=collections.defaultdict(list)
    for directory,dirs,names in os.walk(staging,followlinks=False):
        dirs[:]=[n for n in dirs if not (Path(directory)/n).is_symlink()]
        for name in names:
            path=Path(directory)/name;meta=path.lstat()
            if stat.S_ISREG(meta.st_mode) and meta.st_uid==os.getuid() and meta.st_size>=1024*1024:
                grouped[(meta.st_size,meta.st_mode&0o777,meta.st_dev)].append(path)
    changes=[];saved=0
    for (_,mode,_),paths in grouped.items():
        hashes=collections.defaultdict(list)
        for path in paths:
            if len(paths)<2:continue
            with path.open('rb') as stream:key=hashlib.file_digest(stream,'sha256').hexdigest()
            hashes[key].append(path)
        for digest,duplicates in hashes.items():
            canonical=duplicates[0]
            for path in duplicates[1:]:
                if path.stat().st_ino==canonical.stat().st_ino:continue
                changes.append({'path':str(path),'retained':str(canonical),'sha256':digest,'bytes':path.stat().st_size})
                if apply:
                    require_idle([path,canonical])
                    with path.open('rb') as stream:assert hashlib.file_digest(stream,'sha256').hexdigest()==digest
                    fd,temp=tempfile.mkstemp(prefix='.deduplicate-',dir=path.parent);os.close(fd);os.unlink(temp)
                    try:
                        os.link(canonical,temp);os.replace(temp,path)
                        with path.open('rb') as stream:assert hashlib.file_digest(stream,'sha256').hexdigest()==digest
                    finally:
                        if os.path.exists(temp):os.unlink(temp)
                saved+=path.stat().st_size
    return {'schema_version':1,'mode':'apply' if apply else 'preview','deduplicated_bytes':saved,'files':changes}

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',required=True,type=Path);p.add_argument('--apply',action='store_true');a=p.parse_args()
    try:print(json.dumps(deduplicate(a.root,a.apply),indent=2))
    except Exception:print(json.dumps({'error_code':'cleanup_staging_audit_failed'}),file=sys.stderr);sys.exit(1)
