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
ALLOWED_EXECUTION_MODES = {"sync_short", "async_preferred", "async_required"}
ALLOWED_ASYNC_ADAPTER_KINDS = {
    "skill_poll",
    "local_process_poll",
    "http_job_poll",
    "mcp_job_poll",
    "media_job_poll",
    "browser_job_poll",
    "remote_job_poll",
}
ALLOWED_ISOLATION_PROFILES = {
    "read_only",
    "local_current_workspace",
    "local_worktree",
    "local_temp_workspace",
    "remote_executor",
}
PERMISSION_BOOL_FIELDS = (
    "network_access",
    "filesystem_write",
    "external_publish",
    "credential_access",
    "subprocess",
    "package_install",
    "privilege_escalation",
)
CORE_RUNTIME_PATH = ROOT / "crates" / "clawd" / "src" / "agent_engine" / "skill_execution.rs"
CAPABILITY_RESULT_ADAPTER_PATH = ROOT / "crates" / "clawd" / "src" / "capability_result.rs"
ALIAS_POLICY_FIELDS = (
    "action",
    "effect",
    "risk_level",
    "once_per_task",
    "dedup_scope",
    "dedup_fields",
    "idempotent",
    "execution_mode",
    "async_adapter_kind",
    "isolation_profile",
    "network_access",
    "filesystem_write",
    "external_publish",
    "credential_access",
    "subprocess",
    "package_install",
    "privilege_escalation",
    "final_answer_shape",
)


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

    execution_mode = capability.get("execution_mode")
    async_adapter_kind = capability.get("async_adapter_kind")
    if execution_mode not in ALLOWED_EXECUTION_MODES:
        findings.append(f"{prefix}: invalid_or_missing_execution_mode={execution_mode!r}")
    if execution_mode in {"async_preferred", "async_required"}:
        if not isinstance(async_adapter_kind, str) or not async_adapter_kind.strip():
            findings.append(f"{prefix}: async_execution_requires_adapter")
        elif async_adapter_kind not in ALLOWED_ASYNC_ADAPTER_KINDS:
            findings.append(
                f"{prefix}: async_execution_adapter_not_registered={async_adapter_kind!r}"
            )
    elif async_adapter_kind:
        findings.append(f"{prefix}: sync_execution_must_not_declare_async_adapter")

    isolation_profile = normalized_policy_value(capability, "isolation_profile")
    if isolation_profile not in ALLOWED_ISOLATION_PROFILES:
        findings.append(
            f"{prefix}: invalid_or_missing_isolation_profile={isolation_profile!r}"
        )
    effective_flags = {
        field: normalized_policy_value(capability, field)
        for field in PERMISSION_BOOL_FIELDS
    }
    for field, value in effective_flags.items():
        if not isinstance(value, bool):
            findings.append(f"{prefix}: invalid_or_missing_{field}={value!r}")
    if isolation_profile == "read_only":
        for field in (
            "network_access",
            "filesystem_write",
            "external_publish",
            "credential_access",
            "package_install",
            "privilege_escalation",
        ):
            if effective_flags[field] is True:
                findings.append(f"{prefix}: read_only_profile_forbids_{field}")
    if effective_flags["package_install"] is True and effective_flags["subprocess"] is not True:
        findings.append(f"{prefix}: package_install_requires_subprocess")
    if (
        effective_flags["privilege_escalation"] is True
        and effective_flags["subprocess"] is not True
    ):
        findings.append(f"{prefix}: privilege_escalation_requires_subprocess")

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


def normalized_policy_value(capability: dict[str, Any], field: str) -> Any:
    effect = capability.get("effect")
    if field == "isolation_profile" and field not in capability:
        return {
            "observe": "read_only",
            "validate": "read_only",
            "mutate": "local_current_workspace",
            "external": "remote_executor",
        }.get(effect)
    if field == "network_access" and field not in capability:
        return effect == "external"
    if field == "filesystem_write" and field not in capability:
        return effect == "mutate"
    if field == "external_publish" and field not in capability:
        return effect == "external"
    if field == "credential_access" and field not in capability:
        return False if effect in ALLOWED_EFFECTS else None
    if field in {"subprocess", "package_install", "privilege_escalation"}:
        return capability.get(field, False)
    return capability.get(field)


