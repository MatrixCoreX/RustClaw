#!/usr/bin/env python3
"""Validate planner-capability policy metadata in skill registries."""

from __future__ import annotations

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
ALLOWED_DEDUP_SCOPES = {"args", "action", "none"}


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
    once_per_task = capability.get("once_per_task")

    if effect not in ALLOWED_EFFECTS:
        findings.append(f"{prefix}: invalid_or_missing_effect={effect!r}")
    if risk_level not in ALLOWED_RISK_LEVELS:
        findings.append(f"{prefix}: invalid_or_missing_risk_level={risk_level!r}")
    if not isinstance(idempotent, bool):
        findings.append(f"{prefix}: idempotent_must_be_bool value={idempotent!r}")
    if dedup_scope not in ALLOWED_DEDUP_SCOPES:
        findings.append(f"{prefix}: invalid_or_missing_dedup_scope={dedup_scope!r}")

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


def main() -> int:
    findings: list[str] = []
    capability_count = 0
    for registry_path in REGISTRIES:
        for skill in load_registry(registry_path):
            findings.extend(check_skill_capability_surface(registry_path, skill))
            for index, capability in enumerate(skill.get("planner_capabilities") or []):
                capability_count += 1
                findings.extend(
                    check_capability(registry_path, skill, index, capability)
                )

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
    sys.exit(main())
