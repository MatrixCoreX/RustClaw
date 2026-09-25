#!/usr/bin/env python3
"""Materialize Git-visible source files into an isolated NL workspace."""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_release_python_wheels import add_wheel_hashes, wheel_identity


def git_visible_paths(source: Path) -> list[Path]:
    raw = subprocess.check_output(
        [
            "git",
            "-C",
            str(source),
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ]
    )
    paths: list[Path] = []
    for item in raw.split(b"\0"):
        if not item:
            continue
        relative = Path(item.decode("utf-8"))
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"unsafe Git-visible path: {relative}")
        paths.append(relative)
    return paths


def explicit_root_paths(source: Path, roots: list[Path]) -> list[Path]:
    paths: list[Path] = []
    for root in roots:
        if root.is_absolute() or ".." in root.parts:
            raise ValueError(f"unsafe explicit source root: {root}")
        source_root = source / root
        if source_root.is_symlink():
            raise ValueError(f"explicit source root must not be a symlink: {root}")
        if source_root.is_file():
            paths.append(root)
            continue
        if not source_root.is_dir():
            raise ValueError(f"explicit source root does not exist: {root}")
        for source_path in source_root.rglob("*"):
            relative = source_path.relative_to(source)
            if source_path.is_symlink():
                raise ValueError(
                    f"explicit NL source must not contain symlinks: {relative}"
                )
            if source_path.is_file():
                paths.append(relative)
    return paths


def materialize(
    source: Path, destination: Path, include_roots: list[Path] | None = None
) -> tuple[int, int]:
    source = source.resolve(strict=True)
    destination.mkdir(parents=True, exist_ok=True)
    copied_files = 0
    copied_bytes = 0
    paths = set(git_visible_paths(source))
    paths.update(explicit_root_paths(source, include_roots or []))
    for relative in sorted(paths):
        source_path = source / relative
        if source_path.is_symlink():
            raise ValueError(f"isolated NL source must not contain symlinks: {relative}")
        if not source_path.exists():
            continue
        if not source_path.is_file():
            raise ValueError(f"Git-visible source is not a regular file: {relative}")
        target_path = destination / relative
        target_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_path, target_path)
        copied_files += 1
        copied_bytes += source_path.stat().st_size
    return copied_files, copied_bytes


def stage_python_wheels(
    destination: Path, wheelhouse_root: Path, skill_names: list[str]
) -> tuple[int, int]:
    """Bind publisher-built wheels into isolated source without changing the checkout."""
    copied_files = 0
    copied_bytes = 0
    wheelhouse_root = wheelhouse_root.resolve(strict=True)
    for skill_name in skill_names:
        if not re.fullmatch(r"[a-z0-9_]+", skill_name):
            raise ValueError(f"unsafe wheel skill name: {skill_name}")
        source = wheelhouse_root / skill_name
        if source.is_symlink() or not source.is_dir():
            raise ValueError(f"isolated wheelhouse is missing: {source}")
        package = destination / "optional_skills" / skill_name
        lockfile = package / "requirements.lock"
        if not lockfile.is_file():
            raise ValueError(f"isolated Python lockfile is missing: {lockfile}")
        staged = package / "release-wheels"
        staged.mkdir(parents=True, exist_ok=True)
        wheels = sorted(source.glob("*.whl"))
        if not wheels:
            raise ValueError(f"isolated wheelhouse is empty: {source}")
        for wheel in wheels:
            if wheel.is_symlink() or not wheel.is_file():
                raise ValueError(f"isolated wheel must be a regular file: {wheel}")
            wheel_identity(wheel)
            target = staged / wheel.name
            try:
                os.link(wheel, target)
            except OSError:
                shutil.copy2(wheel, target)
            copied_files += 1
            copied_bytes += wheel.stat().st_size
        add_wheel_hashes(lockfile, staged)
    return copied_files, copied_bytes


