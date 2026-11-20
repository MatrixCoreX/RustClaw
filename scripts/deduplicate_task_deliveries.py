#!/usr/bin/env python3
"""Share identical immutable delivery snapshots; retain every path and result.

Preview is read-only. Apply requires an idle/stopped runtime and a complete
process-reference audit. Invocation/source files are never eligible. Host
publication replaces a path atomically, so a later revision cannot edit another
snapshot through its shared inode.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys
import tempfile
import tomllib
import uuid

from clean_dev_artifacts import require_idle
from runtime_checkpoint_storage import write_json

TERMINAL = {'succeeded', 'failed', 'timeout', 'canceled'}
MINIMUM_BYTES = 1024 * 1024


def signature(path):
    s = path.lstat()
    return s.st_dev, s.st_ino, s.st_size, s.st_mode, s.st_uid, s.st_gid, s.st_mtime_ns, s.st_ctime_ns


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def private_database(root):
    config = tomllib.loads((root / 'configs/config.toml').read_text())
    path = Path(config['database']['sqlite_path'])
    return path if path.is_absolute() else root / path


def task_record(db, task):
    row = db.execute('SELECT principal_id,status,result_json FROM tasks WHERE task_id=?', (task,)).fetchone()
    if not row or not row[0] or row[1] not in TERMINAL:
        return None
    try:
        result = json.loads(row[2] or '{}')
    except (ValueError, UnicodeError):
        return None
    artifacts = result.get('artifacts', []) if isinstance(result, dict) else []
    if not isinstance(artifacts, list):
        return None
    return row[0], artifacts


def matches_manifest(row, path, size, digest):
    if row is None:
        return False
    for artifact in row[1]:
        if not isinstance(artifact, dict):
            continue
        if (artifact.get('schema_version') == 2 and artifact.get('filename') == path.name
                and artifact.get('size_bytes') == size and artifact.get('sha256') == digest
                and isinstance(artifact.get('id'), str)
                and artifact.get('artifact_ref') == 'artifact:task/' + path.parents[1].name + '/' + artifact['id']):
            return True
    return False


def collect(root, db, minimum_bytes):
    delivery = root / '.agent-runtime/artifacts/delivery'
    if not delivery.exists():
        return {}, []
    if delivery.is_symlink() or delivery.resolve() != delivery:
        raise RuntimeError('delivery_storage_scope_invalid')
    groups = collections.defaultdict(list)
    skipped = []
    records = {}
    for path in sorted(delivery.glob('*/*/*')):
        relative = path.relative_to(delivery)
        try:
            task = str(uuid.UUID(relative.parts[0]))
        except ValueError:
            continue
        if task != relative.parts[0] or any(part.startswith('.') for part in relative.parts):
            continue
        if any(p.is_symlink() for p in [path, path.parent, path.parents[1]]):
            continue
        before = signature(path)
        if not stat.S_ISREG(before[3]) or before[4] != os.getuid() or before[2] < minimum_bytes:
            continue
        if task not in records:
            records[task] = task_record(db, task)
        record = records[task]
        if record is None:
            continue
        candidates = [a for a in record[1] if isinstance(a, dict) and a.get('filename') == path.name and a.get('size_bytes') == before[2]]
        if not candidates:
            continue
        digest = sha(path)
        if signature(path) != before:
            raise RuntimeError('delivery_storage_source_changed')
        if not matches_manifest(record, path, before[2], digest):
            skipped.append({'task_id': task, 'reason_code': 'manifest_digest_mismatch'})
            continue
        key = record[0], digest, before[2], stat.S_IMODE(before[3]), before[4], before[5], before[0]
        groups[key].append({'path': path, 'task_id': task, 'signature': before, 'link_count': path.stat().st_nlink})
    return groups, skipped


def deduplicate(root, *, apply=False, minimum_bytes=MINIMUM_BYTES, idle=require_idle):
    root = Path(root).absolute()
    if root.resolve() != root or minimum_bytes < 65536:
        raise RuntimeError('delivery_storage_scope_invalid')
    executable = root / 'target/release/clawd'
    delivery = root / '.agent-runtime/artifacts/delivery'
    if apply:
        # This also refuses incomplete Linux/macOS reference audits and an open
        # daemon executable. Preview never requires or stops the service.
        if not executable.is_file() or executable.is_symlink():
            raise RuntimeError('delivery_storage_runtime_identity_missing')
        idle([executable, delivery])
    uri = private_database(root).resolve().as_uri() + '?mode=ro'
    with sqlite3.connect(uri, uri=True, timeout=5) as db:
        db.execute('PRAGMA query_only=ON')
        groups, skipped = collect(root, db, minimum_bytes)
        changes = []
        reclaimable = 0
        for key, rows in groups.items():
            # Existing shared inodes are safe only when every link is another
            # owner-bound delivery snapshot in this exact eligible group.
            inode_counts = collections.Counter(row['signature'][1] for row in rows)
            private_rows = []
            for row in rows:
                if row['link_count'] != inode_counts[row['signature'][1]]:
                    skipped.append({'task_id': row['task_id'], 'reason_code': 'existing_external_hardlink'})
                else:
                    private_rows.append(row)
            rows = private_rows
            if not rows:
                continue
            canonical = rows[0]
            inode = canonical['signature'][1]
            for row in rows[1:]:
                original = row['signature']
                if original[1] == inode:
                    continue
                if row['link_count'] != 1:
                    skipped.append({'task_id': row['task_id'], 'reason_code': 'existing_external_hardlink'})
                    continue
                path, source = row['path'], canonical['path']
                size, digest = key[2], key[1]
                if apply:
                    # Revalidate both owner-bound manifests and physical files
                    # after hashing, immediately before replacing a path.
                    for candidate in [row, canonical]:
                        current = task_record(db, candidate['task_id'])
                        if current is None or current[0] != key[0] or not matches_manifest(current, candidate['path'], size, digest):
                            raise RuntimeError('delivery_storage_manifest_changed')
                        if signature(candidate['path']) != candidate['signature'] or sha(candidate['path']) != digest:
                            raise RuntimeError('delivery_storage_source_changed')
                    fd, temporary = tempfile.mkstemp(prefix='.delivery-dedup-', dir=path.parent)
                    os.close(fd)
                    os.unlink(temporary)
                    try:
                        os.link(source, temporary)
                        os.replace(temporary, path)
                        fd = os.open(path.parent, os.O_RDONLY)
                        try:
                            os.fsync(fd)
                        finally:
                            os.close(fd)
                        if sha(path) != digest:
                            raise RuntimeError('delivery_storage_verification_failed')
                        # Linking updates ctime on the retained inode.
                        canonical['signature'] = signature(source)
                    finally:
                        if os.path.lexists(temporary):
                            os.unlink(temporary)
                reclaimable += size
                changes.append({'task_id': row['task_id'], 'path': str(path.relative_to(root)), 'retained': str(source.relative_to(root)), 'sha256': digest, 'bytes': size})
    return {'schema_version': 1, 'mode': 'apply' if apply else 'preview', 'deduplicated_bytes': reclaimable,
            'preserved_paths': len(changes), 'skipped': skipped, 'changes': changes}


def main():
    os.umask(0o077)
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--apply', action='store_true')
    p.add_argument('--report', type=Path, required=True)
    args = p.parse_args()
    try:
        if args.report.exists() or args.report.is_symlink() or args.report.absolute().is_relative_to(args.root.absolute() / '.agent-runtime/artifacts'):
            raise RuntimeError('delivery_storage_report_destination_invalid')
        result = deduplicate(args.root, apply=args.apply)
        write_json(args.report, result)
        print(json.dumps({k: v for k, v in result.items() if k not in ('changes', 'skipped')}))
        return 0
    except Exception as error:
        code = str(error) if isinstance(error, RuntimeError) else 'delivery_storage_operation_failed'
        print(json.dumps({'status': 'error', 'error_code': code}))
        return 1


if __name__ == '__main__':
    sys.exit(main())
