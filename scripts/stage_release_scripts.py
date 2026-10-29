#!/usr/bin/env python3
"""Stage distributable scripts without private operator execution evidence."""
from __future__ import annotations

import argparse
from pathlib import Path
import shutil


def stage_scripts(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_dir():
        raise ValueError("release_scripts_source_invalid")
    source = source.resolve()
    if destination.exists() or destination.is_symlink() or destination.resolve().is_relative_to(source):
        raise ValueError("release_scripts_destination_invalid")

    def exclude(directory: str, names: list[str]) -> list[str]:
        return ["nl_suite_logs"] if Path(directory) == source and "nl_suite_logs" in names else []

    shutil.copytree(source, destination, symlinks=True, ignore=exclude)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    try:
        stage_scripts(args.source, args.destination)
    except (OSError, ValueError) as error:
        parser.exit(1, f"release_scripts_stage_failed:{type(error).__name__}\n")