def check_core_skill_contract(registry_path: Path, skill: dict[str, Any]) -> list[str]:
    if skill.get("planner_eager_load") is not True:
        return []
    skill_name = str(skill.get("name") or "unknown_skill").strip() or "unknown_skill"
    prefix = f"{registry_path.relative_to(ROOT)}:{skill_name}"
    findings: list[str] = []
    timeout_seconds = skill.get("timeout_seconds")
    if not isinstance(timeout_seconds, int) or isinstance(timeout_seconds, bool) or timeout_seconds <= 0:
        findings.append(f"{prefix}: eager_core_requires_positive_timeout_seconds")
    supported_os = set(skill.get("supported_os") or [])
    for required_os in ("linux", "macos"):
        if required_os not in supported_os:
            findings.append(f"{prefix}: eager_core_missing_supported_os={required_os}")
    if not skill.get("planner_capabilities"):
        findings.append(f"{prefix}: eager_core_missing_planner_capabilities")
    return findings


def check_core_runtime_contract() -> list[str]:
    findings: list[str] = []
    try:
        execution = CORE_RUNTIME_PATH.read_text(encoding="utf-8")
        adapter = CAPABILITY_RESULT_ADAPTER_PATH.read_text(encoding="utf-8")
    except OSError as exc:
        return [f"core_runtime_contract_read_failed={exc}"]

    ordered_tokens = (
        "pre_tool_use_outcome_for_state",
        "crate::executor::execute_step",
        "record_post_tool_use_hook_observations",
    )
    owner_start = execution.find("pub(super) async fn execute_prepared_skill_action")
    positions = [execution.find(token, owner_start) for token in ordered_tokens]
    if any(position < 0 for position in positions) or positions != sorted(positions):
        findings.append(
            "core_runtime_hook_order_missing="
            + ",".join(ordered_tokens)
        )
    for token in (
        "task_cancellation_token(&task.task_id)",
        "run_with_tool_budget_timeout",
        "CancellationToken",
    ):
        if token not in execution:
            findings.append(f"core_runtime_cancellation_contract_missing={token}")
    for token in (
        "CapabilityResultEnvelope::ok",
        "CapabilityResultEnvelope::failed",
        "apply_result_metadata",
    ):
        if token not in adapter:
            findings.append(f"core_runtime_evidence_envelope_missing={token}")
    return findings


def check_skill_capability_aliases(
    registry_path: Path, skill: dict[str, Any]
) -> tuple[list[str], int]:
    aliases = skill.get("planner_capability_aliases") or {}
    if not isinstance(aliases, dict):
        skill_name = str(skill.get("name") or "unknown_skill")
        return (
            [
                f"{registry_path.relative_to(ROOT)}:{skill_name}: "
                "planner_capability_aliases_must_be_table"
            ],
            0,
        )
    capabilities = {
        str(capability.get("name") or "").strip(): capability
        for capability in skill.get("planner_capabilities") or []
    }
    skill_name = str(skill.get("name") or "unknown_skill").strip() or "unknown_skill"
    prefix = f"{registry_path.relative_to(ROOT)}:{skill_name}"
    findings: list[str] = []
    for raw_alias, raw_target in aliases.items():
        alias = str(raw_alias).strip()
        target = str(raw_target).strip()
        if alias == target:
            findings.append(f"{prefix}: capability_alias_self_reference={alias}")
            continue
        if target in aliases:
            findings.append(f"{prefix}: capability_alias_chain={alias}->{target}")
        alias_capability = capabilities.get(alias)
        target_capability = capabilities.get(target)
        if alias_capability is None:
            findings.append(f"{prefix}: capability_alias_missing_mapping={alias}")
            continue
        if target_capability is None:
            findings.append(f"{prefix}: capability_alias_missing_target={alias}->{target}")
            continue
        drift = [
            field
            for field in ALIAS_POLICY_FIELDS
            if normalized_policy_value(alias_capability, field)
            != normalized_policy_value(target_capability, field)
        ]
        if drift:
            findings.append(
                f"{prefix}: capability_alias_policy_drift={alias}->{target} fields={','.join(drift)}"
            )
        target_args = set(target_capability.get("required") or []) | set(
            target_capability.get("optional") or []
        )
        alias_args = set(alias_capability.get("required") or []) | set(
            alias_capability.get("optional") or []
        )
        uncovered = sorted(alias_args - target_args)
        if uncovered:
            findings.append(
                f"{prefix}: capability_alias_arguments_uncovered={alias}->{target} "
                f"args={','.join(uncovered)}"
            )
    return findings, len(aliases)


