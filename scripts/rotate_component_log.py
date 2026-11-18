#!/usr/bin/env python3
"""Preserve logs before daemon launch; never follow symlinks or truncate a prior run."""
import argparse
import gzip
import hashlib
import os
from pathlib import Path
import re
import shutil
import stat
import time


def digest(path, opener=open):
    value = hashlib.sha256()
    with opener(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.digest()


def rotate(path, keep=20):
    path = Path(path)
    if not 1 <= keep <= 100:
        raise ValueError("log_retention_out_of_range")
    if path.is_symlink():
        raise ValueError("log_symlink_rejected")
    if path.exists():
        info = path.stat()
        if not stat.S_ISREG(info.st_mode):
            raise ValueError("log_not_regular")
        if info.st_size:
            archive = path.with_name(f"{path.name}.previous.{time.time_ns()}.{os.getpid()}.log")
            # link is exclusive; preserve the old bytes even if compression fails.
            os.link(path, archive, follow_symlinks=False)
            path.unlink()
            compressed = archive.with_suffix(".log.gz")
            try:
                with archive.open("rb") as source, compressed.open("xb") as output:
                    os.chmod(compressed, stat.S_IMODE(info.st_mode) & 0o600)
                    with gzip.GzipFile(fileobj=output, mode="wb", compresslevel=1) as target:
                        shutil.copyfileobj(source, target)
                    output.flush()
                    os.fsync(output.fileno())
                if digest(archive) != digest(compressed, gzip.open):
                    raise ValueError("log_archive_verification_failed")
                archive.unlink()
            except Exception:
                compressed.unlink(missing_ok=True)
                raise
    pattern = re.compile(re.escape(path.name) + r"\.previous\.(\d+)\.\d+\.log(?:\.gz)?$")
    archives = []
    for candidate in path.parent.iterdir():
        match = pattern.fullmatch(candidate.name)
        if match and candidate.is_file() and not candidate.is_symlink():
            archives.append((int(match.group(1)), candidate))
    for _, archive in sorted(archives, reverse=True)[keep:]:
        archive.unlink()
    # Launcher appends, so a concurrent startup cannot truncate preserved bytes.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0), 0o600)
    os.close(fd)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log_path", type=Path)
    parser.add_argument("--keep", type=int, default=20)
    args = parser.parse_args()
    rotate(args.log_path, args.keep)