def stage_precompiled_packages(
    destination: Path, precompiled_root: Path, skill_names: list[str]
) -> tuple[int, int]:
    """Project only requested immutable Cargo packages into the isolated runtime."""
    copied_files = 0
    copied_bytes = 0
    precompiled_root = precompiled_root.resolve(strict=True)
    for skill_name in skill_names:
        if not re.fullmatch(r"[a-z0-9_]+", skill_name):
            raise ValueError(f"unsafe precompiled skill name: {skill_name}")
        source = precompiled_root / skill_name
        if source.is_symlink() or not source.is_dir():
            raise ValueError(f"precompiled skill package is missing: {source}")
        destination_root = destination / "prebuilt" / "skill-packages" / skill_name
        for source_file in sorted(source.rglob("*")):
            if source_file.is_symlink():
                raise ValueError(f"precompiled package contains a symlink: {source_file}")
            if not source_file.is_file():
                continue
            relative = source_file.relative_to(source)
            target = destination_root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            try:
                os.link(source_file, target)
            except OSError:
                shutil.copy2(source_file, target)
            copied_files += 1
            copied_bytes += source_file.stat().st_size
    return copied_files, copied_bytes


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="isolated-workspace-self-test-") as root:
        source = Path(root) / "source"
        destination = Path(root) / "destination"
        source.mkdir()
        subprocess.run(["git", "init", "-q", str(source)], check=True)
        (source / ".gitignore").write_text("ignored.txt\ndocs/\n", encoding="utf-8")
        (source / "tracked.txt").write_text("tracked\n", encoding="utf-8")
        (source / "deleted.txt").write_text("deleted\n", encoding="utf-8")
        (source / "untracked.txt").write_text("untracked\n", encoding="utf-8")
        (source / "ignored.txt").write_text("ignored\n", encoding="utf-8")
        (source / "docs").mkdir()
        (source / "docs" / "guide.md").write_text("guide\n", encoding="utf-8")
        subprocess.run(
            [
                "git",
                "-C",
                str(source),
                "add",
                ".gitignore",
                "tracked.txt",
                "deleted.txt",
            ],
            check=True,
        )
        (source / "deleted.txt").unlink()
        count, byte_count = materialize(
            source, destination, include_roots=[Path("docs")]
        )
        assert count == 4
        assert byte_count > 0
        assert (destination / "tracked.txt").read_text(encoding="utf-8") == "tracked\n"
        assert (destination / "untracked.txt").read_text(encoding="utf-8") == "untracked\n"
        assert (destination / "docs" / "guide.md").read_text(encoding="utf-8") == "guide\n"
        assert not (destination / "ignored.txt").exists()
        assert not (destination / "deleted.txt").exists()
        package = destination / "optional_skills" / "fixture"
        package.mkdir(parents=True)
        lockfile = package / "requirements.lock"
        lockfile.write_text(
            "fixture-pkg==1.0 --hash=sha256:source\n", encoding="utf-8"
        )
        wheels = Path(root) / "wheels" / "fixture"
        wheels.mkdir(parents=True)
        wheel = wheels / "fixture_pkg-1.0-py3-none-any.whl"
        import zipfile

        with zipfile.ZipFile(wheel, "w") as archive:
            archive.writestr(
                "fixture_pkg-1.0.dist-info/METADATA",
                "Name: fixture-pkg\nVersion: 1.0\n",
            )
            archive.writestr(
                "fixture_pkg-1.0.dist-info/WHEEL", "Root-Is-Purelib: true\n"
            )
        wheel_count, wheel_bytes = stage_python_wheels(
            destination, Path(root) / "wheels", ["fixture"]
        )
        assert wheel_count == 1
        assert wheel_bytes == wheel.stat().st_size
        assert "sha256:source" in lockfile.read_text(encoding="utf-8")
        assert (package / "release-wheels" / wheel.name).is_file()
        precompiled = Path(root) / "precompiled" / "fixture"
        (precompiled / "versions" / "v1").mkdir(parents=True)
        (precompiled / "current.json").write_text("{}\n", encoding="utf-8")
        (precompiled / "versions" / "v1" / "install-receipt.json").write_text(
            "{}\n", encoding="utf-8"
        )
        precompiled_count, _ = stage_precompiled_packages(
            destination, Path(root) / "precompiled", ["fixture"]
        )
        assert precompiled_count == 2
        assert (
            destination
            / "prebuilt"
            / "skill-packages"
            / "fixture"
            / "current.json"
        ).is_file()
    print("ISOLATED_WORKSPACE_MATERIALIZER_SELF_TEST_OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--include-root", action="append", type=Path, default=[])
    parser.add_argument("--python-wheelhouse-root", type=Path)
    parser.add_argument("--wheel-skill", action="append", default=[])
    parser.add_argument("--precompiled-root", type=Path)
    parser.add_argument("--precompiled-skill", action="append", default=[])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.source is None or args.destination is None:
        parser.error("--source and --destination are required")
    count, byte_count = materialize(
        args.source, args.destination, include_roots=args.include_root
    )
    wheel_count = 0
    wheel_bytes = 0
    if args.wheel_skill:
        if args.python_wheelhouse_root is None:
            parser.error("--wheel-skill requires --python-wheelhouse-root")
        wheel_count, wheel_bytes = stage_python_wheels(
            args.destination, args.python_wheelhouse_root, args.wheel_skill
        )
    precompiled_count = 0
    precompiled_bytes = 0
    if args.precompiled_skill:
        if args.precompiled_root is None:
            parser.error("--precompiled-skill requires --precompiled-root")
        precompiled_count, precompiled_bytes = stage_precompiled_packages(
            args.destination, args.precompiled_root, args.precompiled_skill
        )
    print(
        "ISOLATED_WORKSPACE_MATERIALIZED "
        f"files={count} bytes={byte_count} "
        f"release_wheels={wheel_count} release_wheel_bytes={wheel_bytes} "
        f"precompiled_files={precompiled_count} "
        f"precompiled_bytes={precompiled_bytes}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
