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

from builtin_contract_source_scan import (
    error_field_inventory,
    output_contract_pipeline,
    path_authority_inventory,
    self_test as source_scan_self_test,
    skill_producer_files,
)


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "configs" / "skills_registry.toml"
POLICY = ROOT / "scripts" / "baselines" / "builtin_capability_inventory_policy.json"

TEXT_OUTPUT_DISPOSITIONS = {
    "structured_extra_schema_drift",
    "direct_json_schema_drift",
    "true_text_producer",
    "non_public_compatibility_wrapper",
}
UNCONSTRAINED_ACTION_DISPOSITIONS = {"constrain_schema", "internal_nonplanner"}
UNMAPPED_ACTION_DISPOSITIONS = {
    "add_canonical_mapping",
    "alias_existing",
    "admin_direct",
    "internal",
    "deprecate",
    "remove",
}

# Baseline captured on 2026-07-27. These ceilings only prevent regression;
# completed vertical slices must lower them instead of adding exemptions.
MAX_TEXT_ONLY_OUTPUT_SKILLS = 0
MAX_UNCONSTRAINED_ACTION_SKILLS = 0
MAX_MISSING_PLANNER_ARGUMENTS = 0
MAX_EXPOSED_DUPLICATE_ACTION_GROUPS = 0
# Finite action enums exposed nine previously implicit compatibility/internal
# executor actions during the 2026-07-29 contract hardening rebaseline.
MAX_UNMAPPED_PUBLIC_ACTIONS = 45


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


def load_policy(path: Path) -> dict[str, Any]:
    try:
        policy = json.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise SystemExit(f"failed_to_read_inventory_policy path={path} error={exc}") from exc
    except json.JSONDecodeError as exc:
        raise SystemExit(f"failed_to_parse_inventory_policy path={path} error={exc}") from exc
    if not isinstance(policy, dict) or policy.get("schema_version") != 1:
        raise SystemExit(f"invalid_inventory_policy path={path}")
    return policy


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
        producer_files = skill_producer_files(root, skill)
        row["output_contract_pipeline"] = output_contract_pipeline(
            root, skill, producer_files
        )
        row["path_authority"] = path_authority_inventory(
            root, skill, properties, producer_files
        )
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


def _policy_entry_findings(
    category: str,
    key: str,
    entry: Any,
    allowed_dispositions: set[str],
) -> list[str]:
    if not isinstance(entry, dict):
        return [f"{category}_classification_invalid:{key}"]
    findings: list[str] = []
    disposition = str(entry.get("disposition") or "").strip()
    if disposition not in allowed_dispositions:
        findings.append(f"{category}_disposition_invalid:{key}:{disposition or '<empty>'}")
    if not str(entry.get("owner_wave") or "").strip():
        findings.append(f"{category}_owner_wave_missing:{key}")
    if not str(entry.get("rationale") or "").strip():
        findings.append(f"{category}_rationale_missing:{key}")
    if disposition == "alias_existing" and not str(entry.get("target") or "").strip():
        findings.append(f"{category}_alias_target_missing:{key}")
    return findings


