#!/usr/bin/env python3
"""Remove known Cargo cache files, preserving runtime files and business data.

Dry run by default. --apply deletes only selected cache files. Select relocated
targets explicitly with --target-dir; directory symlinks are not followed.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from dataclasses import dataclass
import os
from pathlib import Path
import stat
import subprocess
import sys

if os.name == "posix":
    import fcntl
else:
    fcntl = None

ROOT = Path(__file__).resolve().parents[1]
CARGO_TAG = b"Signature: 8a477f597d28d172789f06886806bc55"
CACHE_DIRECTORIES = ("deps", "build", ".fingerprint", "incremental")
CACHE_SUFFIXES = {".d", ".o", ".a", ".rlib", ".rmeta"}
CACHE_RECORDS = {"dep-graph.bin", "query-cache.bin", "work-products.bin", "thin-lto-past-keys.bin"}


@dataclass(frozen=True)
class CacheFile:
    directory_fd: int
    relative: Path
    identity: tuple[int, ...]
    size: int
    label: str


def identity(metadata):
    return (metadata.st_dev, metadata.st_ino, metadata.st_size,
            metadata.st_mtime_ns, metadata.st_mode, metadata.st_uid)


def directory_at(stack, name, parent=None):
    fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
    stack.callback(os.close, fd)
    if os.fstat(fd).st_uid != os.geteuid():
        raise RuntimeError("cleanup_directory_not_owned")
    return fd


def regular_at(stack, name, parent):
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    stack.callback(os.close, fd)
    metadata = os.fstat(fd)
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid():
        raise RuntimeError("cleanup_marker_not_owned_regular_file")
    return fd


def idle_builds():
    commands = subprocess.check_output(["ps", "-A", "-o", "comm="], text=True,
                                       stderr=subprocess.PIPE)
    busy = {Path(line.strip()).name for line in commands.splitlines()} & {
        "cargo", "rustc", "rust-lld", "cc", "clang", "gcc", "ld", "lld", "ld.lld",
    }
    if busy:
        raise RuntimeError("cleanup_build_active")


def profile_fds(stack, target):
    target = target.absolute()
    before = target.lstat()
    if not stat.S_ISDIR(before.st_mode):
        raise RuntimeError("cleanup_target_not_directory")
    canonical = target.resolve(strict=True)
    parent = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    stack.callback(os.close, parent)
    for part in canonical.parts[1:]:
        parent = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
        stack.callback(os.close, parent)
    actual = os.fstat(parent)
    if (actual.st_dev, actual.st_ino) != (before.st_dev, before.st_ino) or actual.st_uid != os.geteuid():
        raise RuntimeError("cleanup_target_changed_or_not_owned")
    tag = regular_at(stack, "CACHEDIR.TAG", parent)
    if os.read(tag, len(CARGO_TAG) + 1).splitlines()[:1] != [CARGO_TAG]:
        raise RuntimeError("cleanup_cargo_tag_invalid")
    bases = [(parent, "")]
    for entry in os.listdir(parent):
        if entry in {"debug", "release"}:
            continue
        metadata = os.stat(entry, dir_fd=parent, follow_symlinks=False)
        if stat.S_ISDIR(metadata.st_mode):
            bases.append((directory_at(stack, entry, parent), entry + "/"))
    for base, prefix in bases:
        for profile in ("debug", "release"):
            try:
                metadata = os.stat(profile, dir_fd=base, follow_symlinks=False)
            except FileNotFoundError:
                continue
            if not stat.S_ISDIR(metadata.st_mode):
                continue
            fd = directory_at(stack, profile, base)
            try:
                lock = regular_at(stack, ".cargo-lock", fd)
            except FileNotFoundError:
                continue
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield fd, prefix + profile


def plan_cache_files(stack, targets):
    selected = []
    seen = set()
    seen_targets = set()
    for target in targets:
        if not target.exists() and not target.is_symlink():
            continue
        entry = target.lstat()
        target_key = (entry.st_dev, entry.st_ino)
        if target_key in seen_targets:
            continue
        seen_targets.add(target_key)
        for profile, label in profile_fds(stack, target):
            for name in CACHE_DIRECTORIES:
                try:
                    metadata = os.stat(name, dir_fd=profile, follow_symlinks=False)
                except FileNotFoundError:
                    continue
                if not stat.S_ISDIR(metadata.st_mode):
                    continue
                cache = directory_at(stack, name, profile)
                key = (metadata.st_dev, metadata.st_ino)
                if key in seen:
                    continue
                seen.add(key)
                for relative, _, files, directory in os.fwalk(".", dir_fd=cache,
                                                           follow_symlinks=False):
                    for filename in files:
                        metadata = os.stat(filename, dir_fd=directory, follow_symlinks=False)
                        if not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & 0o111:
                            continue
                        if metadata.st_uid != os.geteuid():
                            continue
                        if Path(filename).suffix not in CACHE_SUFFIXES and filename not in CACHE_RECORDS:
                            continue
                        path = Path(relative) / filename
                        selected.append(CacheFile(cache, path, identity(metadata), metadata.st_size,
                                                  f"{label}/{name}/{path.as_posix()}"))
    return selected


def unlink_cache_file(candidate):
    with ExitStack() as stack:
        parent = candidate.directory_fd
        for part in candidate.relative.parts[:-1]:
            parent = directory_at(stack, part, parent)
        name = candidate.relative.name
        metadata = os.stat(name, dir_fd=parent, follow_symlinks=False)
        if identity(metadata) != candidate.identity:
            raise RuntimeError("cleanup_entry_changed")
        os.unlink(name, dir_fd=parent)


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--apply", action="store_true", help="delete reviewed Cargo cache files")
    parser.add_argument("--target-dir", type=Path, action="append",
                        help="explicit Cargo target root containing CACHEDIR.TAG")
    return parser.parse_args(argv)


def main(argv):
    args = parse_args(argv)
    try:
        if fcntl is None or not hasattr(os, "fwalk") or not hasattr(os, "O_NOFOLLOW"):
            raise RuntimeError("cleanup_platform_unsupported")
        idle_builds()
        with ExitStack() as stack:
            files = plan_cache_files(stack, args.target_dir or [ROOT / "target"])
            for candidate in files:
                print(f"{candidate.size:>12}  {candidate.label}")
            if args.apply:
                for candidate in files:
                    unlink_cache_file(candidate)
            print(f"CARGO_CACHE_CLEAN mode={'APPLY' if args.apply else 'DRY_RUN'} "
                  f"files={len(files)} bytes={sum(item.size for item in files)}")
        return 0
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"CARGO_CACHE_CLEAN refused: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
