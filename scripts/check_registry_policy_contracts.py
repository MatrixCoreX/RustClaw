#!/usr/bin/env python3
"""Validate planner-capability policy metadata in skill registries."""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REGISTRIES = [
    ROOT / "configs" / "skills_registry.toml",
    ROOT / "docker" / "config" / "skills_registry.toml",
]

ALLOWED_EFFECTS = {"observe", "validate", "mutate", "external"}
ALLOWED_RISK_LEVELS = {"low", "medium", "high", "unknown"}
ALLOWED_DEDUP_SCOPES = {"args", "action", "resource"}


def load_registry(path: Path) -> list[dict[str, Any]]:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8")).get("skills", [])
    except OSError as exc:
        raise SystemExit(f"failed_to_read_registry path={path} error={exc}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise SystemExit(f"failed_to_parse_registry path={path} error={exc}") from exc


def capability_ref(skill: dict[str, Any], index: int, capability: dict[str, Any]) -> str:
    skill_name = str(skill.get("name") or "unknown_skill").strip() or "unknown_skill"
    cap_name = str(capability.get("name") or "").strip()
    return cap_name or f"{skill_name}.planner_capabilities[{index}]"


def check_capability(
    registry_path: Path,
    skill: dict[str, Any],
    index: int,
    capability: dict[str, Any],
) -> list[str]:
    ref = capability_ref(skill, index, capability)
    prefix = f"{registry_path.relative_to(ROOT)}:{ref}"
    findings: list[str] = []

    effect = capability.get("effect")
    risk_level = capability.get("risk_level")
    idempotent = capability.get("idempotent")
    dedup_scope = capability.get("dedup_scope")
    dedup_fields = capability.get("dedup_fields")
    once_per_task = capability.get("once_per_task")

    if effect not in ALLOWED_EFFECTS:
        findings.append(f"{prefix}: invalid_or_missing_effect={effect!r}")
    if risk_level not in ALLOWED_RISK_LEVELS:
        findings.append(f"{prefix}: invalid_or_missing_risk_level={risk_level!r}")
    if not isinstance(idempotent, bool):
        findings.append(f"{prefix}: idempotent_must_be_bool value={idempotent!r}")
    if dedup_scope not in ALLOWED_DEDUP_SCOPES:
        findings.append(f"{prefix}: invalid_or_missing_dedup_scope={dedup_scope!r}")
    if dedup_scope == "resource":
        declared_args = {
            token
            for raw in [*(capability.get("required") or []), *(capability.get("optional") or [])]
            for token in str(raw).split("|")
        }
        if not isinstance(dedup_fields, list) or not dedup_fields:
            findings.append(f"{prefix}: resource_dedup_requires_fields")
        elif any(
            not isinstance(field, str)
            or not field.strip()
            or field not in declared_args
            for field in dedup_fields
        ):
            findings.append(f"{prefix}: resource_dedup_field_not_declared={dedup_fields!r}")
    elif dedup_fields:
        findings.append(f"{prefix}: dedup_fields_require_resource_scope")

    controlled_side_effect = effect == "mutate" or risk_level == "high"
    if controlled_side_effect:
        if idempotent is not False:
            findings.append(f"{prefix}: controlled_side_effect_must_be_non_idempotent")
        if once_per_task is not True:
            findings.append(f"{prefix}: controlled_side_effect_requires_once_per_task")

    return findings


def skill_requires_planner_capabilities(skill: dict[str, Any]) -> bool:
    enabled = skill.get("enabled", True)
    planner_visible = skill.get("planner_visible", True)
    return enabled is not False and planner_visible is not False


def check_skill_capability_surface(registry_path: Path, skill: dict[str, Any]) -> list[str]:
    if not skill_requires_planner_capabilities(skill):
        return []
    capabilities = skill.get("planner_capabilities") or []
    if capabilities:
        return []
    skill_name = str(skill.get("name") or "unknown_skill").strip() or "unknown_skill"
    prefix = f"{registry_path.relative_to(ROOT)}:{skill_name}"
    return [f"{prefix}: planner_visible_enabled_skill_missing_planner_capabilities"]


def scan_registries(registries: list[Path]) -> tuple[list[str], int]:
    findings: list[str] = []
    capability_count = 0
    for registry_path in registries:
        for skill in load_registry(registry_path):
            findings.extend(check_skill_capability_surface(registry_path, skill))
            for index, capability in enumerate(skill.get("planner_capabilities") or []):
                capability_count += 1
                findings.extend(
                    check_capability(registry_path, skill, index, capability)
                )
    return findings, capability_count


def run_self_test() -> int:
    registry_path = REGISTRIES[0]
    good_skill = {
        "name": "good_skill",
        "planner_capabilities": [
            {
                "name": "good.observe",
                "effect": "observe",
                "risk_level": "low",
                "idempotent": True,
                "dedup_scope": "args",
            }
        ],
    }
    bad_capability = {
        "name": "bad.mutate",
        "effect": "mutate",
        "risk_level": "high",
        "idempotent": True,
        "dedup_scope": "bad_scope",
    }
    bad_resource_capability = {
        "name": "bad.resource",
        "effect": "observe",
        "risk_level": "low",
        "idempotent": True,
        "dedup_scope": "resource",
        "optional": ["path"],
        "dedup_fields": ["missing"],
    }
    missing_surface = {"name": "visible_without_capabilities"}
    if check_capability(registry_path, good_skill, 0, good_skill["planner_capabilities"][0]):
        print("SELF_TEST_FAIL good_policy_metadata_false_positive", file=sys.stderr)
        return 1
    bad_findings = check_capability(registry_path, {"name": "bad_skill"}, 0, bad_capability)
    expected_tokens = {
        "invalid_or_missing_dedup_scope",
        "controlled_side_effect_must_be_non_idempotent",
        "controlled_side_effect_requires_once_per_task",
    }
    observed_tokens = {
        token
        for finding in bad_findings
        for token in expected_tokens
        if token in finding
    }
    if not expected_tokens.issubset(observed_tokens):
        print(f"SELF_TEST_FAIL missing_bad_policy_findings:{bad_findings}", file=sys.stderr)
        return 1
    resource_findings = check_capability(
        registry_path, {"name": "bad_resource_skill"}, 0, bad_resource_capability
    )
    if not any("resource_dedup_field_not_declared" in finding for finding in resource_findings):
        print(f"SELF_TEST_FAIL missing_resource_dedup_finding:{resource_findings}", file=sys.stderr)
        return 1
    surface_findings = check_skill_capability_surface(registry_path, missing_surface)
    if not any("planner_visible_enabled_skill_missing_planner_capabilities" in finding for finding in surface_findings):
        print(f"SELF_TEST_FAIL missing_surface_finding:{surface_findings}", file=sys.stderr)
        return 1
    print("REGISTRY_POLICY_CONTRACT_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings, capability_count = scan_registries(REGISTRIES)

    if findings:
        print(
            "REGISTRY_POLICY_CONTRACT_CHECK "
            f"findings={len(findings)} capabilities={capability_count}"
        )
        for finding in findings:
            print(finding)
        return 1

    print(
        "REGISTRY_POLICY_CONTRACT_CHECK "
        f"ok registries={len(REGISTRIES)} capabilities={capability_count}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