def check_registry_global_aliases(
    registry_path: Path, skills: list[dict[str, Any]]
) -> list[str]:
    owners: dict[str, tuple[str, str]] = {}
    findings: list[str] = []
    prefix = str(registry_path.relative_to(ROOT))
    for skill in skills:
        skill_name = str(skill.get("name") or "unknown_skill").strip() or "unknown_skill"
        aliases = skill.get("planner_capability_aliases") or {}
        if not isinstance(aliases, dict):
            continue
        for raw_alias, raw_target in aliases.items():
            alias = str(raw_alias).strip()
            target = str(raw_target).strip()
            if alias in owners:
                existing_skill, existing_target = owners[alias]
                findings.append(
                    f"{prefix}: duplicate_capability_alias={alias} "
                    f"owners={existing_skill}->{existing_target},{skill_name}->{target}"
                )
            else:
                owners[alias] = (skill_name, target)
    for alias, (skill_name, target) in owners.items():
        if target in owners:
            target_skill, next_target = owners[target]
            findings.append(
                f"{prefix}: cross_skill_capability_alias_chain={alias}({skill_name})"
                f"->{target}({target_skill})->{next_target}"
            )
    return findings


def scan_registries(registries: list[Path]) -> tuple[list[str], int, int]:
    findings: list[str] = []
    capability_count = 0
    alias_count = 0
    for registry_path in registries:
        skills = load_registry(registry_path)
        findings.extend(check_registry_global_aliases(registry_path, skills))
        for skill in skills:
            findings.extend(check_core_skill_contract(registry_path, skill))
            findings.extend(check_skill_capability_surface(registry_path, skill))
            alias_findings, skill_alias_count = check_skill_capability_aliases(
                registry_path, skill
            )
            findings.extend(alias_findings)
            alias_count += skill_alias_count
            for index, capability in enumerate(skill.get("planner_capabilities") or []):
                capability_count += 1
                findings.extend(
                    check_capability(registry_path, skill, index, capability)
                )
    findings.extend(check_core_runtime_contract())
    return findings, capability_count, alias_count


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
                "execution_mode": "sync_short",
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
        "execution_mode": "sync_short",
    }
    bad_async_capability = {
        "name": "bad.async",
        "effect": "observe",
        "risk_level": "low",
        "idempotent": True,
        "dedup_scope": "args",
        "execution_mode": "async_preferred",
        "async_adapter_kind": "unknown_adapter",
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
    async_findings = check_capability(
        registry_path, {"name": "bad_async_skill"}, 0, bad_async_capability
    )
    if not any(
        "async_execution_adapter_not_registered" in finding
        for finding in async_findings
    ):
        print(f"SELF_TEST_FAIL missing_async_adapter_finding:{async_findings}", file=sys.stderr)
        return 1
    surface_findings = check_skill_capability_surface(registry_path, missing_surface)
    if not any("planner_visible_enabled_skill_missing_planner_capabilities" in finding for finding in surface_findings):
        print(f"SELF_TEST_FAIL missing_surface_finding:{surface_findings}", file=sys.stderr)
        return 1
    alias_findings, alias_count = check_skill_capability_aliases(
        registry_path,
        {
            "name": "alias_skill",
            "planner_capability_aliases": {"legacy.read": "canonical.read"},
            "planner_capabilities": [
                {
                    "name": "canonical.read",
                    "action": "read",
                    "effect": "observe",
                    "risk_level": "low",
                    "idempotent": True,
                    "dedup_scope": "args",
                    "required": ["path"],
                },
                {
                    "name": "legacy.read",
                    "action": "read",
                    "effect": "observe",
                    "risk_level": "low",
                    "idempotent": True,
                    "dedup_scope": "args",
                    "required": ["path"],
                },
            ],
        },
    )
    if alias_findings or alias_count != 1:
        print(f"SELF_TEST_FAIL valid_alias_rejected:{alias_findings}", file=sys.stderr)
        return 1
    duplicate_findings = check_registry_global_aliases(
        registry_path,
        [
            {
                "name": "first",
                "planner_capability_aliases": {"legacy.read": "first.read"},
            },
            {
                "name": "second",
                "planner_capability_aliases": {"legacy.read": "second.read"},
            },
        ],
    )
    if not any("duplicate_capability_alias" in finding for finding in duplicate_findings):
        print(
            f"SELF_TEST_FAIL missing_duplicate_alias_finding:{duplicate_findings}",
            file=sys.stderr,
        )
        return 1
    print("REGISTRY_POLICY_CONTRACT_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings, capability_count, alias_count = scan_registries(REGISTRIES)

    if findings:
        print(
            "REGISTRY_POLICY_CONTRACT_CHECK "
            f"findings={len(findings)} capabilities={capability_count} aliases={alias_count}"
        )
        for finding in findings:
            print(finding)
        return 1

    print(
        "REGISTRY_POLICY_CONTRACT_CHECK "
        f"ok registries={len(REGISTRIES)} capabilities={capability_count} aliases={alias_count}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
