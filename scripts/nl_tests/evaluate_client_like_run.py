#!/usr/bin/env python3
"""Evaluate client-like NL run logs against lightweight expectations.

This is an offline regression helper for runs produced by
run_client_like_continuous_suite.sh. It does not call clawd or any model.

Expectation JSONL rows are intentionally small and optional. Supported fields:

  {
    "case": 12,
    "status": "succeeded",
    "stop_signal": "recipe_repair_budget_exhausted",
    "stop_failure_attribution": "budget_exhausted",
    "routed_mode": "Act",
    "route_gate": "execute",
    "route_gate_any": ["chat", "execute"],
    "first_layer": "planner_execute",
    "first_layer_any": ["direct_answer", "planner_execute"],
    "capability_any": ["filesystem.list_entries", "fs_basic"],
    "planned_action_any": ["fs_basic.list_dir"],
    "planned_action_all": ["fs_basic.list_dir"],
    "planned_action_none_of": ["run_cmd"],
    "requested_action_any": ["fs_basic.list_dir"],
    "requested_action_none_of": ["run_cmd"],
    "executed_any": ["fs_basic"],
    "verifier_approved": true,
    "verifier_issue_any": ["MissingRequiredArg"],
    "verifier_failure_attribution_any": ["model_error"],
    "needs_confirmation": false,
    "final_contains": ["README.md"],
    "final_shape": "path|file_token|integer|list|non_empty|empty",
    "finalizer_stage": "observed_generic",
    "finalizer_fallback": "raw_text",
    "finalizer_final_answer_shape": "single_path",
    "finalizer_final_answer_shape_class": "single_path",
    "finalizer_coarse_response_shape": "scalar",
    "finalizer_allows_model_language": false,
    "finalizer_grounded_ok": true,
    "contract_match": "file_names",
    "contract_final_answer_shape": "name_list",
    "required_evidence_all": ["candidates"],
    "observed_evidence_any": ["candidates"],
    "observed_evidence_all": ["path", "exists"],
    "missing_evidence_empty": true,
    "executed_none_of": ["run_cmd"],
    "error_kind_any": ["contract_action_rejected"],
    "failure_attribution_any": ["contract_gap"],
    "contract_policy_decision_any": ["rejected_not_allowed"],
    "event_type_all": ["coding_checkpoint", "coding_evidence"],
    "event_field_all": ["checkpoint_kind=verification_command"]
  }

Use --write-baseline to capture the current observed route/plan/final shape.
"""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any


CASE_RE = re.compile(r"turn_(?P<turn>\d+)_case_(?P<case>\d+)\.json$")
PATH_RE = re.compile(r"(^|\s)(/[^:\n\r\t ]+|\.{1,2}/[^:\n\r\t ]+)")


def get_path(obj: Any, *path: str) -> Any:
    cur = obj
    for key in path:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def compact(value: Any, max_chars: int = 300) -> str:
    if value is None:
        return ""
    if isinstance(value, (dict, list)):
        text = json.dumps(value, ensure_ascii=False, sort_keys=True)
    else:
        text = str(value)
    text = text.strip()
    if len(text) > max_chars:
        return text[: max_chars - 3].rstrip() + "..."
    return text


def route_legacy_first_layer(route: Any) -> str:
    if not isinstance(route, dict):
        return ""
    return str(
        route.get("legacy_first_layer_decision")
        or route.get("first_layer_decision")
        or ""
    )


def sort_key(path: Path) -> tuple[int, int, str]:
    match = CASE_RE.search(path.name)
    if not match:
        return (10**9, 10**9, path.name)
    return (int(match.group("case")), int(match.group("turn")), path.name)


def final_text(result: dict[str, Any], summary: dict[str, Any]) -> str:
    for value in (summary.get("final_answer"), result.get("text"), result.get("message")):
        if isinstance(value, str) and value.strip():
            return value.strip()
    messages = result.get("messages")
    if isinstance(messages, list):
        for item in reversed(messages):
            if isinstance(item, str) and item.strip():
                return item.strip()
    return ""


def final_shape(text: str) -> str:
    stripped = text.strip()
    if not stripped:
        return "empty"
    lines = [line.strip() for line in stripped.splitlines() if line.strip()]
    if all(line.startswith(("FILE:", "IMAGE_FILE:")) for line in lines):
        return "file_token"
    if re.fullmatch(r"[+-]?\d+", stripped):
        return "integer"
    if len(lines) == 1 and PATH_RE.search(stripped):
        return "path"
    if len(lines) > 1:
        return "list"
    return "non_empty"


