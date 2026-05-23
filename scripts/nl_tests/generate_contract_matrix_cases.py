#!/usr/bin/env python3
"""Generate deterministic contract-matrix regression cases as JSONL.

This is an offline seed generator. It does not call clawd or a model; live NL
replay can consume the emitted contract ids, expected actions, evidence fields,
and final answer shapes.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path
from typing import Any


DEFAULT_MATRIX = Path("configs/task_contract_matrix.toml")


PROBE_ACTIONS = [
    "run_cmd",
    "fs_basic.list_dir",
    "fs_basic.read_text_range",
    "fs_basic.write_text",
    "fs_basic.find_entries",
    "archive_basic.pack",
    "archive_basic.read",
    "config_basic.validate",
    "docker_basic",
    "package_manager.detect",
    "db_basic",
    "health_check",
    "respond",
]


def normalize_token(value: str) -> str:
    return value.strip().lower()


def parse_action(raw: str) -> tuple[str, str | None]:
    raw = normalize_token(raw).replace("-", "_")
    if "." not in raw:
        return raw, None
    skill, action = raw.split(".", 1)
    return skill, action or None


def action_matches(action: str, policies: list[str]) -> bool:
    action_skill, action_name = parse_action(action)
    for policy in policies:
        policy_skill, policy_name = parse_action(policy)
        if action_skill != policy_skill:
            continue
        if policy_name is None or action_name == policy_name:
            return True
    return False


def action_policy(action: str, contract: dict[str, Any]) -> str:
    if action_matches(action, contract.get("forbidden_actions", [])):
        return "rejected_forbidden"
    allowed = contract.get("allowed_actions", [])
    if not allowed:
        return "allowed" if contract.get("none_passthrough") else "rejected_no_actions_allowed"
    if action_matches(action, allowed):
        return "allowed"
    return "rejected_not_allowed"


def normalized_evidence(contract: dict[str, Any]) -> list[str]:
    return sorted({normalize_token(item) for item in contract.get("required_evidence", []) if item})


def matrix_hash(matrix: dict[str, Any]) -> str:
    contracts = matrix.get("contracts", {})
    profiles = matrix.get("generic_profiles", [])
    parts = [
        str(matrix.get("schema_version", 1)),
        str(matrix.get("matrix_version", "")),
        str(len(contracts)),
        str(len(profiles)),
    ]
    for key in sorted(contracts):
        parts.append(f"{key}:{','.join(normalized_evidence(contracts[key]))}")
    text = "|".join(parts)
    h = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        h ^= byte
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def base_case(
    matrix: dict[str, Any],
    contract_type: str,
    contract_id: str,
    contract: dict[str, Any],
    phase: str,
    action_ref: str | None,
    expected_decision: str | None,
) -> dict[str, Any]:
    return {
        "case_id": ".".join(
            item
            for item in [
                contract_type,
                contract_id,
                phase,
                normalize_token(action_ref).replace(".", "_") if action_ref else None,
            ]
            if item
        ),
        "source": "task_contract_matrix",
        "matrix_version": matrix.get("matrix_version"),
        "matrix_hash": matrix_hash(matrix),
        "contract_type": contract_type,
        "contract_id": contract_id,
        "semantic_kind": contract.get("semantic_kind"),
        "phase": phase,
        "action_ref": action_ref,
        "expected_policy_decision": expected_decision,
        "required_evidence": normalized_evidence(contract),
        "final_answer_shape": contract.get("final_answer_shape", ""),
        "allowed_actions": sorted({normalize_token(item) for item in contract.get("allowed_actions", [])}),
        "forbidden_actions": sorted({normalize_token(item) for item in contract.get("forbidden_actions", [])}),
        "failure_policy": contract.get("failure_policy", ""),
    }


def generate_all_cases(matrix: dict[str, Any]) -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    contracts = matrix.get("contracts", {})
    for contract_id in sorted(contracts):
        contract = contracts[contract_id]
        cases.extend(generate_contract_cases(matrix, "semantic", contract_id, contract))
    for profile in matrix.get("generic_profiles", []):
        contract_id = profile.get("name", "unnamed_generic")
        cases.extend(generate_contract_cases(matrix, "generic", contract_id, profile))
    return unique_cases(cases)


def generate_contract_cases(
    matrix: dict[str, Any],
    contract_type: str,
    contract_id: str,
    contract: dict[str, Any],
) -> list[dict[str, Any]]:
    cases = [
        base_case(matrix, contract_type, contract_id, contract, "evidence_shape", None, None)
    ]
    for action in sorted({normalize_token(item) for item in contract.get("allowed_actions", [])}):
        cases.append(
            base_case(
                matrix,
                contract_type,
                contract_id,
                contract,
                "allowed_action",
                action,
                action_policy(action, contract),
            )
        )
    for action in sorted({normalize_token(item) for item in contract.get("forbidden_actions", [])}):
        cases.append(
            base_case(
                matrix,
                contract_type,
                contract_id,
                contract,
                "negative_action",
                action,
                action_policy(action, contract),
            )
        )
    for action in PROBE_ACTIONS:
        decision = action_policy(action, contract)
        if decision != "allowed":
            cases.append(
                base_case(
                    matrix,
                    contract_type,
                    contract_id,
                    contract,
                    "negative_action",
                    action,
                    decision,
                )
            )
    return cases


def unique_cases(cases: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: set[str] = set()
    out: list[dict[str, Any]] = []
    for case in cases:
        case_id = case["case_id"]
        if case_id in seen:
            continue
        seen.add(case_id)
        out.append(case)
    return out


def select_cases(cases: list[dict[str, Any]], count: int, batch: int) -> list[dict[str, Any]]:
    if count <= 0 or count >= len(cases):
        return cases
    mandatory: list[dict[str, Any]] = []
    mandatory.extend([case for case in cases if case["phase"] == "evidence_shape"])
    first_allowed: dict[str, dict[str, Any]] = {}
    for case in cases:
        if case["phase"] == "allowed_action":
            first_allowed.setdefault(case["contract_id"], case)
    mandatory.extend(first_allowed.values())
    first_decision: dict[str, dict[str, Any]] = {}
    for case in cases:
        decision = case.get("expected_policy_decision")
        if decision in {"allowed", "rejected_forbidden", "rejected_not_allowed"}:
            first_decision.setdefault(decision, case)
    mandatory.extend(first_decision.values())
    mandatory = unique_cases(mandatory)
    if len(mandatory) >= count:
        return mandatory[:count]

    mandatory_ids = {case["case_id"] for case in mandatory}
    extras = [case for case in cases if case["case_id"] not in mandatory_ids]
    offset = (batch * max(1, count - len(mandatory))) % len(extras) if extras else 0
    rotated = extras[offset:] + extras[:offset]
    return unique_cases(mandatory + rotated)[:count]


def coverage_report(cases: list[dict[str, Any]]) -> dict[str, Any]:
    semantics = sorted(
        {
            case["semantic_kind"]
            for case in cases
            if case["contract_type"] == "semantic" and case.get("semantic_kind")
        }
    )
    generic_profiles = sorted(
        {case["contract_id"] for case in cases if case["contract_type"] == "generic"}
    )
    decisions = sorted(
        {
            case["expected_policy_decision"]
            for case in cases
            if case.get("expected_policy_decision")
        }
    )
    phases = sorted({case["phase"] for case in cases})
    return {
        "case_count": len(cases),
        "semantic_count": len(semantics),
        "generic_profile_count": len(generic_profiles),
        "phase_count": len(phases),
        "policy_decisions": decisions,
        "phases": phases,
    }


def validate_selected_cases(cases: list[dict[str, Any]], requested_count: int) -> list[str]:
    errors: list[str] = []
    if requested_count > 0 and len(cases) < requested_count:
        errors.append(f"only generated {len(cases)} cases, requested {requested_count}")
    ids = [case["case_id"] for case in cases]
    if len(ids) != len(set(ids)):
        errors.append("generated duplicate case ids")
    report = coverage_report(cases)
    if report["case_count"] >= 100 and report["semantic_count"] == 0:
        errors.append("generated cases do not cover semantic contracts")
    for decision in ("allowed", "rejected_forbidden", "rejected_not_allowed"):
        if decision not in report["policy_decisions"]:
            errors.append(f"generated cases do not include policy decision {decision}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--batch", type=int, default=0)
    parser.add_argument("--report", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    with args.matrix.open("rb") as fh:
        matrix = tomllib.load(fh)
    cases = select_cases(generate_all_cases(matrix), args.count, args.batch)

    if args.check:
        errors = validate_selected_cases(cases, args.count)
        if errors:
            for error in errors:
                print(f"ERROR: {error}", file=sys.stderr)
            return 1

    for case in cases:
        print(json.dumps(case, ensure_ascii=False, sort_keys=True))

    if args.report:
        print(json.dumps(coverage_report(cases), ensure_ascii=False, sort_keys=True), file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