def classify_debt(
    result: dict[str, Any], policy: dict[str, Any]
) -> tuple[dict[str, Any], list[str]]:
    findings: list[str] = []
    classifications: dict[str, Any] = {}

    text_debt = set(result["debt"]["text_only_output_skills"])
    text_policy = policy.get("text_only_outputs") or {}
    if not isinstance(text_policy, dict):
        text_policy = {}
        findings.append("text_output_policy_not_object")
    text_policy_keys = set(text_policy)
    for name in sorted(text_debt - text_policy_keys):
        findings.append(f"text_output_classification_missing:{name}")
    for name in sorted(text_policy_keys - text_debt):
        findings.append(f"text_output_classification_stale:{name}")
    for name in sorted(text_debt & text_policy_keys):
        findings.extend(
            _policy_entry_findings(
                "text_output", name, text_policy[name], TEXT_OUTPUT_DISPOSITIONS
            )
        )
    classifications["text_only_outputs"] = [
        {"skill": name, **text_policy[name]}
        for name in sorted(text_debt & text_policy_keys)
        if isinstance(text_policy[name], dict)
    ]

    unconstrained_debt = set(result["debt"]["unconstrained_action_skills"])
    unconstrained_policy = policy.get("unconstrained_actions") or {}
    if not isinstance(unconstrained_policy, dict):
        unconstrained_policy = {}
        findings.append("unconstrained_action_policy_not_object")
    unconstrained_policy_keys = set(unconstrained_policy)
    for name in sorted(unconstrained_debt - unconstrained_policy_keys):
        findings.append(f"unconstrained_action_classification_missing:{name}")
    for name in sorted(unconstrained_policy_keys - unconstrained_debt):
        findings.append(f"unconstrained_action_classification_stale:{name}")
    for name in sorted(unconstrained_debt & unconstrained_policy_keys):
        findings.extend(
            _policy_entry_findings(
                "unconstrained_action",
                name,
                unconstrained_policy[name],
                UNCONSTRAINED_ACTION_DISPOSITIONS,
            )
        )
    classifications["unconstrained_actions"] = [
        {"skill": name, **unconstrained_policy[name]}
        for name in sorted(unconstrained_debt & unconstrained_policy_keys)
        if isinstance(unconstrained_policy[name], dict)
    ]

    unmapped_debt = {
        (str(item["skill"]), str(item["action"]))
        for item in result["debt"]["unmapped_public_actions"]
    }
    unmapped_policy_list = policy.get("unmapped_public_actions") or []
    if not isinstance(unmapped_policy_list, list):
        unmapped_policy_list = []
        findings.append("unmapped_action_policy_not_array")
    unmapped_policy: dict[tuple[str, str], dict[str, Any]] = {}
    for raw in unmapped_policy_list:
        if not isinstance(raw, dict):
            findings.append("unmapped_action_classification_invalid:<non-object>")
            continue
        key = (
            str(raw.get("skill") or "").strip(),
            str(raw.get("action") or "").strip(),
        )
        printable = ".".join(key)
        if not all(key):
            findings.append(f"unmapped_action_key_invalid:{printable}")
            continue
        if key in unmapped_policy:
            findings.append(f"unmapped_action_classification_duplicate:{printable}")
            continue
        unmapped_policy[key] = raw
    for key in sorted(unmapped_debt - set(unmapped_policy)):
        findings.append(f"unmapped_action_classification_missing:{'.'.join(key)}")
    for key in sorted(set(unmapped_policy) - unmapped_debt):
        findings.append(f"unmapped_action_classification_stale:{'.'.join(key)}")
    for key in sorted(unmapped_debt & set(unmapped_policy)):
        findings.extend(
            _policy_entry_findings(
                "unmapped_action",
                ".".join(key),
                unmapped_policy[key],
                UNMAPPED_ACTION_DISPOSITIONS,
            )
        )
    classifications["unmapped_public_actions"] = [
        dict(unmapped_policy[key])
        for key in sorted(unmapped_debt & set(unmapped_policy))
    ]
    return classifications, findings


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
    positive_policy = {
        "schema_version": 1,
        "text_only_outputs": {
            "demo": {
                "disposition": "true_text_producer",
                "owner_wave": "3",
                "rationale": "fixture",
            }
        },
        "unconstrained_actions": {
            "demo": {
                "disposition": "constrain_schema",
                "owner_wave": "1",
                "rationale": "fixture",
            }
        },
        "unmapped_public_actions": [
            {
                "skill": "broken",
                "action": "inspect",
                "disposition": "alias_existing",
                "target": "broken.run",
                "owner_wave": "3",
                "rationale": "fixture",
            }
        ],
    }
    classifications, policy_findings = classify_debt(result, positive_policy)
    assert not policy_findings
    assert len(classifications["text_only_outputs"]) == 1
    negative_policy = json.loads(json.dumps(positive_policy))
    negative_policy["unmapped_public_actions"][0].pop("target")
    negative_policy["text_only_outputs"]["stale"] = {
        "disposition": "true_text_producer",
        "owner_wave": "3",
        "rationale": "fixture",
    }
    _, negative_findings = classify_debt(result, negative_policy)
    assert "unmapped_action_alias_target_missing:broken.inspect" in negative_findings
    assert "text_output_classification_stale:stale" in negative_findings
    with tempfile.TemporaryDirectory(prefix="builtin-contract-source-scan-") as tmp:
        source_scan_self_test(Path(tmp))
    print("BUILTIN_CAPABILITY_INVENTORY_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    parser.add_argument("--policy", type=Path, default=POLICY)
    parser.add_argument("--compare-registry", type=Path)
    parser.add_argument("--output", type=Path, help="write the full JSON inventory")
    parser.add_argument("--json", action="store_true", help="print full JSON inventory")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    result = inventory(load_skills(args.registry))
    result["error_field_inventory"] = error_field_inventory(ROOT)
    classifications, policy_findings = classify_debt(result, load_policy(args.policy))
    result["debt_classifications"] = classifications
    result["registry"] = str(args.registry)
    findings = findings_for(result["metrics"])
    findings.extend(policy_findings)
    findings.extend(
        f"unowned_legacy_error_writer:{skill}"
        for skill in result["error_field_inventory"]["unowned_skill_writers"]
    )
    if args.compare_registry is not None:
        comparison = inventory(load_skills(args.compare_registry))
        comparable = {
            key: value
            for key, value in result.items()
            if key not in {"debt_classifications", "error_field_inventory", "registry"}
        }
        result["comparison_registry"] = str(args.compare_registry)
        result["comparison_equal"] = comparison == comparable
        if not result["comparison_equal"]:
            findings.append("registry_inventory_differs")
    rendered = json.dumps(result, ensure_ascii=True, indent=2, sort_keys=True)
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(f"{rendered}\n", encoding="utf-8")
    if args.json:
        print(rendered)
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
