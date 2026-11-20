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


def capacity_report(root, destination, reserve):
    report = {'schema_version': 1, 'sampled_at_ts': int(time.time()), 'status': 'unavailable',
              'estimated_snapshot_bytes': None, 'reserve_bytes': reserve,
              'required_available_bytes': None, 'destination_available_bytes': None,
              'destination_shortfall_bytes': None, 'source_available_bytes': None,
              'source_shortfall_bytes': None}
    try:
        estimate = estimate_snapshot_bytes(destination)
        free = shutil.disk_usage(probe_parent(destination)).free
        source = shutil.disk_usage(root).free
        report.update(estimated_snapshot_bytes=estimate, required_available_bytes=reserve + estimate,
                      destination_available_bytes=free, destination_shortfall_bytes=max(0, reserve + estimate - free),
                      source_available_bytes=source, source_shortfall_bytes=max(0, reserve - source))
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