def collect_plan_targets(trace: dict[str, Any]) -> list[str]:
    targets: list[str] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return targets
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        plan = round_obj.get("plan_result")
        steps = plan.get("steps") if isinstance(plan, dict) else None
        if not isinstance(steps, list):
            continue
        for step in steps:
            if not isinstance(step, dict):
                continue
            for key in ("capability", "skill", "tool"):
                value = step.get(key)
                if isinstance(value, str) and value.strip():
                    targets.append(value.strip())
                    break
    return targets


def collect_plan_action_refs(trace: dict[str, Any]) -> list[str]:
    refs: list[str] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return refs
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        plan = round_obj.get("plan_result")
        steps = plan.get("steps") if isinstance(plan, dict) else None
        if not isinstance(steps, list):
            continue
        for step in steps:
            if not isinstance(step, dict):
                continue
            value = step.get("action_ref")
            if isinstance(value, str) and value.strip():
                refs.append(value.strip())
    return refs


def collect_verifier_approved(trace: dict[str, Any]) -> list[bool]:
    approvals: list[bool] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return approvals
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        verify = round_obj.get("verify_result")
        if isinstance(verify, dict) and isinstance(verify.get("approved"), bool):
            approvals.append(bool(verify["approved"]))
    return approvals


def collect_verifier_issue_kinds(trace: dict[str, Any]) -> list[str]:
    kinds: list[str] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return kinds
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        verify = round_obj.get("verify_result")
        issues = verify.get("issues") if isinstance(verify, dict) else None
        if not isinstance(issues, list):
            continue
        for issue in issues:
            if not isinstance(issue, dict):
                continue
            kind = issue.get("kind")
            if isinstance(kind, str) and kind.strip():
                kinds.append(kind.strip())
    return kinds


def collect_verifier_issue_attributions(trace: dict[str, Any]) -> list[str]:
    values: list[str] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return values
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        verify = round_obj.get("verify_result")
        issues = verify.get("issues") if isinstance(verify, dict) else None
        if not isinstance(issues, list):
            continue
        for issue in issues:
            if not isinstance(issue, dict):
                continue
            value = issue.get("failure_attribution")
            if isinstance(value, str) and value.strip():
                values.append(value.strip())
    return values


def collect_verifier_needs_confirmation(trace: dict[str, Any]) -> list[bool]:
    values: list[bool] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return values
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        verify = round_obj.get("verify_result")
        if isinstance(verify, dict) and isinstance(verify.get("needs_confirmation"), bool):
            values.append(bool(verify["needs_confirmation"]))
    return values


def collect_executed(trace: dict[str, Any]) -> list[str]:
    executed: list[str] = []
    steps = trace.get("step_results")
    if not isinstance(steps, list):
        return executed
    for step in steps:
        if not isinstance(step, dict):
            continue
        for key in ("requested_capability", "executed_skill", "skill", "requested_action_type"):
            value = step.get(key)
            if isinstance(value, str) and value.strip():
                executed.append(value.strip())
    return executed


def collect_requested_action_refs(trace: dict[str, Any]) -> list[str]:
    refs: list[str] = []
    steps = trace.get("step_results")
    if not isinstance(steps, list):
        return refs
    for step in steps:
        if not isinstance(step, dict):
            continue
        value = step.get("requested_action_ref")
        if isinstance(value, str) and value.strip():
            refs.append(value.strip())
    return refs


def collect_step_string_field(trace: dict[str, Any], field: str) -> list[str]:
    values: list[str] = []
    steps = trace.get("step_results")
    if not isinstance(steps, list):
        return values
    for step in steps:
        if not isinstance(step, dict):
            continue
        value = step.get(field)
        if isinstance(value, str) and value.strip():
            values.append(value.strip())
    return values


def collect_task_observation_string_field(trace: dict[str, Any], field: str) -> list[str]:
    values: list[str] = []
    observations = trace.get("task_observations")
    if not isinstance(observations, list):
        return values
    for observation in observations:
        if not isinstance(observation, dict):
            continue
        value = observation.get(field)
        if isinstance(value, str) and value.strip():
            values.append(value.strip())
    return values


