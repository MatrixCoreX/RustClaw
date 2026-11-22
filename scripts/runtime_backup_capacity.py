"""Read-only capacity evidence shared by checkpoint admission and the console."""
import json
from pathlib import Path
import shutil
import stat
import time

DEFAULT_ESTIMATE = 900 * 1024**2


def estimate_snapshot_bytes(destination):
    destination = Path(destination)
    candidates = []
    if destination.is_dir():
        for path in destination.glob('checkpoint-*'):
            if path.is_symlink() or not path.is_dir() or not (path / 'manifest.json').is_file():
                continue
            candidates.append(path)
    if not candidates:
        return DEFAULT_ESTIMATE
    recent = sorted(candidates)[-1]
    return sum(p.stat().st_size for p in recent.iterdir() if not p.is_symlink() and stat.S_ISREG(p.stat().st_mode))


def probe_parent(path):
    path = Path(path)
    while not path.exists() and path.parent != path:
        path = path.parent
    return path


def database_growth_bytes(root, destination):
    """Include positive live SQLite growth since the latest measured snapshot."""
    root = Path(root).resolve()
    candidates = sorted(p for p in Path(destination).glob('checkpoint-*')
                        if p.is_dir() and not p.is_symlink()
                        and (p / 'manifest.json').is_file()
                        and not (p / 'manifest.json').is_symlink())
    if not candidates:
        return 0
    manifest = candidates[-1] / 'manifest.json'
    if manifest.stat().st_size > 16 * 1024**2:
        raise ValueError('checkpoint_estimate_manifest_oversized')
    rows = json.loads(manifest.read_text()).get('files', [])
    if not isinstance(rows, list):
        raise ValueError('checkpoint_estimate_manifest_invalid')
    growth = 0
    seen = set()
    for row in rows:
        if not isinstance(row, dict) or row.get('kind') != 'sqlite':
            continue
        relative, previous = row.get('path'), row.get('raw_bytes')
        if not isinstance(relative, str) or not relative or type(previous) is not int or previous < 0:
            raise ValueError('checkpoint_estimate_database_invalid')
        name = Path(relative)
        if name.is_absolute() or '..' in name.parts or '\\' in relative:
            raise ValueError('checkpoint_estimate_path_invalid')
        source = (root / name).resolve()
        if not source.is_relative_to(root):
            raise ValueError('checkpoint_estimate_path_invalid')
        if source in seen:
            raise ValueError('checkpoint_estimate_database_duplicate')
        seen.add(source)
        if not source.exists():
            continue
        if not source.is_file():
            raise ValueError('checkpoint_estimate_database_invalid')
        size = source.stat().st_size
        wal = source.with_name(source.name + '-wal')
        if wal.exists():
            if wal.is_symlink() or not wal.is_file():
                raise ValueError('checkpoint_estimate_wal_invalid')
            size += wal.stat().st_size
        growth += max(0, size - previous)
    return growth


def capacity_report(root, destination, reserve):
    report = {'schema_version': 1, 'sampled_at_ts': int(time.time()), 'status': 'unavailable',
              'estimated_snapshot_bytes': None, 'reserve_bytes': reserve,
              'required_available_bytes': None, 'destination_available_bytes': None,
              'destination_shortfall_bytes': None, 'source_available_bytes': None,
              'source_shortfall_bytes': None}
    try:
        growth = database_growth_bytes(root, destination)
        estimate = estimate_snapshot_bytes(destination) + growth
        destination_usage = shutil.disk_usage(probe_parent(destination))
        free = destination_usage.free
        source = shutil.disk_usage(root).free
        report.update(estimated_snapshot_bytes=estimate, required_available_bytes=reserve + estimate,
                      destination_available_bytes=free, destination_shortfall_bytes=max(0, reserve + estimate - free),
                      source_available_bytes=source, source_shortfall_bytes=max(0, reserve - source))
        warning = max(2 * 1024**3, int(destination_usage.total * .03))
        projected = max(0, free - estimate)
        report.update(estimated_database_growth_bytes=growth,
                      projected_destination_available_bytes=projected,
                      destination_warning_reserve_bytes=warning,
                      projected_destination_pressure='warning' if projected < warning else 'normal')
        report['status'] = 'ready' if free >= reserve + estimate and source >= reserve else 'insufficient'
    except (OSError, ValueError):
        report['error_code'] = 'checkpoint_capacity_unavailable'
    return report


def verified_source_revision(root, destination):
    try:
        status_path = Path(root) / 'data/runtime-checkpoint-status.json'
        if status_path.is_symlink() or status_path.stat().st_size > 65536:
            return None
        status = json.loads(status_path.read_text())
        if status.get('status') not in ('verified', 'verified_remote', 'verified_local_remote_failed'):
            return None
        checkpoint = Path(status['checkpoint'])
        if checkpoint.is_symlink() or checkpoint.parent.resolve() != Path(destination).resolve():
            return None
        path = checkpoint / 'manifest.json'
        if path.is_symlink() or path.stat().st_size > 16 * 1024**2:
            return None
        value = json.loads(path.read_text()).get('source_revision')
        if isinstance(value, str) and len(value) == 40 and all(c in '0123456789abcdef' for c in value):
            return value
    except (OSError, ValueError, KeyError, TypeError):
        pass
    return None
