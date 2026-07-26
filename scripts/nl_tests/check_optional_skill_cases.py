#!/usr/bin/env python3
"""Validate one independent offline NL case file per on-demand skill."""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REGISTRY = ROOT / "configs" / "skills_registry.toml"
DEFAULT_CASE_DIR = ROOT / "scripts" / "nl_tests" / "cases" / "optional_skills"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--case-dir", type=Path, default=DEFAULT_CASE_DIR)
    return parser.parse_args()


def on_demand_skills(registry_path: Path) -> list[dict[str, object]]:
    with registry_path.open("rb") as handle:
        registry = tomllib.load(handle)
    return sorted(
        (
            entry
            for entry in registry.get("skills", [])
            if entry.get("install_mode") == "on_demand"
        ),
        key=lambda entry: str(entry.get("name", "")),
    )


def active_rows(path: Path) -> list[tuple[str, str, str, str]]:
    rows: list[tuple[str, str, str, str]] = []
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 3)
        if len(parts) != 4:
            raise ValueError(f"{path}:{line_number}: expected four pipe-delimited fields")
        rows.append(tuple(parts))
    return rows


def validate_case_file(skill: dict[str, object], path: Path) -> tuple[int, list[str]]:
    name = str(skill["name"])
    rows = active_rows(path)
    errors: list[str] = []
    roles: set[str] = set()
    for suite, case_name, tags, prompt in rows:
        tag_set = {tag.strip() for tag in tags.split(",") if tag.strip()}
        if suite != "optional_skill_lifecycle":
            errors.append(f"{case_name}: suite must be optional_skill_lifecycle")
        for required in (
            f"optional_skill:{name}",
            "provider_mode:offline",
            "external_calls:0",
            "independent",
        ):
            if required not in tag_set:
                errors.append(f"{case_name}: missing tag {required}")
        for role in ("case_role:happy", "case_role:failure"):
            if role in tag_set:
                roles.add(role)
        if not prompt.strip():
            errors.append(f"{case_name}: prompt is empty")
        if name == "x" and "side_effect_mode:dry_run" not in tag_set:
            errors.append(f"{case_name}: X cases must declare side_effect_mode:dry_run")
    for role in ("case_role:happy", "case_role:failure"):
        if role not in roles:
            errors.append(f"missing {role} row")
    return len(rows), errors


def main() -> int:
    args = parse_args()
    skills = on_demand_skills(args.registry)
    expected_names = {str(skill["name"]) for skill in skills}
    actual_names = {path.stem for path in args.case_dir.glob("*.txt")}
    errors: list[str] = []
    if expected_names != actual_names:
        errors.append(
            "case inventory mismatch "
            f"missing={sorted(expected_names - actual_names)} "
            f"unexpected={sorted(actual_names - expected_names)}"
        )

    print("skill\tpackage\trunner\tcase_file\trows\tprovider_mode")
    for skill in skills:
        name = str(skill["name"])
        path = args.case_dir / f"{name}.txt"
        row_count = 0
        file_errors: list[str] = []
        if not path.is_file():
            file_errors.append("case file missing")
        else:
            try:
                row_count, file_errors = validate_case_file(skill, path)
            except (OSError, UnicodeError, ValueError) as error:
                file_errors.append(str(error))
        errors.extend(f"{name}: {error}" for error in file_errors)
        print(
            "\t".join(
                (
                    name,
                    str(skill.get("install_package", "")),
                    str(skill.get("runner_name", name.replace("_", "-")) + "-skill")
                    if not skill.get("runner_name")
                    else str(skill["runner_name"]),
                    path.relative_to(ROOT).as_posix(),
                    str(row_count),
                    "offline",
                )
            )
        )

    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print(f"OK: {len(skills)} on-demand skills have independent offline NL cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