def collect_contract_policy_decisions(trace: dict[str, Any]) -> list[str]:
    values: list[str] = []
    steps = trace.get("step_results")
    if not isinstance(steps, list):
        return values
    for step in steps:
        if not isinstance(step, dict):
            continue
        policy = step.get("contract_policy")
        if not isinstance(policy, dict):
            continue
        value = policy.get("decision")
        if isinstance(value, str) and value.strip():
            values.append(value.strip())
    return values


def collect_event_types(trace: dict[str, Any]) -> list[str]:
    values: list[str] = []
    events = trace.get("event_stream")
    if not isinstance(events, list):
        return values
    for event in events:
        if not isinstance(event, dict):
            continue
        event_type = event.get("event_type")
        if isinstance(event_type, str) and event_type.strip():
            values.append(event_type.strip())
    return values


def collect_event_field_tokens(trace: dict[str, Any]) -> list[str]:
    values: list[str] = []
    events = trace.get("event_stream")
    if not isinstance(events, list):
        return values
    for event in events:
        if not isinstance(event, dict):
            continue
        event_type = event.get("event_type")
        if isinstance(event_type, str) and event_type.strip():
            values.append(f"event_type={event_type.strip()}")
        payload = event.get("payload")
        if isinstance(payload, dict):
            collect_event_payload_tokens(payload, values)
    return values


def collect_event_payload_tokens(payload: dict[str, Any], out: list[str]) -> None:
    for key, value in payload.items():
        key = str(key).strip()
        if not key:
            continue
        if isinstance(value, (str, int, float, bool)):
            text = str(value).strip()
            if text:
                out.append(f"{key}={text}")
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, (str, int, float, bool)):
                    text = str(item).strip()
                    if text:
                        out.append(f"{key}={text}")


