#!/usr/bin/env python3
"""Inventory registered skill contracts and ratchet known capability debt."""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
import tempfile
import tomllib
from collections import defaultdict
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "configs" / "skills_registry.toml"

# Baseline captured on 2026-07-27. These ceilings only prevent regression;
# completed vertical slices must lower them instead of adding exemptions.
MAX_TEXT_ONLY_OUTPUT_SKILLS = 24
MAX_UNCONSTRAINED_ACTION_SKILLS = 12
MAX_MISSING_PLANNER_ARGUMENTS = 0
MAX_EXPOSED_DUPLICATE_ACTION_GROUPS = 0
MAX_UNMAPPED_PUBLIC_ACTIONS = 36


@dataclasses.dataclass(frozen=True)
class InventoryMetrics:
    skills: int
    core_skills: int
    on_demand_skills: int
    planner_visible_skills: int
    declared_capabilities: int
    alias_capabilities: int
    exposed_capabilities: int
    eager_exposed_capabilities: int
    text_only_output_skills: int
    unconstrained_action_skills: int
    missing_planner_arguments: int
    exposed_duplicate_action_groups: int
    unmapped_public_actions: int


def load_skills(path: Path) -> list[dict[str, Any]]:
    try:
        parsed = tomllib.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise SystemExit(f"failed_to_read_registry path={path} error={exc}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise SystemExit(f"failed_to_parse_registry path={path} error={exc}") from exc
    skills = parsed.get("skills", [])
    if not isinstance(skills, list):
        raise SystemExit(f"invalid_registry_skills path={path}")
    return skills


def planner_argument_tokens(capability: dict[str, Any]) -> set[str]:
    return {
        token.strip()
        for expression in [
            *(capability.get("required") or []),
            *(capability.get("optional") or []),
        ]
        for alternative in str(expression).split("|")
        for token in alternative.split("+")
        if token.strip()
    }


def schema_properties(skill: dict[str, Any]) -> dict[str, Any]:
    schema = skill.get("input_schema") or {}
    properties = schema.get("properties") or {}
    return properties if isinstance(properties, dict) else {}


def action_enum(skill: dict[str, Any]) -> tuple[str, ...]:
    action = schema_properties(skill).get("action") or {}
    values = action.get("enum") if isinstance(action, dict) else None
    if not isinstance(values, list):
        return ()
    return tuple(str(value) for value in values)


def has_unconstrained_action(skill: dict[str, Any]) -> bool:
    action = schema_properties(skill).get("action")
    if not isinstance(action, dict) or action.get("type") != "string":
        return False
    return not any(key in action for key in ("enum", "const", "oneOf", "anyOf"))


def is_text_only_output(skill: dict[str, Any]) -> bool:
    schema = skill.get("output_schema") or {}
    properties = schema.get("properties") or {}
    required = schema.get("required") or []
    return (
        isinstance(properties, dict)
        and set(properties) == {"text"}
        and list(required) == ["text"]
    )


def source_inventory(skill: dict[str, Any], root: Path) -> dict[str, Any]:
    name = str(skill.get("name") or "").strip()
    source_candidates = [
        root / "crates" / "skills" / name,
        root / "optional_skills" / name,
        root / "external_skills" / name,
    ]
    sources = [str(path.relative_to(root)) for path in source_candidates if path.exists()]
    tests: list[str] = []
    for source in source_candidates:
        if not source.exists():
            continue
        tests.extend(
            str(path.relative_to(root))
            for path in source.rglob("*")
            if path.is_file()
            and (
                "test" in path.name.lower()
                or "fixture" in {part.lower() for part in path.parts}
            )
        )
    if skill.get("kind") == "builtin":
        builtin_dir = root / "crates" / "clawd" / "src" / "skills"
        builtin_candidates = [
            builtin_dir / f"builtin_{name}.rs",
            builtin_dir / "builtin.rs",
        ]
        sources.extend(
            str(path.relative_to(root)) for path in builtin_candidates if path.exists()
        )
        tests.extend(
            str(path.relative_to(root))
            for path in builtin_dir.glob("builtin*tests.rs")
            if path.is_file()
            and (name in path.name or path.name == "builtin_tests.rs")
        )
    prompt_file = str(skill.get("prompt_file") or "").strip()
    return {
        "source_paths": sorted(set(sources)),
        "test_files": sorted(set(tests)),
        "prompt_file": prompt_file or None,
    }


def inventory(skills: list[dict[str, Any]], root: Path = ROOT) -> dict[str, Any]:
    rows: list[dict[str, Any]] = []
    missing_planner_arguments: list[dict[str, Any]] = []
    exposed_duplicate_groups: list[dict[str, Any]] = []
    unmapped_public_actions: list[dict[str, Any]] = []
    declared_capabilities = 0
    alias_capabilities = 0
    exposed_capabilities = 0
    eager_exposed_capabilities = 0

    for skill in skills:
        name = str(skill.get("name") or "").strip()
        aliases = {
            str(alias): str(target)
            for alias, target in (skill.get("planner_capability_aliases") or {}).items()
        }
        capabilities = skill.get("planner_capabilities") or []
        properties = schema_properties(skill)
        enum_actions = action_enum(skill)
        exposed = [cap for cap in capabilities if str(cap.get("name")) not in aliases]
        declared_capabilities += len(capabilities)
        alias_capabilities += len(aliases)
        exposed_capabilities += len(exposed)
        if (
            skill.get("enabled", True)
            and skill.get("planner_visible", True)
            and skill.get("planner_eager_load", False)
        ):
            eager_exposed_capabilities += len(exposed)

        for capability in capabilities:
            absent = sorted(planner_argument_tokens(capability) - set(properties))
            if absent:
                missing_planner_arguments.append(
                    {
                        "skill": name,
                        "capability": capability.get("name"),
                        "arguments": absent,
                    }
                )

        by_action: dict[tuple[Any, ...], list[str]] = defaultdict(list)
        for capability in exposed:
            action = str(capability.get("action") or "<default>")
            signature = (
                action,
                capability.get("effect"),
                tuple(capability.get("required") or []),
                tuple(capability.get("optional") or []),
            )
            by_action[signature].append(str(capability.get("name") or ""))
        for signature, names in sorted(by_action.items()):
            if len(names) > 1:
                exposed_duplicate_groups.append(
                    {
                        "skill": name,
                        "action": signature[0],
                        "effect": signature[1],
                        "capabilities": sorted(names),
                    }
                )

        mapped_actions = {
            str(capability.get("action"))
            for capability in exposed
            if capability.get("action") is not None
        }
        if skill.get("planner_visible", True):
            for action in sorted(set(enum_actions) - mapped_actions):
                unmapped_public_actions.append({"skill": name, "action": action})

        row = {
            "name": name,
            "kind": skill.get("kind"),
            "install_mode": skill.get("install_mode") or "core",
            "fixed_on": bool(skill.get("fixed_on", False)),
            "planner_visible": bool(skill.get("planner_visible", True)),
            "planner_eager_load": bool(skill.get("planner_eager_load", False)),
            "requires_confirmation": bool(skill.get("requires_confirmation", False)),
            "confirmation_exempt_when": skill.get("confirmation_exempt_when") or [],
            "supported_os": skill.get("supported_os") or [],
            "runtime_adapter": {
                "runtime_skill": skill.get("runtime_skill"),
                "runtime_action": skill.get("runtime_action"),
                "runtime_default_args": skill.get("runtime_default_args") or {},
                "runtime_rewrite_arg_keys": skill.get("runtime_rewrite_arg_keys") or [],
                "package_manifest": skill.get("package_manifest"),
            },
            "evidence_owner": skill.get("evidence_owner"),
            "input_fields": sorted(properties),
            "action_enum": list(enum_actions),
            "action_constrained": not has_unconstrained_action(skill),
            "output_required": list((skill.get("output_schema") or {}).get("required") or []),
            "output_fields": sorted(
                ((skill.get("output_schema") or {}).get("properties") or {}).keys()
            ),
            "text_only_output": is_text_only_output(skill),
            "declared_capabilities": [
                {
                    **capability,
                    "alias_hidden": str(capability.get("name")) in aliases,
                }
                for capability in capabilities
            ],
            "planner_capability_aliases": aliases,
            **source_inventory(skill, root),
        }
        rows.append(row)

    metrics = InventoryMetrics(
        skills=len(skills),
        core_skills=sum(skill.get("install_mode") != "on_demand" for skill in skills),
        on_demand_skills=sum(skill.get("install_mode") == "on_demand" for skill in skills),
        planner_visible_skills=sum(skill.get("planner_visible", True) for skill in skills),
        declared_capabilities=declared_capabilities,
        alias_capabilities=alias_capabilities,
        exposed_capabilities=exposed_capabilities,
        eager_exposed_capabilities=eager_exposed_capabilities,
        text_only_output_skills=sum(is_text_only_output(skill) for skill in skills),
        unconstrained_action_skills=sum(has_unconstrained_action(skill) for skill in skills),
        missing_planner_arguments=len(missing_planner_arguments),
        exposed_duplicate_action_groups=len(exposed_duplicate_groups),
        unmapped_public_actions=len(unmapped_public_actions),
    )
    return {
        "schema_version": 1,
        "metrics": dataclasses.asdict(metrics),
        "debt": {
            "missing_planner_arguments": missing_planner_arguments,
            "exposed_duplicate_action_groups": exposed_duplicate_groups,
            "unmapped_public_actions": unmapped_public_actions,
            "text_only_output_skills": sorted(
                str(skill.get("name")) for skill in skills if is_text_only_output(skill)
            ),
            "unconstrained_action_skills": sorted(
                str(skill.get("name"))
                for skill in skills
                if has_unconstrained_action(skill)
            ),
        },
        "skills": sorted(rows, key=lambda row: row["name"]),
    }


def findings_for(metrics: dict[str, int]) -> list[str]:
    ceilings = {
        "text_only_output_skills": MAX_TEXT_ONLY_OUTPUT_SKILLS,
        "unconstrained_action_skills": MAX_UNCONSTRAINED_ACTION_SKILLS,
        "missing_planner_arguments": MAX_MISSING_PLANNER_ARGUMENTS,
        "exposed_duplicate_action_groups": MAX_EXPOSED_DUPLICATE_ACTION_GROUPS,
        "unmapped_public_actions": MAX_UNMAPPED_PUBLIC_ACTIONS,
    }
    return [
        f"{name}_grew:{metrics[name]}>{ceiling}"
        for name, ceiling in ceilings.items()
        if metrics[name] > ceiling
    ]


def run_self_test() -> int:
    fixture = tomllib.loads(
        """
[[skills]]
name = "demo"
kind = "runner"
planner_eager_load = true
planner_capability_aliases = { "demo.read_old" = "demo.read" }
planner_capabilities = [
  { name = "demo.read", action = "read", required = ["path"] },
  { name = "demo.read_old", action = "read", required = ["path"] },
]
input_schema = { type = "object", properties = { action = { type = "string" }, path = { type = "string" } } }
output_schema = { type = "object", required = ["text"], properties = { text = { type = "string" } } }

[[skills]]
name = "broken"
kind = "runner"
planner_capabilities = [
  { name = "broken.run", action = "run", required = ["missing"] },
  { name = "broken.run_again", action = "run", required = ["missing"] },
]
input_schema = { type = "object", properties = { action = { type = "string", enum = ["run", "inspect"] } } }
output_schema = { type = "object", required = ["status"], properties = { status = { type = "string" } } }
"""
    )["skills"]
    with tempfile.TemporaryDirectory(prefix="builtin-capability-inventory-") as tmp:
        result = inventory(fixture, Path(tmp))
    metrics = result["metrics"]
    assert metrics["skills"] == 2
    assert metrics["declared_capabilities"] == 4
    assert metrics["alias_capabilities"] == 1
    assert metrics["exposed_capabilities"] == 3
    assert metrics["eager_exposed_capabilities"] == 1
    assert metrics["text_only_output_skills"] == 1
    assert metrics["unconstrained_action_skills"] == 1
    assert metrics["missing_planner_arguments"] == 2
    assert metrics["exposed_duplicate_action_groups"] == 1
    assert metrics["unmapped_public_actions"] == 1
    assert result["debt"]["unmapped_public_actions"] == [
        {"skill": "broken", "action": "inspect"}
    ]
    over = {
        "text_only_output_skills": MAX_TEXT_ONLY_OUTPUT_SKILLS + 1,
        "unconstrained_action_skills": MAX_UNCONSTRAINED_ACTION_SKILLS,
        "missing_planner_arguments": MAX_MISSING_PLANNER_ARGUMENTS,
        "exposed_duplicate_action_groups": MAX_EXPOSED_DUPLICATE_ACTION_GROUPS,
        "unmapped_public_actions": MAX_UNMAPPED_PUBLIC_ACTIONS,
    }
    assert findings_for(over) == [
        f"text_only_output_skills_grew:{MAX_TEXT_ONLY_OUTPUT_SKILLS + 1}>{MAX_TEXT_ONLY_OUTPUT_SKILLS}"
    ]
    print("BUILTIN_CAPABILITY_INVENTORY_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    parser.add_argument("--json", action="store_true", help="print full JSON inventory")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    result = inventory(load_skills(args.registry))
    findings = findings_for(result["metrics"])
    if args.json:
        print(json.dumps(result, ensure_ascii=True, indent=2, sort_keys=True))
    else:
        metrics = " ".join(f"{key}={value}" for key, value in result["metrics"].items())
        print(f"BUILTIN_CAPABILITY_INVENTORY findings={len(findings)} {metrics}")
        for finding in findings:
            print(f"  - {finding}")
        for debt_name, rows in result["debt"].items():
            print(f"  debt.{debt_name}={len(rows)}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
