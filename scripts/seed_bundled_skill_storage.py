#!/usr/bin/env python3
"""Initialize missing private skill storage from a signed Release seed.

Existing non-empty skill directories are never merged or overwritten. This keeps
runtime data, browser profiles, and user configuration outside Release ownership.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import uuid


SAFE_SKILL_NAME = re.compile(r"^[a-z0-9][a-z0-9_.-]{0,95}$")


def reject_unsafe_tree(root: Path) -> None:
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"seed root must be a real directory: {root}")
    for current, directories, files in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in [*directories, *files]:
            path = current_path / name
            mode = path.lstat().st_mode
            if stat.S_ISLNK(mode) or not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
                raise ValueError(f"unsupported seed entry: {path}")


def private_permissions(root: Path) -> None:
    for current, directories, files in os.walk(root):
        Path(current).chmod(0o700)
        for name in directories:
            (Path(current) / name).chmod(0o700)
        for name in files:
            (Path(current) / name).chmod(0o600)


def seed_skill(source: Path, destination: Path) -> str:
    if destination.exists():
        if destination.is_symlink() or not destination.is_dir():
            raise ValueError(f"unsafe existing skill storage: {destination}")
        if any(destination.iterdir()):
            return "preserved_existing"
        destination.rmdir()
    staging = destination.parent / f".{destination.name}.seed-{uuid.uuid4().hex}"
    try:
        shutil.copytree(source, staging, symlinks=False)
        private_permissions(staging)
        staging.rename(destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return "initialized"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    args = parser.parse_args()

    source = args.source.resolve()
    destination = args.destination.resolve()
    reject_unsafe_tree(source)
    destination.mkdir(parents=True, exist_ok=True)
    destination.chmod(0o700)

    results: dict[str, str] = {}
    for skill_source in sorted(path for path in source.iterdir() if path.is_dir()):
        skill_name = skill_source.name
        if not SAFE_SKILL_NAME.fullmatch(skill_name):
            raise ValueError(f"unsafe skill seed name: {skill_name}")
        reject_unsafe_tree(skill_source)
        results[skill_name] = seed_skill(skill_source, destination / skill_name)

    print(json.dumps({"schema_version": 1, "skills": results}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