def list_strings(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [str(item) for item in value if isinstance(item, (str, int, float, bool)) and str(item)]


@dataclass
class Observation:
    case: int
    turn: int
    file: str
    prompt: str
    status: str
    stop_signal: str
    stop_failure_attribution: str
    first_layer: str
    routed_mode: str
    route_gate: str
    plan_targets: list[str]
    plan_action_refs: list[str]
    requested_action_refs: list[str]
    executed: list[str]
    error_kinds: list[str]
    failure_attributions: list[str]
    contract_policy_decisions: list[str]
    event_types: list[str]
    event_field_tokens: list[str]
    verifier_approvals: list[bool]
    verifier_issue_kinds: list[str]
    verifier_issue_attributions: list[str]
    verifier_needs_confirmation: list[bool]
    contract_match: str
    contract_semantic_kind: str
    contract_final_answer_shape: str
    required_evidence: list[str]
    observed_evidence_fields: list[str]
    observed_evidence_canonical: list[str]
    missing_evidence: list[str]
    finalizer_stage: str
    finalizer_fallback: str
    finalizer_final_answer_shape: str
    finalizer_final_answer_shape_class: str
    finalizer_coarse_response_shape: str
    finalizer_allows_model_language: bool | None
    finalizer_grounded_ok: bool | None
    finalizer_used_evidence_ids_count: int | None
    final_text: str
    final_shape: str


def observe_file(path: Path) -> Observation:
    match = CASE_RE.search(path.name)
    case_id = int(match.group("case")) if match else -1
    turn_id = int(match.group("turn")) if match else -1
    obj = json.loads(path.read_text(encoding="utf-8"))
    result = get_path(obj, "data", "result_json") or {}
    journal = result.get("task_journal") if isinstance(result, dict) else {}
    summary = journal.get("summary") if isinstance(journal, dict) else {}
    trace = journal.get("trace") if isinstance(journal, dict) else {}
    route = summary.get("route_result") if isinstance(summary, dict) else {}
    finalizer = summary.get("finalizer_summary") if isinstance(summary, dict) else {}
    contract_matrix = trace.get("contract_matrix") if isinstance(trace, dict) else {}
    evidence_coverage = trace.get("evidence_coverage") if isinstance(trace, dict) else {}
    text = final_text(result if isinstance(result, dict) else {}, summary if isinstance(summary, dict) else {})
    return Observation(
        case=case_id,
        turn=turn_id,
        file=path.name,
        prompt=compact(summary.get("input_text") if isinstance(summary, dict) else "", 1000),
        status=str(get_path(obj, "data", "status") or ""),
        stop_signal=(
            str(summary.get("final_stop_signal") or trace.get("final_stop_signal") or "")
            if isinstance(summary, dict) and isinstance(trace, dict)
            else ""
        ),
        stop_failure_attribution=(
            str(summary.get("final_failure_attribution") or trace.get("final_failure_attribution") or "")
            if isinstance(summary, dict) and isinstance(trace, dict)
            else ""
        ),
        first_layer=route_legacy_first_layer(route),
        routed_mode=str(route.get("routed_mode") or "") if isinstance(route, dict) else "",
        route_gate=str(route.get("route_gate_kind") or "") if isinstance(route, dict) else "",
        plan_targets=collect_plan_targets(trace if isinstance(trace, dict) else {}),
        plan_action_refs=collect_plan_action_refs(trace if isinstance(trace, dict) else {}),
        requested_action_refs=collect_requested_action_refs(trace if isinstance(trace, dict) else {}),
        executed=collect_executed(trace if isinstance(trace, dict) else {}),
        error_kinds=(
            collect_step_string_field(trace if isinstance(trace, dict) else {}, "error_kind")
            + collect_task_observation_string_field(
                trace if isinstance(trace, dict) else {}, "error_kind"
            )
        ),
        failure_attributions=(
            collect_step_string_field(trace if isinstance(trace, dict) else {}, "failure_attribution")
            + collect_task_observation_string_field(
                trace if isinstance(trace, dict) else {}, "failure_attribution"
            )
        ),
        contract_policy_decisions=collect_contract_policy_decisions(trace if isinstance(trace, dict) else {}),
        event_types=collect_event_types(trace if isinstance(trace, dict) else {}),
        event_field_tokens=collect_event_field_tokens(trace if isinstance(trace, dict) else {}),
        verifier_approvals=collect_verifier_approved(trace if isinstance(trace, dict) else {}),
        verifier_issue_kinds=collect_verifier_issue_kinds(trace if isinstance(trace, dict) else {}),
        verifier_issue_attributions=collect_verifier_issue_attributions(trace if isinstance(trace, dict) else {}),
        verifier_needs_confirmation=collect_verifier_needs_confirmation(trace if isinstance(trace, dict) else {}),
        contract_match=(
            str(contract_matrix.get("contract_match") or "") if isinstance(contract_matrix, dict) else ""
        ),
        contract_semantic_kind=(
            str(contract_matrix.get("semantic_kind") or "") if isinstance(contract_matrix, dict) else ""
        ),
        contract_final_answer_shape=(
            str(contract_matrix.get("final_answer_shape") or "") if isinstance(contract_matrix, dict) else ""
        ),
        required_evidence=(
            list_strings(evidence_coverage.get("required_evidence"))
            if isinstance(evidence_coverage, dict)
            else []
        ),
        observed_evidence_fields=(
            list_strings(evidence_coverage.get("observed_fields"))
            if isinstance(evidence_coverage, dict)
            else []
        ),
        observed_evidence_canonical=(
            list_strings(evidence_coverage.get("observed_canonical"))
            if isinstance(evidence_coverage, dict)
            else []
        ),
        missing_evidence=(
            list_strings(evidence_coverage.get("missing_evidence"))
            if isinstance(evidence_coverage, dict)
            else []
        ),
        finalizer_stage=str(finalizer.get("stage") or "") if isinstance(finalizer, dict) else "",
        finalizer_fallback=str(finalizer.get("fallback") or "") if isinstance(finalizer, dict) else "",
        finalizer_final_answer_shape=(
            str(finalizer.get("final_answer_shape") or "") if isinstance(finalizer, dict) else ""
        ),
        finalizer_final_answer_shape_class=(
            str(finalizer.get("final_answer_shape_class") or "") if isinstance(finalizer, dict) else ""
        ),
        finalizer_coarse_response_shape=(
            str(finalizer.get("coarse_response_shape") or "") if isinstance(finalizer, dict) else ""
        ),
        finalizer_allows_model_language=(
            finalizer.get("allows_model_language")
            if isinstance(finalizer, dict) and isinstance(finalizer.get("allows_model_language"), bool)
            else None
        ),
        finalizer_grounded_ok=(
            finalizer.get("grounded_ok") if isinstance(finalizer, dict) and isinstance(finalizer.get("grounded_ok"), bool) else None
        ),
        finalizer_used_evidence_ids_count=(
            finalizer.get("used_evidence_ids_count")
            if isinstance(finalizer, dict) and isinstance(finalizer.get("used_evidence_ids_count"), int)
            else None
        ),
        final_text=text,
        final_shape=final_shape(text),
    )


def load_expectations(path: Path) -> dict[int, dict[str, Any]]:
    rows: dict[int, dict[str, Any]] = {}
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        row = json.loads(line)
        case_id = row.get("case")
        if not isinstance(case_id, int):
            raise ValueError(f"{path}:{lineno}: expectation row must contain integer case")
        rows[case_id] = row
    return rows


def any_expected_present(expected: Any, observed: list[str]) -> bool:
    values = expected if isinstance(expected, list) else [expected]
    values = [str(value) for value in values if str(value).strip()]
    return not values or any(value in observed for value in values)


def all_expected_present(expected: Any, observed: list[str]) -> bool:
    values = expected if isinstance(expected, list) else [expected]
    values = [str(value) for value in values if str(value).strip()]
    return all(value in observed for value in values)


def scalar_expected_matches(expected: Any, observed: str) -> bool:
    values = expected if isinstance(expected, list) else [expected]
    values = [str(value) for value in values if str(value).strip()]
    return not values or str(observed) in values


def evaluate(obs: Observation, expected: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    checks = {
        "status": obs.status,
        "stop_signal": obs.stop_signal,
        "stop_failure_attribution": obs.stop_failure_attribution,
        "first_layer": obs.first_layer,
        "routed_mode": obs.routed_mode,
        "route_gate": obs.route_gate,
        "final_shape": obs.final_shape,
        "finalizer_stage": obs.finalizer_stage,
        "finalizer_fallback": obs.finalizer_fallback,
        "finalizer_final_answer_shape": obs.finalizer_final_answer_shape,
        "finalizer_final_answer_shape_class": obs.finalizer_final_answer_shape_class,
        "finalizer_coarse_response_shape": obs.finalizer_coarse_response_shape,
    }
    for key, observed in checks.items():
        if key in expected and str(expected[key]) != str(observed):
            failures.append(f"{key}: expected {expected[key]!r}, got {observed!r}")
        any_key = f"{key}_any"
        if any_key in expected and not scalar_expected_matches(expected[any_key], observed):
            failures.append(f"{any_key}: expected one of {expected[any_key]!r}, got {observed!r}")
    if "capability_any" in expected and not any_expected_present(
        expected["capability_any"], obs.plan_targets
    ):
        failures.append(
            f"capability_any: expected one of {expected['capability_any']!r}, got {obs.plan_targets!r}"
        )
    if "planned_action_any" in expected and not any_expected_present(
        expected["planned_action_any"], obs.plan_action_refs
    ):
        failures.append(
            f"planned_action_any: expected one of {expected['planned_action_any']!r}, got {obs.plan_action_refs!r}"
        )
    if "planned_action_all" in expected and not all_expected_present(
        expected["planned_action_all"], obs.plan_action_refs
    ):
        failures.append(
            f"planned_action_all: expected all {expected['planned_action_all']!r}, got {obs.plan_action_refs!r}"
        )
    if "planned_action_none_of" in expected:
        forbidden = expected["planned_action_none_of"]
        forbidden = forbidden if isinstance(forbidden, list) else [forbidden]
        forbidden = [str(value) for value in forbidden if str(value).strip()]
        matched = [value for value in forbidden if value in obs.plan_action_refs]
        if matched:
            failures.append(
                f"planned_action_none_of: forbidden planned action(s) {matched!r}, got {obs.plan_action_refs!r}"
            )
    if "requested_action_any" in expected and not any_expected_present(
        expected["requested_action_any"], obs.requested_action_refs
    ):
        failures.append(
            f"requested_action_any: expected one of {expected['requested_action_any']!r}, got {obs.requested_action_refs!r}"
        )
    if "requested_action_none_of" in expected:
        forbidden = expected["requested_action_none_of"]
        forbidden = forbidden if isinstance(forbidden, list) else [forbidden]
        forbidden = [str(value) for value in forbidden if str(value).strip()]
        matched = [value for value in forbidden if value in obs.requested_action_refs]
        if matched:
            failures.append(
                f"requested_action_none_of: forbidden requested action(s) {matched!r}, got {obs.requested_action_refs!r}"
            )
    if "executed_any" in expected and not any_expected_present(expected["executed_any"], obs.executed):
        failures.append(f"executed_any: expected one of {expected['executed_any']!r}, got {obs.executed!r}")
    if "executed_none_of" in expected:
        forbidden = expected["executed_none_of"]
        forbidden = forbidden if isinstance(forbidden, list) else [forbidden]
        forbidden = [str(value) for value in forbidden if str(value).strip()]
        matched = [value for value in forbidden if value in obs.executed]
        if matched:
            failures.append(f"executed_none_of: forbidden executed value(s) {matched!r}, got {obs.executed!r}")
    if "error_kind_any" in expected and not any_expected_present(expected["error_kind_any"], obs.error_kinds):
        failures.append(f"error_kind_any: expected one of {expected['error_kind_any']!r}, got {obs.error_kinds!r}")
    if "error_kind_all" in expected and not all_expected_present(expected["error_kind_all"], obs.error_kinds):
        failures.append(f"error_kind_all: expected all {expected['error_kind_all']!r}, got {obs.error_kinds!r}")
    if "error_kind_none_of" in expected:
        forbidden = expected["error_kind_none_of"]
        forbidden = forbidden if isinstance(forbidden, list) else [forbidden]
        forbidden = [str(value) for value in forbidden if str(value).strip()]
        matched = [value for value in forbidden if value in obs.error_kinds]
        if matched:
            failures.append(f"error_kind_none_of: forbidden error kind(s) {matched!r}, got {obs.error_kinds!r}")
    if "failure_attribution_any" in expected and not any_expected_present(
        expected["failure_attribution_any"], obs.failure_attributions
    ):
        failures.append(
            f"failure_attribution_any: expected one of {expected['failure_attribution_any']!r}, got {obs.failure_attributions!r}"
        )
    if "failure_attribution_all" in expected and not all_expected_present(
        expected["failure_attribution_all"], obs.failure_attributions
    ):
        failures.append(
            f"failure_attribution_all: expected all {expected['failure_attribution_all']!r}, got {obs.failure_attributions!r}"
        )
    if "failure_attribution_none_of" in expected:
        forbidden = expected["failure_attribution_none_of"]
        forbidden = forbidden if isinstance(forbidden, list) else [forbidden]
        forbidden = [str(value) for value in forbidden if str(value).strip()]
        matched = [value for value in forbidden if value in obs.failure_attributions]
        if matched:
            failures.append(
                f"failure_attribution_none_of: forbidden attribution(s) {matched!r}, got {obs.failure_attributions!r}"
            )
    if "contract_policy_decision_any" in expected and not any_expected_present(
        expected["contract_policy_decision_any"], obs.contract_policy_decisions
    ):
        failures.append(
            f"contract_policy_decision_any: expected one of {expected['contract_policy_decision_any']!r}, got {obs.contract_policy_decisions!r}"
        )
    if "contract_policy_decision_all" in expected and not all_expected_present(
        expected["contract_policy_decision_all"], obs.contract_policy_decisions
    ):
        failures.append(
            f"contract_policy_decision_all: expected all {expected['contract_policy_decision_all']!r}, got {obs.contract_policy_decisions!r}"
        )
    if "event_type_any" in expected and not any_expected_present(expected["event_type_any"], obs.event_types):
        failures.append(
            f"event_type_any: expected one of {expected['event_type_any']!r}, got {obs.event_types!r}"
        )
    if "event_type_all" in expected and not all_expected_present(expected["event_type_all"], obs.event_types):
        failures.append(
            f"event_type_all: expected all {expected['event_type_all']!r}, got {obs.event_types!r}"
        )
    if "event_field_any" in expected and not any_expected_present(
        expected["event_field_any"], obs.event_field_tokens
    ):
        failures.append(
            f"event_field_any: expected one of {expected['event_field_any']!r}, got {obs.event_field_tokens!r}"
        )
    if "event_field_all" in expected and not all_expected_present(
        expected["event_field_all"], obs.event_field_tokens
    ):
        failures.append(
            f"event_field_all: expected all {expected['event_field_all']!r}, got {obs.event_field_tokens!r}"
        )
    if "verifier_approved" in expected:
        wanted = bool(expected["verifier_approved"])
        if not obs.verifier_approvals or wanted not in obs.verifier_approvals:
            failures.append(
                f"verifier_approved: expected {wanted!r}, got {obs.verifier_approvals!r}"
            )
    if "verifier_issue_any" in expected and not any_expected_present(
        expected["verifier_issue_any"], obs.verifier_issue_kinds
    ):
        failures.append(
            f"verifier_issue_any: expected one of {expected['verifier_issue_any']!r}, got {obs.verifier_issue_kinds!r}"
        )
    if "verifier_issue_all" in expected and not all_expected_present(
        expected["verifier_issue_all"], obs.verifier_issue_kinds
    ):
        failures.append(
            f"verifier_issue_all: expected all {expected['verifier_issue_all']!r}, got {obs.verifier_issue_kinds!r}"
        )
    if "verifier_failure_attribution_any" in expected and not any_expected_present(
        expected["verifier_failure_attribution_any"], obs.verifier_issue_attributions
    ):
        failures.append(
            f"verifier_failure_attribution_any: expected one of {expected['verifier_failure_attribution_any']!r}, got {obs.verifier_issue_attributions!r}"
        )
    if "verifier_failure_attribution_all" in expected and not all_expected_present(
        expected["verifier_failure_attribution_all"], obs.verifier_issue_attributions
    ):
        failures.append(
            f"verifier_failure_attribution_all: expected all {expected['verifier_failure_attribution_all']!r}, got {obs.verifier_issue_attributions!r}"
        )
    if "needs_confirmation" in expected:
        wanted = bool(expected["needs_confirmation"])
        if not obs.verifier_needs_confirmation or wanted not in obs.verifier_needs_confirmation:
            failures.append(
                f"needs_confirmation: expected {wanted!r}, got {obs.verifier_needs_confirmation!r}"
            )
    if "finalizer_grounded_ok" in expected:
        wanted = bool(expected["finalizer_grounded_ok"])
        if obs.finalizer_grounded_ok is not wanted:
            failures.append(
                f"finalizer_grounded_ok: expected {wanted!r}, got {obs.finalizer_grounded_ok!r}"
            )
    if "finalizer_allows_model_language" in expected:
        wanted = bool(expected["finalizer_allows_model_language"])
        if obs.finalizer_allows_model_language is not wanted:
            failures.append(
                f"finalizer_allows_model_language: expected {wanted!r}, got {obs.finalizer_allows_model_language!r}"
            )
    if "finalizer_used_evidence_ids_min" in expected:
        wanted = int(expected["finalizer_used_evidence_ids_min"])
        observed = obs.finalizer_used_evidence_ids_count
        if observed is None or observed < wanted:
            failures.append(
                f"finalizer_used_evidence_ids_min: expected >= {wanted}, got {observed!r}"
            )
    contract_checks = {
        "contract_match": obs.contract_match,
        "contract_semantic_kind": obs.contract_semantic_kind,
        "contract_final_answer_shape": obs.contract_final_answer_shape,
    }
    for key, observed in contract_checks.items():
        if key in expected and str(expected[key]) != str(observed):
            failures.append(f"{key}: expected {expected[key]!r}, got {observed!r}")
        any_key = f"{key}_any"
        if any_key in expected and not scalar_expected_matches(expected[any_key], observed):
            failures.append(f"{any_key}: expected one of {expected[any_key]!r}, got {observed!r}")
    if "required_evidence_all" in expected and not all_expected_present(
        expected["required_evidence_all"], obs.required_evidence
    ):
        failures.append(
            f"required_evidence_all: expected all {expected['required_evidence_all']!r}, got {obs.required_evidence!r}"
        )
    observed_evidence = sorted(set(obs.observed_evidence_fields) | set(obs.observed_evidence_canonical))
    if "observed_evidence_any" in expected and not any_expected_present(
        expected["observed_evidence_any"], observed_evidence
    ):
        failures.append(
            f"observed_evidence_any: expected one of {expected['observed_evidence_any']!r}, got {observed_evidence!r}"
        )
    if "observed_evidence_all" in expected and not all_expected_present(
        expected["observed_evidence_all"], observed_evidence
    ):
        failures.append(
            f"observed_evidence_all: expected all {expected['observed_evidence_all']!r}, got {observed_evidence!r}"
        )
    if bool(expected.get("missing_evidence_empty")) and obs.missing_evidence:
        failures.append(f"missing_evidence_empty: expected [], got {obs.missing_evidence!r}")
    if "final_contains" in expected:
        values = expected["final_contains"]
        values = values if isinstance(values, list) else [values]
        for value in values:
            if str(value) not in obs.final_text:
                failures.append(f"final_contains: missing {value!r}")
    if "final_not_contains" in expected:
        values = expected["final_not_contains"]
        values = values if isinstance(values, list) else [values]
        for value in values:
            if str(value) in obs.final_text:
                failures.append(f"final_not_contains: forbidden {value!r}")
    return failures


def baseline_row(obs: Observation) -> dict[str, Any]:
    return {
        "case": obs.case,
        "turn": obs.turn,
        "prompt": obs.prompt,
        "status": obs.status,
        "stop_signal": obs.stop_signal,
        "stop_failure_attribution": obs.stop_failure_attribution,
        "first_layer": obs.first_layer,
        "routed_mode": obs.routed_mode,
        "route_gate": obs.route_gate,
        "plan_targets": obs.plan_targets,
        "plan_action_refs": obs.plan_action_refs,
        "requested_action_refs": obs.requested_action_refs,
        "executed": obs.executed,
        "error_kinds": obs.error_kinds,
        "failure_attributions": obs.failure_attributions,
        "contract_policy_decisions": obs.contract_policy_decisions,
        "event_types": obs.event_types,
        "event_field_tokens": obs.event_field_tokens,
        "verifier_approvals": obs.verifier_approvals,
        "verifier_issue_kinds": obs.verifier_issue_kinds,
        "verifier_issue_attributions": obs.verifier_issue_attributions,
        "verifier_needs_confirmation": obs.verifier_needs_confirmation,
        "contract_match": obs.contract_match,
        "contract_semantic_kind": obs.contract_semantic_kind,
        "contract_final_answer_shape": obs.contract_final_answer_shape,
        "required_evidence": obs.required_evidence,
        "observed_evidence_fields": obs.observed_evidence_fields,
        "observed_evidence_canonical": obs.observed_evidence_canonical,
        "missing_evidence": obs.missing_evidence,
        "finalizer_stage": obs.finalizer_stage,
        "finalizer_fallback": obs.finalizer_fallback,
        "finalizer_final_answer_shape": obs.finalizer_final_answer_shape,
        "finalizer_final_answer_shape_class": obs.finalizer_final_answer_shape_class,
        "finalizer_coarse_response_shape": obs.finalizer_coarse_response_shape,
        "finalizer_allows_model_language": obs.finalizer_allows_model_language,
        "finalizer_grounded_ok": obs.finalizer_grounded_ok,
        "finalizer_used_evidence_ids_count": obs.finalizer_used_evidence_ids_count,
        "final_shape": obs.final_shape,
        "final_preview": compact(obs.final_text, 500),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path, help="client-like continuous run log directory")
    parser.add_argument("--expectations", type=Path, help="JSONL expectation file")
    parser.add_argument("--write-baseline", type=Path, help="write observed baseline JSONL")
    args = parser.parse_args()

    if not args.run_dir.is_dir():
        raise SystemExit(f"Not a directory: {args.run_dir}")
    files = sorted(args.run_dir.glob("turn_*_case_*.json"), key=sort_key)
    if not files:
        raise SystemExit(f"No turn_*_case_*.json files found under {args.run_dir}")

    observations = [observe_file(path) for path in files]

    if args.write_baseline:
        args.write_baseline.parent.mkdir(parents=True, exist_ok=True)
        args.write_baseline.write_text(
            "".join(
                json.dumps(baseline_row(obs), ensure_ascii=False, sort_keys=True) + "\n"
                for obs in observations
            ),
            encoding="utf-8",
        )

    expectations = load_expectations(args.expectations) if args.expectations else {}
    failures: list[dict[str, Any]] = []
    checked = 0
    for obs in observations:
        expected = expectations.get(obs.case)
        if not expected:
            continue
        checked += 1
        case_failures = evaluate(obs, expected)
        if case_failures:
            failures.append(
                {
                    "case": obs.case,
                    "file": obs.file,
                    "failures": case_failures,
                    "observed": baseline_row(obs),
                }
            )

    if failures:
        for failure in failures:
            print(json.dumps(failure, ensure_ascii=False, sort_keys=True))
        print(f"CLIENT_LIKE_EVAL_FAIL failed={len(failures)} total={len(observations)}")
        return 1

    suffix = f" baseline={args.write_baseline}" if args.write_baseline else ""
    expectations_total = len(expectations)
    print(
        f"CLIENT_LIKE_EVAL_OK total={len(observations)} checked={checked} "
        f"expectations={expectations_total}{suffix}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
