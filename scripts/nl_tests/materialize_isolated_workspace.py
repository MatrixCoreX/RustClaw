#!/usr/bin/env python3
"""Materialize Git-visible source files into an isolated NL workspace."""

from __future__ import annotations

import argparse
import shutil
import subprocess
import tempfile
from pathlib import Path


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
        if not source_path.is_file():
            raise ValueError(f"Git-visible source is not a regular file: {relative}")
        target_path = destination / relative
        target_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_path, target_path)
        copied_files += 1
        copied_bytes += source_path.stat().st_size
    return copied_files, copied_bytes


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="isolated-workspace-self-test-") as root:
        source = Path(root) / "source"
        destination = Path(root) / "destination"
        source.mkdir()
        subprocess.run(["git", "init", "-q", str(source)], check=True)
        (source / ".gitignore").write_text("ignored.txt\ndocs/\n", encoding="utf-8")
        (source / "tracked.txt").write_text("tracked\n", encoding="utf-8")
        (source / "untracked.txt").write_text("untracked\n", encoding="utf-8")
        (source / "ignored.txt").write_text("ignored\n", encoding="utf-8")
        (source / "docs").mkdir()
        (source / "docs" / "guide.md").write_text("guide\n", encoding="utf-8")
        subprocess.run(
            ["git", "-C", str(source), "add", ".gitignore", "tracked.txt"],
            check=True,
        )
        count, byte_count = materialize(
            source, destination, include_roots=[Path("docs")]
        )
        assert count == 4
        assert byte_count > 0
        assert (destination / "tracked.txt").read_text(encoding="utf-8") == "tracked\n"
        assert (destination / "untracked.txt").read_text(encoding="utf-8") == "untracked\n"
        assert (destination / "docs" / "guide.md").read_text(encoding="utf-8") == "guide\n"
        assert not (destination / "ignored.txt").exists()
    print("ISOLATED_WORKSPACE_MATERIALIZER_SELF_TEST_OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--include-root", action="append", type=Path, default=[])
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
    print(
        "ISOLATED_WORKSPACE_MATERIALIZED "
        f"files={count} bytes={byte_count}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
