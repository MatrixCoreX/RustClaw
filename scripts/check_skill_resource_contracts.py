#!/usr/bin/env python3
"""Inventory skill resource contracts and prevent declaration coverage regressions."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path
from typing import Any


MAX_UNCOVERED_ACTIONS = 0
MAX_SKILLS_WITH_UNCOVERED_ACTIONS = 0


def action_key(capability: dict[str, Any]) -> str:
    action = capability.get("action")
    if isinstance(action, str) and action.strip():
        return action.strip()
    return "<direct>"


def canonical_request(request: Any) -> str | None:
    if not isinstance(request, dict):
        return None
    return json.dumps(request, sort_keys=True, separators=(",", ":"))


def inventory(registry: dict[str, Any]) -> dict[str, Any]:
    skills = registry.get("skills")
    if not isinstance(skills, list):
        raise ValueError("skills_registry_missing_skills")

    unique_actions = 0
    covered_actions = 0
    uncovered: list[str] = []
    inconsistent: list[str] = []
    invalid: list[str] = []

    for skill in skills:
        if not isinstance(skill, dict):
            raise ValueError("skills_registry_skill_invalid")
        skill_name = skill.get("name")
        if not isinstance(skill_name, str) or not skill_name.strip():
            raise ValueError("skills_registry_skill_name_invalid")
        base_request = skill.get("resource_request")
        groups: dict[str, list[Any]] = {}
        for capability in skill.get("planner_capabilities", []):
            if not isinstance(capability, dict):
                raise ValueError(f"planner_capability_invalid:{skill_name}")
            groups.setdefault(action_key(capability), []).append(
                capability.get("resource_request", base_request)
            )

        for action, requests in sorted(groups.items()):
            unique_actions += 1
            identifier = f"{skill_name}:{action}"
            canonical = {canonical_request(request) for request in requests}
            if len(canonical) > 1:
                inconsistent.append(identifier)
            effective = requests[0] if requests else base_request
            if not isinstance(effective, dict):
                uncovered.append(identifier)
                continue
            covered_actions += 1
            if not isinstance(effective.get("class"), str):
                invalid.append(f"{identifier}:class")
            if not isinstance(effective.get("cpu_cores"), int) or effective["cpu_cores"] <= 0:
                invalid.append(f"{identifier}:cpu_cores")
            if not isinstance(effective.get("memory_mb"), int) or effective["memory_mb"] <= 0:
                invalid.append(f"{identifier}:memory_mb")

    uncovered_skills = sorted({item.split(":", 1)[0] for item in uncovered})
    return {
        "schema_version": 1,
        "skills": len(skills),
        "unique_actions": unique_actions,
        "covered_actions": covered_actions,
        "uncovered_actions": len(uncovered),
        "skills_with_uncovered_actions": len(uncovered_skills),
        "uncovered_skills": uncovered_skills,
        "inconsistent_actions": inconsistent,
        "invalid_contract_fields": invalid,
    }


def self_test() -> None:
    fixture = {
        "skills": [
            {
                "name": "covered",
                "resource_request": {
                    "class": "general",
                    "cpu_cores": 1,
                    "memory_mb": 128,
                },
                "planner_capabilities": [
                    {"name": "covered.read", "action": "read"},
                    {"name": "covered.read_alias", "action": "read"},
                ],
            },
            {
                "name": "mixed",
                "planner_capabilities": [
                    {"name": "mixed.read", "action": "read"},
                    {
                        "name": "mixed.write",
                        "action": "write",
                        "resource_request": {
                            "class": "disk_io",
                            "cpu_cores": 1,
                            "memory_mb": 64,
                        },
                    },
                ],
            },
        ]
    }
    report = inventory(fixture)
    assert report["unique_actions"] == 3
    assert report["covered_actions"] == 2
    assert report["uncovered_actions"] == 1
    assert report["skills_with_uncovered_actions"] == 1
    assert report["inconsistent_actions"] == []
    assert report["invalid_contract_fields"] == []


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--registry",
        default="configs/skills_registry.toml",
        help="Path to the base skills registry.",
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        print("SKILL_RESOURCE_CONTRACT_SELF_TEST status=ok")
        return 0

    with Path(args.registry).open("rb") as handle:
        report = inventory(tomllib.load(handle))
    print("SKILL_RESOURCE_CONTRACT_INVENTORY " + json.dumps(report, sort_keys=True))

    errors: list[str] = []
    if report["uncovered_actions"] > MAX_UNCOVERED_ACTIONS:
        errors.append(
            f"uncovered_actions_grew:{report['uncovered_actions']}>{MAX_UNCOVERED_ACTIONS}"
        )
    if report["skills_with_uncovered_actions"] > MAX_SKILLS_WITH_UNCOVERED_ACTIONS:
        errors.append(
            "skills_with_uncovered_actions_grew:"
            f"{report['skills_with_uncovered_actions']}>{MAX_SKILLS_WITH_UNCOVERED_ACTIONS}"
        )
    errors.extend(f"inconsistent_action:{item}" for item in report["inconsistent_actions"])
    errors.extend(f"invalid_contract:{item}" for item in report["invalid_contract_fields"])
    if errors:
        for error in errors:
            print(f"SKILL_RESOURCE_CONTRACT_ERROR {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
