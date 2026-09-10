#!/usr/bin/env python3
"""Stage tracked sources for on-demand non-Cargo adapters without installing them."""

from __future__ import annotations

import argparse
import shutil
import subprocess
import tomllib
from pathlib import Path

from skill_store_packages import arch_for_target, platform_for_target, runner_specs, supports_platform


ROOT = Path(__file__).resolve().parents[1]


def copy_tracked_source(root: Path, source: Path, destination: Path, lockfile: str | None) -> int:
    relative = source.relative_to(root)
    if relative == Path(".") or not source.is_dir() or source.is_symlink():
        raise ValueError("release_skill_source_invalid")
    output = subprocess.check_output(
        ["git", "-C", str(root), "ls-files", "-z", "--", relative.as_posix()]
    )
    paths = [Path(value.decode("utf-8")) for value in output.split(b"\0") if value]
    if not paths:
        raise ValueError("release_skill_source_untracked")
    required = {relative / "skill.toml"}
    if lockfile:
        required.add(relative / lockfile)
    if not required.issubset(paths):
        raise ValueError("release_skill_source_contract_missing")
    for path in paths:
        original = root / path
        if not original.is_file() or original.is_symlink() or not original.resolve().is_relative_to(source):
            raise ValueError("release_skill_source_file_invalid")
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(original, target)
    return len(paths)


def stage_sources(root: Path, destination: Path, target: str) -> int:
    count = 0
    for spec in runner_specs(root / "configs/skills_registry.toml"):
        if spec.install_mode != "on_demand" or spec.adapter == "cargo":
            continue
        if not supports_platform(spec, platform_for_target(target), arch_for_target(target)):
            continue
        manifest = tomllib.loads(spec.manifest_path.read_text(encoding="utf-8"))
        source = (root / manifest["build"]["source_root"]).resolve(strict=True)
        if source != spec.manifest_path.parent.resolve():
            raise ValueError("release_skill_source_must_be_package_local")
        files = copy_tracked_source(root, source, destination, manifest["build"].get("lockfile"))
        print(f"RELEASE_SKILL_SOURCE skill={spec.skill_name} adapter={spec.adapter} files={files}")
        count += 1
    return count


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    count = stage_sources(ROOT, args.destination.resolve(), args.target)
    print(f"RELEASE_SKILL_SOURCES_CHECK ok packages={count} installed=0 enabled=0")


if __name__ == "__main__":
    main()
