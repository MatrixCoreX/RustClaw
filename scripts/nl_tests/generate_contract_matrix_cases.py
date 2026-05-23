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


def choose_first_case(
    cases: list[dict[str, Any]],
    seen_case_ids: set[str],
    predicate: Any,
) -> dict[str, Any] | None:
    unseen = [case for case in cases if case["case_id"] not in seen_case_ids and predicate(case)]
    if unseen:
        return unseen[0]
    for case in cases:
        if predicate(case):
            return case
    return None


def coverage_anchor_cases(cases: list[dict[str, Any]], seen_case_ids: set[str]) -> list[dict[str, Any]]:
    anchors: list[dict[str, Any]] = []
    semantic_ids = sorted(
        {
            case["contract_id"]
            for case in cases
            if case["contract_type"] == "semantic" and case.get("contract_id")
        }
    )
    generic_ids = sorted(
        {
            case["contract_id"]
            for case in cases
            if case["contract_type"] == "generic" and case.get("contract_id")
        }
    )
    phases = sorted({case["phase"] for case in cases if case.get("phase")})
    decisions = sorted(
        {
            case["expected_policy_decision"]
            for case in cases
            if case.get("expected_policy_decision")
        }
    )
    final_shapes = sorted(
        {case["final_answer_shape"] for case in cases if case.get("final_answer_shape")}
    )

    for contract_id in semantic_ids:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, contract_id=contract_id: item["contract_type"] == "semantic"
            and item["contract_id"] == contract_id,
        )
        if case:
            anchors.append(case)
    for contract_id in generic_ids:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, contract_id=contract_id: item["contract_type"] == "generic"
            and item["contract_id"] == contract_id,
        )
        if case:
            anchors.append(case)
    for phase in phases:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, phase=phase: item.get("phase") == phase,
        )
        if case:
            anchors.append(case)
    for decision in decisions:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, decision=decision: item.get("expected_policy_decision") == decision,
        )
        if case:
            anchors.append(case)
    for shape in final_shapes:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, shape=shape: item.get("final_answer_shape") == shape,
        )
        if case:
            anchors.append(case)
    return unique_cases(anchors)


def select_cases(
    cases: list[dict[str, Any]],
    count: int,
    batch: int,
    seen_case_ids: set[str] | None = None,
) -> list[dict[str, Any]]:
    seen_case_ids = seen_case_ids or set()
    if count <= 0 or count >= len(cases):
        return cases
    mandatory = coverage_anchor_cases(cases, seen_case_ids)
    if len(mandatory) >= count:
        return mandatory[:count]

    mandatory_ids = {case["case_id"] for case in mandatory}
    unseen_extras = [
        case
        for case in cases
        if case["case_id"] not in mandatory_ids and case["case_id"] not in seen_case_ids
    ]
    seen_extras = [
        case
        for case in cases
        if case["case_id"] not in mandatory_ids and case["case_id"] in seen_case_ids
    ]
    extras = unseen_extras or seen_extras
    offset = (batch * max(1, count - len(mandatory))) % len(extras) if extras else 0
    rotated = extras[offset:] + extras[:offset]
    selected = unique_cases(mandatory + rotated)
    if len(selected) < count and unseen_extras:
        selected = unique_cases(selected + seen_extras)
    return selected[:count]


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
    final_shapes = sorted(
        {case["final_answer_shape"] for case in cases if case.get("final_answer_shape")}
    )
    return {
        "case_count": len(cases),
        "contract_count": len(
            {
                (case["contract_type"], case["contract_id"])
                for case in cases
                if case.get("contract_type") and case.get("contract_id")
            }
        ),
        "semantic_count": len(semantics),
        "generic_profile_count": len(generic_profiles),
        "final_answer_shape_count": len(final_shapes),
        "phase_count": len(phases),
        "policy_decisions": decisions,
        "phases": phases,
    }


def validate_selected_cases(
    cases: list[dict[str, Any]],
    requested_count: int,
    matrix: dict[str, Any],
) -> list[str]:
    errors: list[str] = []
    if requested_count > 0 and len(cases) < requested_count:
        errors.append(f"only generated {len(cases)} cases, requested {requested_count}")
    ids = [case["case_id"] for case in cases]
    if len(ids) != len(set(ids)):
        errors.append("generated duplicate case ids")
    report = coverage_report(cases)
    if report["case_count"] >= 100 and report["semantic_count"] == 0:
        errors.append("generated cases do not cover semantic contracts")
    expected_semantics = set(matrix.get("contracts", {}))
    expected_generics = {
        profile.get("name", "unnamed_generic")
        for profile in matrix.get("generic_profiles", [])
    }
    expected_shapes = {
        contract.get("final_answer_shape", "")
        for contract in matrix.get("contracts", {}).values()
    } | {
        profile.get("final_answer_shape", "")
        for profile in matrix.get("generic_profiles", [])
    }
    selected_semantics = {
        case["contract_id"]
        for case in cases
        if case["contract_type"] == "semantic"
    }
    selected_generics = {
        case["contract_id"]
        for case in cases
        if case["contract_type"] == "generic"
    }
    selected_shapes = {
        case["final_answer_shape"]
        for case in cases
        if case.get("final_answer_shape")
    }
    if report["case_count"] >= 100:
        missing_semantics = sorted(expected_semantics - selected_semantics)
        missing_generics = sorted(expected_generics - selected_generics)
        missing_shapes = sorted(expected_shapes - selected_shapes)
        if missing_semantics:
            errors.append(f"generated cases miss semantic contracts: {missing_semantics}")
        if missing_generics:
            errors.append(f"generated cases miss generic profiles: {missing_generics}")
        if missing_shapes:
            errors.append(f"generated cases miss final answer shapes: {missing_shapes}")
    for decision in ("allowed", "rejected_forbidden", "rejected_not_allowed"):
        if decision not in report["policy_decisions"]:
            errors.append(f"generated cases do not include policy decision {decision}")
    return errors


def read_history_case_ids(path: Path | None) -> set[str]:
    if path is None or not path.exists():
        return set()
    seen: set[str] = set()
    with path.open("r", encoding="utf-8") as fh:
        for raw_line in fh:
            line = raw_line.strip()
            if not line:
                continue
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                seen.add(line)
                continue
            if isinstance(item, dict) and isinstance(item.get("case_id"), str):
                seen.add(item["case_id"])
            elif isinstance(item, str):
                seen.add(item)
    return seen


def append_history_case_ids(path: Path, cases: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as fh:
        for case in cases:
            fh.write(json.dumps({"case_id": case["case_id"]}, sort_keys=True))
            fh.write("\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--batch", type=int, default=0)
    parser.add_argument("--history", type=Path)
    parser.add_argument("--update-history", action="store_true")
    parser.add_argument("--report", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    if args.update_history and args.history is None:
        parser.error("--update-history requires --history")

    with args.matrix.open("rb") as fh:
        matrix = tomllib.load(fh)
    seen_case_ids = read_history_case_ids(args.history)
    cases = select_cases(generate_all_cases(matrix), args.count, args.batch, seen_case_ids)

    if args.check:
        errors = validate_selected_cases(cases, args.count, matrix)
        if errors:
            for error in errors:
                print(f"ERROR: {error}", file=sys.stderr)
            return 1

    for case in cases:
        print(json.dumps(case, ensure_ascii=False, sort_keys=True))

    if args.update_history and args.history is not None:
        append_history_case_ids(args.history, cases)

    if args.report:
        print(json.dumps(coverage_report(cases), ensure_ascii=False, sort_keys=True), file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
