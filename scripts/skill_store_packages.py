#!/usr/bin/env python3
"""List registry packages and runners installed on demand by Skill Store."""

from __future__ import annotations

import argparse
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


OPTIONAL_SKILLS_ROOT = Path("optional_skills")


@dataclass(frozen=True)
class OnDemandSkillSpec:
    skill_name: str
    package: str
    runner: str
    source_dir: Path


def on_demand_specs(registry_path: Path) -> list[OnDemandSkillSpec]:
    registry = tomllib.loads(registry_path.read_text(encoding="utf-8"))
    specs: set[OnDemandSkillSpec] = set()
    for skill in registry.get("skills", []):
        if skill.get("install_mode") != "on_demand":
            continue
        skill_name = str(skill.get("name") or "").strip()
        runner = str(skill.get("runner_name") or skill_name).strip()
        runner = runner.replace("_", "-")
        if runner and not runner.endswith("-skill"):
            runner += "-skill"
        package = str(skill.get("install_package") or runner).strip()
        if not skill_name or not package or not runner:
            raise ValueError("on-demand skill must declare a name, package, and runner")
        specs.add(
            OnDemandSkillSpec(
                skill_name=skill_name,
                package=package,
                runner=runner,
                source_dir=OPTIONAL_SKILLS_ROOT / skill_name,
            )
        )
    return sorted(specs, key=lambda item: (item.package, item.runner, item.skill_name))


def on_demand_pairs(registry_path: Path) -> list[tuple[str, str]]:
    return [(spec.package, spec.runner) for spec in on_demand_specs(registry_path)]


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
