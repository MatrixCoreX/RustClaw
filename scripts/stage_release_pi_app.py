#!/usr/bin/env python3
"""Stage the small-screen app with chip libraries for one release target."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import tempfile

from verify_release_binary import TARGETS, verify_binary


def _library_target(path: Path) -> str:
    for target in TARGETS:
        try:
            verify_binary(path, target)
        except ValueError:
            continue
        return target
    raise ValueError("release_chip_library_header_invalid")


def _stage_libraries(app: Path, target: str) -> dict[str, int | str]:
    vendor = app / "vendor"
    root = vendor / "cryptoauthlib"
    if vendor.is_symlink() or root.is_symlink():
        raise ValueError("release_chip_library_root_invalid")
    kept: list[Path] = []
    excluded: list[Path] = []
    if root.exists():
        if not root.is_dir():
            raise ValueError("release_chip_library_root_invalid")
        for path in sorted(root.rglob("*")):
            if path.is_symlink() and path.is_dir():
                raise ValueError("release_chip_library_directory_alias_invalid")
            if not (
                path.name == "libcryptoauth.so"
                or path.name.startswith("libcryptoauth.so.")
                or path.name == "libcryptoauth.dylib"
                or (path.name.startswith("libcryptoauth.") and path.name.endswith(".dylib"))
            ):
                continue
            if not path.is_file() or not path.resolve().is_relative_to(root.resolve()):
                raise ValueError("release_chip_library_path_invalid")
            (kept if _library_target(path) == target else excluded).append(path)

    # Validate every alias before removing targets; release archives contain files,
    # not native-library symlinks that the deployment extractor would reject.
    for path in kept:
        if path.is_symlink():
            resolved = path.resolve()
            path.unlink()
            shutil.copy2(resolved, path)
    for path in excluded:
        path.unlink()
    return {"target": target, "kept": len(kept), "excluded": len(excluded)}


def stage_app(source: Path, destination: Path, target: str) -> dict[str, int | str]:
    if target not in TARGETS:
        raise ValueError("unsupported_release_target")
    if source.is_symlink() or not source.is_dir():
        raise ValueError("release_pi_app_source_invalid")
    source = source.resolve()
    if (
        destination.exists()
        or destination.is_symlink()
        or destination.resolve().is_relative_to(source)
    ):
        raise ValueError("release_pi_app_destination_invalid")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".release-pi-app-", dir=destination.parent) as temporary:
        staged = Path(temporary) / "app"
        shutil.copytree(source, staged, symlinks=True)
        result = _stage_libraries(staged, target)
        staged.rename(destination)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("target", choices=TARGETS)
    args = parser.parse_args()
    try:
        print(json.dumps(stage_app(args.source, args.destination, args.target), sort_keys=True))
    except (OSError, ValueError) as error:
        parser.exit(1, f"release_pi_app_stage_failed:{error}\n")
