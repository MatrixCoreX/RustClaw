#!/usr/bin/env python3
"""Check copy space, not future task-output capacity, before starting an isolated suite."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess


def check_space(requirements):
    devices = {}
    for path, size in requirements:
        device = path.stat().st_dev
        entry = devices.setdefault(device, {"path": path, "required": 0})
        entry["required"] += size
    for entry in devices.values():
        if shutil.disk_usage(entry["path"]).free < entry["required"]:
            raise ValueError("isolated_suite_copy_space_unavailable")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--state-root", type=Path, required=True)
    parser.add_argument("--snapshot-root", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    names = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard",
        "configs", "prompts", "crates/skills", "optional_skills", "README.md", "Cargo.toml", "Cargo.lock"],
        cwd=root, check=True, capture_output=True).stdout.split(b"\0")
    block = os.statvfs(args.state_root).f_frsize
    sizes = [(root / os.fsdecode(name)).lstat().st_size for name in set(names) if name]
    source_bytes = sum(((size + block - 1) // block + 1) * block for size in sizes)
    binary_bytes = args.binary.stat().st_size
    check_space([(args.state_root, source_bytes), (args.snapshot_root, binary_bytes)])
    print(json.dumps({"copy_space_preflight": "passed", "source_estimated_bytes": source_bytes,
        "binary_bytes": binary_bytes, "future_output_capacity_guaranteed": False}))


if __name__ == "__main__":
    main()
