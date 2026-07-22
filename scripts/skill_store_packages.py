#!/usr/bin/env python3
"""List registry packages and runners installed on demand by Skill Store."""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path


def on_demand_pairs(registry_path: Path) -> list[tuple[str, str]]:
    registry = tomllib.loads(registry_path.read_text(encoding="utf-8"))
    pairs: set[tuple[str, str]] = set()
    for skill in registry.get("skills", []):
        if skill.get("install_mode") != "on_demand":
            continue
        runner = str(skill.get("runner_name") or skill.get("name") or "").strip()
        runner = runner.replace("_", "-")
        if runner and not runner.endswith("-skill"):
            runner += "-skill"
        package = str(skill.get("install_package") or runner).strip()
        if not package or not runner:
            raise ValueError("on-demand skill must declare a package and runner")
        pairs.add((package, runner))
    return sorted(pairs)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--registry",
        type=Path,
        default=Path("configs/skills_registry.toml"),
    )
    parser.add_argument(
        "--format",
        choices=("packages", "runners", "pairs", "cargo-excludes"),
        default="packages",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        pairs = on_demand_pairs(args.registry)
    except (OSError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"skill store package discovery failed: {error}", file=sys.stderr)
        return 1

    for package, runner in pairs:
        if args.format == "packages":
            print(package)
        elif args.format == "runners":
            print(runner)
        elif args.format == "pairs":
            print(f"{package}\t{runner}")
        else:
            print(f"--exclude={package}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
