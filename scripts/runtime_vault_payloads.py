"""Bounded, read-only enumeration of explicitly selected backup payload scopes."""
import json
import os
from pathlib import Path

SCOPES = frozenset(('packages', 'models', 'attachments'))
MAX_ENTRIES = 160000


def selected_roots(root, scopes, package_pointers=None):
    if not set(scopes).issubset(SCOPES): raise RuntimeError('vault_scope_invalid')
    result = []
    if 'models' in scopes: result.append(root / 'data/models')
    if 'attachments' in scopes: result.append(root / '.agent-runtime/artifacts')
    if 'packages' in scopes:
        result.extend([root / 'data/runtime-dependencies', root / 'prebuilt/toolchains'])
        pointers=package_pointers if package_pointers is not None else {str(path.relative_to(root)):None for path in (root/'data/skill-packages').glob('*/current.json')}
        for relative in sorted(pointers):
            pointer=root/relative
            if pointer.is_symlink():raise RuntimeError('vault_package_pointer_symlink')
            value=pointers[relative] if pointers[relative] is not None else json.loads(pointer.read_text())
            # Read the host pointer's actual schema; never guess an installation
            # from a version string, extension or current working directory.
            directory = value.get('install_dir')
            if value.get('schema_version') != 1 or not isinstance(directory, str) or Path(directory).name != directory or directory in ('', '.', '..'):
                raise RuntimeError('vault_package_pointer_invalid')
            path = pointer.parent / 'versions' / directory
            path = path.resolve()
            if not path.is_relative_to(pointer.parent.resolve()):raise RuntimeError('vault_package_pointer_invalid')
            if not path.is_dir():raise RuntimeError('vault_pinned_package_missing')
            result.append(path)
    return result


def enumerate_payloads(root, scopes, package_pointers=None):
    root = Path(root).resolve()
    seen_paths = set(); inodes = {}; count = 0
    for base in selected_roots(root, scopes,package_pointers):
        if not base.exists(): continue
        if base.is_symlink() or not base.resolve().is_relative_to(root): raise RuntimeError('vault_payload_root_invalid')
        base_name=str(base.relative_to(root))
        if base_name not in seen_paths:
            seen_paths.add(base_name);yield {'path':base_name,'kind':'directory','mode':base.stat().st_mode&0o777}
        for directory, dirs, files in os.walk(base, followlinks=False):
            for name in sorted([*dirs, *files]):
                path = Path(directory) / name
                relative = str(path.relative_to(root))
                if relative in seen_paths: continue
                seen_paths.add(relative); count += 1
                if count > MAX_ENTRIES: raise RuntimeError('vault_payload_entry_limit')
                meta = path.lstat()
                if path.is_symlink():
                    resolved = path.resolve()
                    if resolved.is_relative_to(root):
                        original=os.readlink(path)
                        yield {'path':relative,'kind':'symlink','target':original if not Path(original).is_absolute() else os.path.relpath(resolved,path.parent)}
                    else:
                        yield {'path':relative,'kind':'external_dependency','target':os.readlink(path)}
                elif path.is_file():
                    identity = (meta.st_dev, meta.st_ino)
                    if identity in inodes:
                        yield {'path':relative,'kind':'hardlink','target':inodes[identity]}
                    else:
                        inodes[identity] = relative
                        yield {'path':relative,'kind':'file','raw_bytes':meta.st_size,'mode':meta.st_mode & 0o777,'signature':(meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns)}
                elif path.is_dir():yield {'path':relative,'kind':'directory','mode':meta.st_mode&0o777}
                else:raise RuntimeError('vault_payload_special_file')
