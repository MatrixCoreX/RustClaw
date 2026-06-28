#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any


PRIMARY_SWITCH_NAME = "semantic_route_authority"
LEGACY_SWITCH_NAME = "agent_decides_semantic_route"
ROUTE_DELTA_SWITCH_NAMES = {PRIMARY_SWITCH_NAME, LEGACY_SWITCH_NAME}
CASE_FILE_RE = re.compile(r"^turn_(?P<turn>\d+)_case_(?P<case>\d+)\.json$")
FINAL_CASE_DIR_RE = re.compile(r"^case_(?P<case>\d+)_")
PRE_PLANNER_PROMPTS = {
    "normalizer",
    "contract_repair",
    "direct_answer_gate",
    "router_legacy",
    "delivery_classifier",
    "direct_classifier",
    "intent_meta",
    "schedule",
    "nl2cmd",
    "semantic_judge",
}
PLANNER_LOOP_PROMPTS = {
    "plan",
    "plan_repair",
}
POST_PLANNER_PROMPTS = {
    "verifier",
    "observed",
    "user_response_composer",
    "user_response_validator",
    "clarify",
    "chat",
    "memory",
    "self_extension",
}
COUNTER_FIELDS = (
    "status_counts",
    "final_status_counts",
    "final_stop_signal_counts",
    "route_gate_counts",
    "clarification_counts",
    "verifier_pass_counts",
    "contract_match_counts",
    "contract_final_answer_shape_counts",
    "finalizer_answer_shape_counts",
    "finalizer_answer_shape_class_counts",
    "event_counts",
    "switch_name_counts",
    "outcome_counts",
    "decision_delta_counts",
    "old_first_layer_decision_counts",
    "agent_decision_counts",
    "decision_envelope_decision_counts",
    "decision_envelope_validation_status_counts",
    "decision_envelope_validation_reason_counts",
    "round_decision_envelope_source_counts",
    "round_decision_envelope_authority_counts",
    "round_decision_envelope_decision_counts",
    "round_decision_envelope_capability_counts",
    "round_decision_envelope_validation_status_counts",
    "round_decision_envelope_validation_reason_counts",
    "capability_delta_counts",
    "risk_delta_counts",
    "output_contract_delta_counts",
    "evidence_delta_counts",
    "budget_profile_counts",
    "configured_migration_class_counts",
    "eligible_migration_class_counts",
    "selected_migration_class_counts",
    "semantic_routing_activation_state_counts",
    "semantic_routing_authority_counts",
    "semantic_routing_runtime_default_authority_counts",
    "semantic_routing_normalizer_role_counts",
    "semantic_routing_post_route_role_counts",
    "semantic_routing_direct_answer_gate_role_counts",
    "pre_agent_intent_authority_counts",
    "pre_agent_intent_ownership_class_counts",
    "pre_agent_intent_boundary_allowed_counts",
    "pre_agent_intent_semantic_migration_target_counts",
    "pre_agent_post_route_boundary_class_counts",
    "pre_agent_post_route_ownership_class_counts",
    "pre_agent_post_route_boundary_allowed_counts",
    "pre_agent_post_route_semantic_migration_target_counts",
    "pre_agent_direct_answer_observation_class_counts",
    "pre_agent_direct_answer_boundary_class_counts",
    "pre_agent_direct_answer_ownership_class_counts",
    "pre_agent_direct_answer_boundary_allowed_counts",
    "pre_agent_direct_answer_semantic_migration_target_counts",
    "runtime_decision_source_counts",
    "runtime_semantic_control_state_counts",
    "runtime_rewrite_reason_counts",
    "reason_code_counts",
    "mismatch_explanation_counts",
)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as err:
        return {"_parse_error": str(err)}
    return value if isinstance(value, dict) else {"_parse_error": "top_level_not_object"}


def dict_path(obj: dict[str, Any], *keys: str) -> Any:
    cur: Any = obj
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def counter_json(counter: Counter[str]) -> dict[str, int]:
    return {key: counter[key] for key in sorted(counter)}


def merge_counter_dicts(summaries: list[dict[str, Any]], key: str) -> dict[str, int]:
    merged: Counter[str] = Counter()
    for summary in summaries:
        value = summary.get(key)
        if not isinstance(value, dict):
            continue
        for item_key, item_value in value.items():
            merged[str(item_key)] += safe_int(item_value)
    return counter_json(merged)


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def sorted_str_list(value: Any) -> list[str]:
    return sorted(str(item) for item in list_value(value))


def safe_int(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def ratio(numerator: int, denominator: int) -> float:
    return round(numerator / denominator, 4) if denominator else 0.0


def turn_json_paths(run_dir: Path) -> list[Path]:
    return sorted(
        {
            *run_dir.glob("turn*_case_*.json"),
            *run_dir.glob("turn_*.json"),
            *run_dir.glob("case_*/final.json"),
        }
    )


def case_turn_json_paths(run_dir: Path) -> list[Path]:
    return sorted({*run_dir.glob("turn*_case_*.json"), *run_dir.glob("case_*/final.json")})


def turn_case_id(path: Path) -> int | None:
    match = CASE_FILE_RE.match(path.name)
    if match:
        return int(match.group("case"))
    if path.name == "final.json":
        match = FINAL_CASE_DIR_RE.match(path.parent.name)
        if match:
            return int(match.group("case"))
    return None


def turn_file_order(path: Path, run_order: dict[Path, int]) -> tuple[int, str, int, str]:
    match = CASE_FILE_RE.match(path.name)
    if match:
        turn_number = int(match.group("turn"))
        parent = path.parent.resolve()
    elif path.name == "final.json":
        turn_number = turn_case_id(path) or 0
        parent = path.parent.parent.resolve()
    else:
        turn_number = 0
        parent = path.parent.resolve()
    return (
        run_order.get(parent, -1),
        path.parent.name if path.name == "final.json" else parent.name,
        turn_number,
        path.name,
    )


def latest_valid_case_paths(run_dirs: list[Path]) -> tuple[list[Path], dict[str, Any]]:
    run_order = {run_dir.resolve(): index for index, run_dir in enumerate(run_dirs)}
    latest: dict[int, tuple[tuple[int, str, int, str], Path]] = {}
    skipped_parse_errors: list[dict[str, str]] = []
    ignored_without_case_id = 0
    for run_dir in run_dirs:
        for path in case_turn_json_paths(run_dir):
            case_id = turn_case_id(path)
            if case_id is None:
                ignored_without_case_id += 1
                continue
            obj = load_json(path)
            if obj.get("_parse_error"):
                skipped_parse_errors.append(
                    {
                        "run_dir": str(run_dir),
                        "file": path.name,
                        "error": str(obj["_parse_error"]),
                    }
                )
                continue
            order = turn_file_order(path, run_order)
            current = latest.get(case_id)
            if current is None or order > current[0]:
                latest[case_id] = (order, path)
    selected = [latest[case_id][1] for case_id in sorted(latest)]
    selected_run_counts: Counter[str] = Counter(path.parent.name for path in selected)
    case_ids = sorted(latest)
    missing_case_ids: list[int] = []
    if case_ids:
        missing_case_ids = [
            case_id for case_id in range(case_ids[0], case_ids[-1] + 1)
            if case_id not in latest
        ]
    return selected, {
        "mode": "latest_valid_case_id",
        "case_count": len(case_ids),
        "min_case_id": case_ids[0] if case_ids else None,
        "max_case_id": case_ids[-1] if case_ids else None,
        "missing_case_ids": missing_case_ids,
        "selected_run_dir_counts": counter_json(selected_run_counts),
        "skipped_parse_error_count": len(skipped_parse_errors),
        "skipped_parse_error_examples": skipped_parse_errors[:10],
        "ignored_without_case_id": ignored_without_case_id,
    }


def rollout_items(turn_obj: dict[str, Any]) -> list[dict[str, Any]]:
    summary_items = dict_path(
        turn_obj,
        "data",
        "result_json",
        "task_journal",
        "summary",
        "rollout_attribution",
    )
    trace_items = dict_path(
        turn_obj,
        "data",
        "result_json",
        "task_journal",
        "trace",
        "rollout_attribution",
    )
    items: list[dict[str, Any]] = []
    seen: set[str] = set()
    for source in (summary_items, trace_items):
        for item in list_value(source):
            if not isinstance(item, dict):
                continue
            fingerprint = json.dumps(item, sort_keys=True, ensure_ascii=True)
            if fingerprint in seen:
                continue
            seen.add(fingerprint)
            items.append(item)
    return items


def task_id(turn_obj: dict[str, Any]) -> str:
    value = dict_path(turn_obj, "data", "task_id") or dict_path(
        turn_obj, "data", "result_json", "task_journal", "summary", "task_id"
    )
    return str(value or "")


def route_result(turn_obj: dict[str, Any]) -> dict[str, Any]:
    value = dict_path(
        turn_obj,
        "data",
        "result_json",
        "task_journal",
        "summary",
        "route_result",
    )
    return value if isinstance(value, dict) else {}


def journal_summary(turn_obj: dict[str, Any]) -> dict[str, Any]:
    value = dict_path(turn_obj, "data", "result_json", "task_journal", "summary")
    return value if isinstance(value, dict) else {}


def journal_trace(turn_obj: dict[str, Any]) -> dict[str, Any]:
    value = dict_path(turn_obj, "data", "result_json", "task_journal", "trace")
    return value if isinstance(value, dict) else {}


def contract_snapshot(turn_obj: dict[str, Any]) -> dict[str, Any]:
    trace = journal_trace(turn_obj)
    value = trace.get("contract_matrix")
    if not isinstance(value, dict):
        value = dict_path(trace, "runtime_contract_snapshot", "contract")
    return value if isinstance(value, dict) else {}


def finalizer_summary(turn_obj: dict[str, Any]) -> dict[str, Any]:
    value = journal_summary(turn_obj).get("finalizer_summary")
    return value if isinstance(value, dict) else {}


def task_metrics(turn_obj: dict[str, Any]) -> dict[str, Any]:
    value = journal_summary(turn_obj).get("task_metrics")
    return value if isinstance(value, dict) else {}


def answer_verifier_summary(turn_obj: dict[str, Any]) -> dict[str, Any]:
    value = journal_summary(turn_obj).get("answer_verifier_summary")
    return value if isinstance(value, dict) else {}


def trace_step_results(turn_obj: dict[str, Any]) -> list[dict[str, Any]]:
    value = journal_trace(turn_obj).get("step_results")
    return [item for item in list_value(value) if isinstance(item, dict)]


def trace_rounds(turn_obj: dict[str, Any]) -> list[dict[str, Any]]:
    value = journal_trace(turn_obj).get("rounds")
    return [item for item in list_value(value) if isinstance(item, dict)]


def count_tool_calls(steps: list[dict[str, Any]]) -> int:
    total = 0
    for step in steps:
        skill = str(step.get("executed_skill") or step.get("skill") or "").strip()
        if not skill or skill in {"synthesize_answer", "respond", "think"}:
            continue
        total += 1
    return total


def route_legacy_first_layer(route: Any) -> str:
    if not isinstance(route, dict):
        return ""
    return str(
        route.get("legacy_first_layer_decision")
        or route.get("first_layer_decision")
        or ""
    )


def route_requested_clarification(route: dict[str, Any], summary: dict[str, Any]) -> bool:
    return (
        route.get("needs_clarify") is True
        or route_legacy_first_layer(route) == "clarify"
        or route.get("route_gate_kind") == "clarify"
        or summary.get("final_status") == "clarification_requested"
    )


def evidence_delta(item: dict[str, Any]) -> str:
    old_evidence = set(sorted_str_list(item.get("old_required_evidence")))
    agent_evidence = set(sorted_str_list(item.get("agent_required_evidence")))
    if not old_evidence and not agent_evidence:
        return "not_evaluated"
    if old_evidence == agent_evidence:
        return "same_evidence"
    if old_evidence < agent_evidence:
        return "agent_adds_evidence"
    if agent_evidence < old_evidence:
        return "agent_drops_evidence"
    return "different_evidence"


def mismatch_explanation(item: dict[str, Any]) -> str:
    decision_delta = str(item.get("decision_delta") or "unknown")
    if decision_delta in {"same_gate", "not_evaluated", "not_comparable"}:
        return "not_mismatch"
    decision_envelope = dict_value(item.get("decision_envelope"))
    if not decision_envelope:
        return "legacy_attribution_schema_without_decision_envelope"
    validation_status = str(
        decision_envelope.get("validation_status") or "not_recorded"
    )
    validation_reason = str(
        decision_envelope.get("validation_reason_code") or "not_recorded"
    )
    if validation_status in {"shadow_invalid", "invalid"}:
        return f"planner_decision_rejected:{validation_reason}"
    old_decision = str(item.get("old_first_layer_decision") or "")
    agent_decision = str(
        item.get("agent_decision") or decision_envelope.get("decision") or ""
    )
    capability_delta = str(item.get("capability_delta") or "")
    capability_ref = str(
        item.get("capability_ref") or decision_envelope.get("capability_ref") or ""
    )
    if (
        decision_delta == "different_gate"
        and validation_status == "valid"
        and validation_reason == "agent_loop_decision_shadow_valid"
        and old_decision == "planner_execute"
        and agent_decision == "respond"
        and capability_delta == "no_capability_ref"
        and capability_ref == "respond"
    ):
        return "agent_loop_valid_direct_response_vs_legacy_planner"
    return "unexplained"


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def bool_token(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return "not_recorded"


def pre_agent_gates(boundary_context: dict[str, Any]) -> dict[str, Any]:
    return dict_value(boundary_context.get("pre_agent_gates"))


def semantic_routing(boundary_context: dict[str, Any]) -> dict[str, Any]:
    return dict_value(boundary_context.get("semantic_routing"))


def prompt_phase(label: str) -> str:
    if label in PRE_PLANNER_PROMPTS:
        return "pre_planner"
    if label in PLANNER_LOOP_PROMPTS:
        return "planner_loop"
    if label in POST_PLANNER_PROMPTS:
        return "post_planner"
    return "other"


def by_prompt_metrics(metrics: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        str(label): dict_value(bucket)
        for label, bucket in dict_value(metrics.get("by_prompt")).items()
    }


def compact_item(path: Path, turn_obj: dict[str, Any], item: dict[str, Any]) -> dict[str, Any]:
    route = route_result(turn_obj)
    decision_envelope = dict_value(item.get("decision_envelope"))
    boundary_context = dict_value(item.get("boundary_context"))
    boundary_budget = dict_value(boundary_context.get("budget"))
    routing = semantic_routing(boundary_context)
    gates = pre_agent_gates(boundary_context)
    intent_normalizer = dict_value(gates.get("intent_normalizer"))
    post_route_policy = dict_value(gates.get("post_route_policy"))
    direct_answer_gate = dict_value(gates.get("direct_answer_gate"))
    return {
        "file": path.name,
        "task_id": task_id(turn_obj),
        "event": item.get("event"),
        "outcome": item.get("outcome"),
        "reason_code": item.get("reason_code"),
        "old_first_layer_decision": item.get("old_first_layer_decision")
        or route_legacy_first_layer(route),
        "agent_decision": item.get("agent_decision"),
        "decision_delta": item.get("decision_delta"),
        "route_layer_that_disagreed": item.get("route_layer_that_disagreed"),
        "decision_envelope_decision": decision_envelope.get("decision"),
        "decision_envelope_validation_status": decision_envelope.get("validation_status"),
        "decision_envelope_validation_reason_code": decision_envelope.get(
            "validation_reason_code"
        ),
        "capability_ref": item.get("capability_ref"),
        "capability_delta": item.get("capability_delta"),
        "risk_level": item.get("risk_level") or route.get("risk_ceiling"),
        "risk_delta": item.get("risk_delta"),
        "output_contract_ref": item.get("output_contract_ref"),
        "output_contract_delta": item.get("output_contract_delta"),
        "evidence_delta": evidence_delta(item),
        "old_required_evidence": sorted_str_list(item.get("old_required_evidence")),
        "agent_required_evidence": sorted_str_list(item.get("agent_required_evidence")),
        "missing_slots": sorted_str_list(item.get("missing_slots")),
        "budget_profile": item.get("budget_profile"),
        "configured_migration_class": boundary_budget.get("agent_decides_migration_class"),
        "eligible_migration_class": boundary_budget.get("eligible_migration_class"),
        "selected_migration_class": boundary_budget.get("selected_migration_class"),
        "semantic_routing_activation_state": routing.get("activation_state"),
        "semantic_routing_authority": routing.get("ordinary_semantic_authority"),
        "semantic_routing_runtime_default_authority": routing.get("runtime_default_authority"),
        "semantic_routing_normalizer_role": routing.get("normalizer_role"),
        "semantic_routing_post_route_role": routing.get("post_route_role"),
        "semantic_routing_direct_answer_gate_role": routing.get("direct_answer_gate_role"),
        "pre_agent_intent_authority": intent_normalizer.get("authority_target"),
        "pre_agent_intent_ownership_class": intent_normalizer.get("ownership_class"),
        "pre_agent_intent_boundary_allowed": intent_normalizer.get("boundary_allowed"),
        "pre_agent_intent_semantic_migration_target": intent_normalizer.get(
            "semantic_migration_target"
        ),
        "pre_agent_post_route_boundary_class": post_route_policy.get("boundary_class"),
        "pre_agent_post_route_ownership_class": post_route_policy.get("ownership_class"),
        "pre_agent_post_route_boundary_allowed": post_route_policy.get("boundary_allowed"),
        "pre_agent_post_route_semantic_migration_target": post_route_policy.get(
            "semantic_migration_target"
        ),
        "pre_agent_direct_answer_observation_class": direct_answer_gate.get(
            "observation_class"
        ),
        "pre_agent_direct_answer_boundary_class": direct_answer_gate.get("boundary_class"),
        "pre_agent_direct_answer_ownership_class": direct_answer_gate.get("ownership_class"),
        "pre_agent_direct_answer_boundary_allowed": direct_answer_gate.get("boundary_allowed"),
        "pre_agent_direct_answer_semantic_migration_target": direct_answer_gate.get(
            "semantic_migration_target"
        ),
    }


def summarize_run(
    run_dir: Path,
    max_examples: int,
    paths: list[Path] | None = None,
    source_run_dir: str | None = None,
) -> dict[str, Any]:
    paths = turn_json_paths(run_dir) if paths is None else sorted(paths)
    parse_errors = 0
    parse_error_examples: list[dict[str, Any]] = []
    tasks_with_items: set[str] = set()
    status_counts: Counter[str] = Counter()
    final_status_counts: Counter[str] = Counter()
    final_stop_signal_counts: Counter[str] = Counter()
    route_gate_counts: Counter[str] = Counter()
    clarification_counts: Counter[str] = Counter()
    verifier_pass_counts: Counter[str] = Counter()
    contract_match_counts: Counter[str] = Counter()
    contract_final_shape_counts: Counter[str] = Counter()
    finalizer_shape_counts: Counter[str] = Counter()
    finalizer_shape_class_counts: Counter[str] = Counter()
    event_counts: Counter[str] = Counter()
    outcome_counts: Counter[str] = Counter()
    switch_name_counts: Counter[str] = Counter()
    decision_delta_counts: Counter[str] = Counter()
    old_decision_counts: Counter[str] = Counter()
    agent_decision_counts: Counter[str] = Counter()
    decision_envelope_decision_counts: Counter[str] = Counter()
    decision_envelope_validation_status_counts: Counter[str] = Counter()
    decision_envelope_validation_reason_counts: Counter[str] = Counter()
    round_decision_envelope_source_counts: Counter[str] = Counter()
    round_decision_envelope_authority_counts: Counter[str] = Counter()
    round_decision_envelope_decision_counts: Counter[str] = Counter()
    round_decision_envelope_capability_counts: Counter[str] = Counter()
    round_decision_envelope_validation_status_counts: Counter[str] = Counter()
    round_decision_envelope_validation_reason_counts: Counter[str] = Counter()
    capability_delta_counts: Counter[str] = Counter()
    risk_delta_counts: Counter[str] = Counter()
    output_contract_delta_counts: Counter[str] = Counter()
    evidence_delta_counts: Counter[str] = Counter()
    budget_profile_counts: Counter[str] = Counter()
    configured_migration_class_counts: Counter[str] = Counter()
    eligible_migration_class_counts: Counter[str] = Counter()
    selected_migration_class_counts: Counter[str] = Counter()
    semantic_routing_activation_state_counts: Counter[str] = Counter()
    semantic_routing_authority_counts: Counter[str] = Counter()
    semantic_routing_runtime_default_authority_counts: Counter[str] = Counter()
    semantic_routing_normalizer_role_counts: Counter[str] = Counter()
    semantic_routing_post_route_role_counts: Counter[str] = Counter()
    semantic_routing_direct_answer_gate_role_counts: Counter[str] = Counter()
    pre_agent_intent_authority_counts: Counter[str] = Counter()
    pre_agent_intent_ownership_class_counts: Counter[str] = Counter()
    pre_agent_intent_boundary_allowed_counts: Counter[str] = Counter()
    pre_agent_intent_semantic_migration_target_counts: Counter[str] = Counter()
    pre_agent_post_route_boundary_class_counts: Counter[str] = Counter()
    pre_agent_post_route_ownership_class_counts: Counter[str] = Counter()
    pre_agent_post_route_boundary_allowed_counts: Counter[str] = Counter()
    pre_agent_post_route_semantic_migration_target_counts: Counter[str] = Counter()
    pre_agent_direct_answer_observation_class_counts: Counter[str] = Counter()
    pre_agent_direct_answer_boundary_class_counts: Counter[str] = Counter()
    pre_agent_direct_answer_ownership_class_counts: Counter[str] = Counter()
    pre_agent_direct_answer_boundary_allowed_counts: Counter[str] = Counter()
    pre_agent_direct_answer_semantic_migration_target_counts: Counter[str] = Counter()
    runtime_decision_source_counts: Counter[str] = Counter()
    runtime_semantic_control_state_counts: Counter[str] = Counter()
    runtime_rewrite_reason_counts: Counter[str] = Counter()
    reason_code_counts: Counter[str] = Counter()
    mismatch_explanation_counts: Counter[str] = Counter()
    attribution_items = 0
    mismatch_examples: list[dict[str, Any]] = []
    unexplained_mismatch_examples: list[dict[str, Any]] = []
    total_llm_calls = 0
    total_llm_elapsed_ms = 0
    llm_prompt_call_counts: Counter[str] = Counter()
    llm_prompt_elapsed_ms_counts: Counter[str] = Counter()
    llm_phase_call_counts: Counter[str] = Counter()
    llm_phase_elapsed_ms_counts: Counter[str] = Counter()
    turns_with_pre_planner_llm = 0
    total_rounds = 0
    total_steps = 0
    total_tool_calls = 0

    for path in paths:
        obj = load_json(path)
        if obj.get("_parse_error"):
            parse_errors += 1
            if len(parse_error_examples) < max_examples:
                parse_error_examples.append(
                    {"file": path.name, "error": str(obj.get("_parse_error") or "")}
                )
            continue
        summary = journal_summary(obj)
        route = route_result(obj)
        contract = contract_snapshot(obj)
        finalizer = finalizer_summary(obj)
        metrics = task_metrics(obj)
        verifier = answer_verifier_summary(obj)
        status_counts[str(dict_path(obj, "data", "status") or "unknown")] += 1
        final_status_counts[str(summary.get("final_status") or "unknown")] += 1
        final_stop_signal_counts[str(summary.get("final_stop_signal") or "unknown")] += 1
        route_gate_counts[str(route.get("route_gate_kind") or "unknown")] += 1
        clarification_counts[
            "requested" if route_requested_clarification(route, summary) else "not_requested"
        ] += 1
        if verifier:
            verifier_pass_counts[str(verifier.get("pass"))] += 1
        else:
            verifier_pass_counts["missing"] += 1
        contract_match_counts[str(contract.get("contract_match") or "unknown")] += 1
        contract_final_shape_counts[str(contract.get("final_answer_shape") or "unknown")] += 1
        finalizer_shape_counts[str(finalizer.get("final_answer_shape") or "unknown")] += 1
        finalizer_shape_class_counts[str(finalizer.get("final_answer_shape_class") or "unknown")] += 1
        total_llm_calls += safe_int(metrics.get("llm_calls_per_task"))
        total_llm_elapsed_ms += safe_int(metrics.get("llm_elapsed_ms_per_task"))
        turn_pre_planner_calls = 0
        for label, bucket in by_prompt_metrics(metrics).items():
            count = safe_int(bucket.get("count"))
            elapsed_ms = safe_int(bucket.get("elapsed_ms"))
            phase = prompt_phase(label)
            llm_prompt_call_counts[label] += count
            llm_prompt_elapsed_ms_counts[label] += elapsed_ms
            llm_phase_call_counts[phase] += count
            llm_phase_elapsed_ms_counts[phase] += elapsed_ms
            if phase == "pre_planner":
                turn_pre_planner_calls += count
        if turn_pre_planner_calls:
            turns_with_pre_planner_llm += 1
        total_rounds += safe_int(summary.get("round_count"))
        total_steps += safe_int(summary.get("step_count"))
        total_tool_calls += count_tool_calls(trace_step_results(obj))
        for round_item in trace_rounds(obj):
            round_decision_envelope = dict_value(round_item.get("decision_envelope"))
            round_decision_envelope_source_counts[
                str(round_decision_envelope.get("source") or "not_recorded")
            ] += 1
            round_decision_envelope_authority_counts[
                str(round_decision_envelope.get("semantic_authority") or "not_recorded")
            ] += 1
            round_decision_envelope_decision_counts[
                str(round_decision_envelope.get("decision") or "not_recorded")
            ] += 1
            round_decision_envelope_capability_counts[
                str(round_decision_envelope.get("capability_ref") or "not_recorded")
            ] += 1
            round_decision_envelope_validation_status_counts[
                str(round_decision_envelope.get("validation_status") or "not_recorded")
            ] += 1
            round_decision_envelope_validation_reason_counts[
                str(round_decision_envelope.get("validation_reason_code") or "not_recorded")
            ] += 1
        for item in rollout_items(obj):
            switch_name = str(item.get("switch_name") or "")
            item_boundary_context = dict_value(item.get("boundary_context"))
            runtime_decision_source_counts[
                str(item_boundary_context.get("decision_source") or "not_recorded")
            ] += 1
            runtime_semantic_control_state_counts[
                str(item_boundary_context.get("semantic_control_state") or "not_recorded")
            ] += 1
            runtime_rewrite_reason_counts[
                str(
                    item_boundary_context.get("rewrite_reason_code")
                    or item.get("reason_code")
                    or "not_recorded"
                )
            ] += 1
            if switch_name not in ROUTE_DELTA_SWITCH_NAMES:
                continue
            attribution_items += 1
            switch_name_counts[switch_name] += 1
            task = task_id(obj)
            if task:
                tasks_with_items.add(task)
            event_counts[str(item.get("event") or "unknown")] += 1
            outcome_counts[str(item.get("outcome") or "unknown")] += 1
            decision_delta = str(item.get("decision_delta") or "unknown")
            decision_delta_counts[decision_delta] += 1
            old_decision_counts[str(item.get("old_first_layer_decision") or "unknown")] += 1
            agent_decision_counts[str(item.get("agent_decision") or "unknown")] += 1
            decision_envelope = dict_value(item.get("decision_envelope"))
            decision_envelope_decision_counts[
                str(decision_envelope.get("decision") or "not_recorded")
            ] += 1
            decision_envelope_validation_status_counts[
                str(decision_envelope.get("validation_status") or "not_recorded")
            ] += 1
            decision_envelope_validation_reason_counts[
                str(decision_envelope.get("validation_reason_code") or "not_recorded")
            ] += 1
            capability_delta_counts[str(item.get("capability_delta") or "unknown")] += 1
            risk_delta_counts[str(item.get("risk_delta") or "unknown")] += 1
            output_contract_delta_counts[str(item.get("output_contract_delta") or "unknown")] += 1
            evidence_delta_counts[evidence_delta(item)] += 1
            budget_profile_counts[str(item.get("budget_profile") or "unknown")] += 1
            boundary_context = dict_value(item.get("boundary_context"))
            boundary_budget = dict_value(boundary_context.get("budget"))
            configured_migration_class_counts[
                str(boundary_budget.get("agent_decides_migration_class") or "unknown")
            ] += 1
            eligible_migration_class_counts[
                str(boundary_budget.get("eligible_migration_class") or "unknown")
            ] += 1
            selected_migration_class_counts[
                str(boundary_budget.get("selected_migration_class") or "unknown")
            ] += 1
            routing = semantic_routing(boundary_context)
            semantic_routing_activation_state_counts[
                str(routing.get("activation_state") or "not_recorded")
            ] += 1
            semantic_routing_authority_counts[
                str(routing.get("ordinary_semantic_authority") or "not_recorded")
            ] += 1
            semantic_routing_runtime_default_authority_counts[
                str(routing.get("runtime_default_authority") or "not_recorded")
            ] += 1
            semantic_routing_normalizer_role_counts[
                str(routing.get("normalizer_role") or "not_recorded")
            ] += 1
            semantic_routing_post_route_role_counts[
                str(routing.get("post_route_role") or "not_recorded")
            ] += 1
            semantic_routing_direct_answer_gate_role_counts[
                str(routing.get("direct_answer_gate_role") or "not_recorded")
            ] += 1
            gates = pre_agent_gates(boundary_context)
            intent_normalizer = dict_value(gates.get("intent_normalizer"))
            post_route_policy = dict_value(gates.get("post_route_policy"))
            direct_answer_gate = dict_value(gates.get("direct_answer_gate"))
            pre_agent_intent_authority_counts[
                str(intent_normalizer.get("authority_target") or "not_recorded")
            ] += 1
            pre_agent_intent_ownership_class_counts[
                str(intent_normalizer.get("ownership_class") or "not_recorded")
            ] += 1
            pre_agent_intent_boundary_allowed_counts[
                bool_token(intent_normalizer.get("boundary_allowed"))
            ] += 1
            pre_agent_intent_semantic_migration_target_counts[
                str(intent_normalizer.get("semantic_migration_target") or "not_recorded")
            ] += 1
            pre_agent_post_route_boundary_class_counts[
                str(post_route_policy.get("boundary_class") or "not_recorded")
            ] += 1
            pre_agent_post_route_ownership_class_counts[
                str(post_route_policy.get("ownership_class") or "not_recorded")
            ] += 1
            pre_agent_post_route_boundary_allowed_counts[
                bool_token(post_route_policy.get("boundary_allowed"))
            ] += 1
            pre_agent_post_route_semantic_migration_target_counts[
                str(post_route_policy.get("semantic_migration_target") or "not_recorded")
            ] += 1
            pre_agent_direct_answer_observation_class_counts[
                str(direct_answer_gate.get("observation_class") or "not_recorded")
            ] += 1
            pre_agent_direct_answer_boundary_class_counts[
                str(direct_answer_gate.get("boundary_class") or "not_recorded")
            ] += 1
            pre_agent_direct_answer_ownership_class_counts[
                str(direct_answer_gate.get("ownership_class") or "not_recorded")
            ] += 1
            pre_agent_direct_answer_boundary_allowed_counts[
                bool_token(direct_answer_gate.get("boundary_allowed"))
            ] += 1
            pre_agent_direct_answer_semantic_migration_target_counts[
                str(direct_answer_gate.get("semantic_migration_target") or "not_recorded")
            ] += 1
            reason_code_counts[str(item.get("reason_code") or "unknown")] += 1
            if (
                decision_delta not in {"same_gate", "not_evaluated", "not_comparable"}
                and len(mismatch_examples) < max_examples
            ):
                mismatch_examples.append(compact_item(path, obj, item))
            explanation = mismatch_explanation(item)
            mismatch_explanation_counts[explanation] += 1
            if (
                explanation == "unexplained"
                and len(unexplained_mismatch_examples) < max_examples
            ):
                unexplained_mismatch_examples.append(compact_item(path, obj, item))

    mismatch_explanation_json = counter_json(mismatch_explanation_counts)
    mismatch_count = sum(
        count for key, count in mismatch_explanation_json.items() if key != "not_mismatch"
    )
    return {
        "schema_version": 1,
        "run_dir": source_run_dir or str(run_dir),
        "switch_names": sorted(ROUTE_DELTA_SWITCH_NAMES),
        "turn_files": len(paths),
        "parse_errors": parse_errors,
        "parse_error_examples": parse_error_examples,
        "tasks_with_route_delta_items": len(tasks_with_items),
        "route_delta_items": attribution_items,
        "tasks_with_agent_decides_items": len(tasks_with_items),
        "agent_decides_items": attribution_items,
        "status_counts": counter_json(status_counts),
        "final_status_counts": counter_json(final_status_counts),
        "final_stop_signal_counts": counter_json(final_stop_signal_counts),
        "route_gate_counts": counter_json(route_gate_counts),
        "clarification_counts": counter_json(clarification_counts),
        "verifier_pass_counts": counter_json(verifier_pass_counts),
        "contract_match_counts": counter_json(contract_match_counts),
        "contract_final_answer_shape_counts": counter_json(contract_final_shape_counts),
        "finalizer_answer_shape_counts": counter_json(finalizer_shape_counts),
        "finalizer_answer_shape_class_counts": counter_json(finalizer_shape_class_counts),
        "llm": {
            "total_calls": total_llm_calls,
            "total_elapsed_ms": total_llm_elapsed_ms,
            "avg_calls_per_turn": ratio(total_llm_calls, len(paths)),
            "avg_elapsed_ms_per_turn": ratio(total_llm_elapsed_ms, len(paths)),
            "by_prompt_calls": counter_json(llm_prompt_call_counts),
            "by_prompt_elapsed_ms": counter_json(llm_prompt_elapsed_ms_counts),
            "by_phase_calls": counter_json(llm_phase_call_counts),
            "by_phase_elapsed_ms": counter_json(llm_phase_elapsed_ms_counts),
            "pre_planner": {
                "total_calls": llm_phase_call_counts["pre_planner"],
                "turns_with_calls": turns_with_pre_planner_llm,
                "avg_calls_per_turn": ratio(llm_phase_call_counts["pre_planner"], len(paths)),
                "avg_calls_per_turn_with_calls": ratio(
                    llm_phase_call_counts["pre_planner"],
                    turns_with_pre_planner_llm,
                ),
            },
        },
        "execution": {
            "round_count": total_rounds,
            "step_count": total_steps,
            "tool_call_count": total_tool_calls,
        },
        "event_counts": counter_json(event_counts),
        "switch_name_counts": counter_json(switch_name_counts),
        "outcome_counts": counter_json(outcome_counts),
        "decision_delta_counts": counter_json(decision_delta_counts),
        "old_first_layer_decision_counts": counter_json(old_decision_counts),
        "agent_decision_counts": counter_json(agent_decision_counts),
        "decision_envelope_decision_counts": counter_json(decision_envelope_decision_counts),
        "decision_envelope_validation_status_counts": counter_json(
            decision_envelope_validation_status_counts
        ),
        "decision_envelope_validation_reason_counts": counter_json(
            decision_envelope_validation_reason_counts
        ),
        "round_decision_envelope_source_counts": counter_json(
            round_decision_envelope_source_counts
        ),
        "round_decision_envelope_authority_counts": counter_json(
            round_decision_envelope_authority_counts
        ),
        "round_decision_envelope_decision_counts": counter_json(
            round_decision_envelope_decision_counts
        ),
        "round_decision_envelope_capability_counts": counter_json(
            round_decision_envelope_capability_counts
        ),
        "round_decision_envelope_validation_status_counts": counter_json(
            round_decision_envelope_validation_status_counts
        ),
        "round_decision_envelope_validation_reason_counts": counter_json(
            round_decision_envelope_validation_reason_counts
        ),
        "capability_delta_counts": counter_json(capability_delta_counts),
        "risk_delta_counts": counter_json(risk_delta_counts),
        "output_contract_delta_counts": counter_json(output_contract_delta_counts),
        "evidence_delta_counts": counter_json(evidence_delta_counts),
        "budget_profile_counts": counter_json(budget_profile_counts),
        "configured_migration_class_counts": counter_json(configured_migration_class_counts),
        "eligible_migration_class_counts": counter_json(eligible_migration_class_counts),
        "selected_migration_class_counts": counter_json(selected_migration_class_counts),
        "semantic_routing_activation_state_counts": counter_json(
            semantic_routing_activation_state_counts
        ),
        "semantic_routing_authority_counts": counter_json(semantic_routing_authority_counts),
        "semantic_routing_runtime_default_authority_counts": counter_json(
            semantic_routing_runtime_default_authority_counts
        ),
        "semantic_routing_normalizer_role_counts": counter_json(
            semantic_routing_normalizer_role_counts
        ),
        "semantic_routing_post_route_role_counts": counter_json(
            semantic_routing_post_route_role_counts
        ),
        "semantic_routing_direct_answer_gate_role_counts": counter_json(
            semantic_routing_direct_answer_gate_role_counts
        ),
        "pre_agent_intent_authority_counts": counter_json(pre_agent_intent_authority_counts),
        "pre_agent_intent_ownership_class_counts": counter_json(
            pre_agent_intent_ownership_class_counts
        ),
        "pre_agent_intent_boundary_allowed_counts": counter_json(
            pre_agent_intent_boundary_allowed_counts
        ),
        "pre_agent_intent_semantic_migration_target_counts": counter_json(
            pre_agent_intent_semantic_migration_target_counts
        ),
        "pre_agent_post_route_boundary_class_counts": counter_json(
            pre_agent_post_route_boundary_class_counts
        ),
        "pre_agent_post_route_ownership_class_counts": counter_json(
            pre_agent_post_route_ownership_class_counts
        ),
        "pre_agent_post_route_boundary_allowed_counts": counter_json(
            pre_agent_post_route_boundary_allowed_counts
        ),
        "pre_agent_post_route_semantic_migration_target_counts": counter_json(
            pre_agent_post_route_semantic_migration_target_counts
        ),
        "pre_agent_direct_answer_observation_class_counts": counter_json(
            pre_agent_direct_answer_observation_class_counts
        ),
        "pre_agent_direct_answer_boundary_class_counts": counter_json(
            pre_agent_direct_answer_boundary_class_counts
        ),
        "pre_agent_direct_answer_ownership_class_counts": counter_json(
            pre_agent_direct_answer_ownership_class_counts
        ),
        "pre_agent_direct_answer_boundary_allowed_counts": counter_json(
            pre_agent_direct_answer_boundary_allowed_counts
        ),
        "pre_agent_direct_answer_semantic_migration_target_counts": counter_json(
            pre_agent_direct_answer_semantic_migration_target_counts
        ),
        "runtime_decision_source_counts": counter_json(runtime_decision_source_counts),
        "runtime_semantic_control_state_counts": counter_json(
            runtime_semantic_control_state_counts
        ),
        "runtime_rewrite_reason_counts": counter_json(runtime_rewrite_reason_counts),
        "reason_code_counts": counter_json(reason_code_counts),
        "mismatch_count": mismatch_count,
        "unexplained_mismatch_count": mismatch_explanation_json.get("unexplained", 0),
        "mismatch_explanation_counts": mismatch_explanation_json,
        "mismatch_examples": mismatch_examples,
        "unexplained_mismatch_examples": unexplained_mismatch_examples,
    }


def summarize_run_dirs(
    run_dirs: list[Path],
    max_examples: int,
    dedupe_latest_case: bool = False,
) -> dict[str, Any]:
    if dedupe_latest_case:
        selected_paths, case_dedupe = latest_valid_case_paths(run_dirs)
        summary = summarize_run(
            run_dirs[0],
            max_examples,
            paths=selected_paths,
            source_run_dir="multiple",
        )
        summary["run_dirs"] = [str(run_dir) for run_dir in run_dirs]
        summary["case_dedupe"] = case_dedupe
        return summary

    if len(run_dirs) == 1:
        return summarize_run(run_dirs[0], max_examples)

    summaries = [summarize_run(run_dir, max_examples) for run_dir in run_dirs]
    turn_files = sum(safe_int(summary.get("turn_files")) for summary in summaries)
    parse_errors = sum(safe_int(summary.get("parse_errors")) for summary in summaries)
    route_delta_items = sum(
        safe_int(summary.get("route_delta_items") or summary.get("agent_decides_items"))
        for summary in summaries
    )
    tasks_with_items = sum(
        safe_int(
            summary.get("tasks_with_route_delta_items")
            or summary.get("tasks_with_agent_decides_items")
        )
        for summary in summaries
    )
    total_llm_calls = sum(
        safe_int(dict_value(summary.get("llm")).get("total_calls"))
        for summary in summaries
    )
    total_llm_elapsed_ms = sum(
        safe_int(dict_value(summary.get("llm")).get("total_elapsed_ms"))
        for summary in summaries
    )
    llm_prompt_calls: Counter[str] = Counter()
    llm_prompt_elapsed_ms: Counter[str] = Counter()
    llm_phase_calls: Counter[str] = Counter()
    llm_phase_elapsed_ms: Counter[str] = Counter()
    pre_planner_calls = 0
    pre_planner_turns_with_calls = 0
    execution_rounds = 0
    execution_steps = 0
    execution_tool_calls = 0
    mismatch_examples: list[dict[str, Any]] = []
    unexplained_mismatch_examples: list[dict[str, Any]] = []
    parse_error_examples: list[dict[str, Any]] = []

    for summary in summaries:
        llm = dict_value(summary.get("llm"))
        for key, value in dict_value(llm.get("by_prompt_calls")).items():
            llm_prompt_calls[str(key)] += safe_int(value)
        for key, value in dict_value(llm.get("by_prompt_elapsed_ms")).items():
            llm_prompt_elapsed_ms[str(key)] += safe_int(value)
        for key, value in dict_value(llm.get("by_phase_calls")).items():
            llm_phase_calls[str(key)] += safe_int(value)
        for key, value in dict_value(llm.get("by_phase_elapsed_ms")).items():
            llm_phase_elapsed_ms[str(key)] += safe_int(value)
        pre_planner = dict_value(llm.get("pre_planner"))
        pre_planner_calls += safe_int(pre_planner.get("total_calls"))
        pre_planner_turns_with_calls += safe_int(pre_planner.get("turns_with_calls"))
        execution = dict_value(summary.get("execution"))
        execution_rounds += safe_int(execution.get("round_count"))
        execution_steps += safe_int(execution.get("step_count"))
        execution_tool_calls += safe_int(execution.get("tool_call_count"))
        run_dir = str(summary.get("run_dir") or "")
        for example in list_value(summary.get("mismatch_examples")):
            if not isinstance(example, dict) or len(mismatch_examples) >= max_examples:
                continue
            merged_example = dict(example)
            merged_example["run_dir"] = run_dir
            mismatch_examples.append(merged_example)
        for example in list_value(summary.get("unexplained_mismatch_examples")):
            if (
                not isinstance(example, dict)
                or len(unexplained_mismatch_examples) >= max_examples
            ):
                continue
            merged_example = dict(example)
            merged_example["run_dir"] = run_dir
            unexplained_mismatch_examples.append(merged_example)
        for example in list_value(summary.get("parse_error_examples")):
            if not isinstance(example, dict) or len(parse_error_examples) >= max_examples:
                continue
            merged_example = dict(example)
            merged_example["run_dir"] = run_dir
            parse_error_examples.append(merged_example)

    merged: dict[str, Any] = {
        "schema_version": 1,
        "run_dir": "multiple",
        "run_dirs": [str(run_dir) for run_dir in run_dirs],
        "switch_names": sorted(ROUTE_DELTA_SWITCH_NAMES),
        "turn_files": turn_files,
        "parse_errors": parse_errors,
        "parse_error_examples": parse_error_examples,
        "tasks_with_route_delta_items": tasks_with_items,
        "route_delta_items": route_delta_items,
        "tasks_with_agent_decides_items": tasks_with_items,
        "agent_decides_items": route_delta_items,
        "llm": {
            "total_calls": total_llm_calls,
            "total_elapsed_ms": total_llm_elapsed_ms,
            "avg_calls_per_turn": ratio(total_llm_calls, turn_files),
            "avg_elapsed_ms_per_turn": ratio(total_llm_elapsed_ms, turn_files),
            "by_prompt_calls": counter_json(llm_prompt_calls),
            "by_prompt_elapsed_ms": counter_json(llm_prompt_elapsed_ms),
            "by_phase_calls": counter_json(llm_phase_calls),
            "by_phase_elapsed_ms": counter_json(llm_phase_elapsed_ms),
            "pre_planner": {
                "total_calls": pre_planner_calls,
                "turns_with_calls": pre_planner_turns_with_calls,
                "avg_calls_per_turn": ratio(pre_planner_calls, turn_files),
                "avg_calls_per_turn_with_calls": ratio(
                    pre_planner_calls,
                    pre_planner_turns_with_calls,
                ),
            },
        },
        "execution": {
            "round_count": execution_rounds,
            "step_count": execution_steps,
            "tool_call_count": execution_tool_calls,
        },
        "mismatch_examples": mismatch_examples,
        "unexplained_mismatch_examples": unexplained_mismatch_examples,
    }
    for key in COUNTER_FIELDS:
        merged[key] = merge_counter_dicts(summaries, key)
    mismatch_explanations = dict_value(merged.get("mismatch_explanation_counts"))
    merged["mismatch_count"] = sum(
        safe_int(count)
        for key, count in mismatch_explanations.items()
        if key != "not_mismatch"
    )
    merged["unexplained_mismatch_count"] = safe_int(
        mismatch_explanations.get("unexplained")
    )
    return merged


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("run_dirs", nargs="+", type=Path)
    parser.add_argument("--max-examples", type=int, default=20)
    parser.add_argument(
        "--require-items",
        action="store_true",
        help="Exit non-zero when no route-delta attribution was found.",
    )
    parser.add_argument(
        "--allow-parse-errors",
        action="store_true",
        help="Report parse errors but do not fail when scanning historical partial run logs.",
    )
    parser.add_argument(
        "--dedupe-latest-case",
        action="store_true",
        help="For rerun shards, keep only the latest valid turn per numeric case id.",
    )
    parser.add_argument(
        "--expect-case-count",
        type=int,
        default=0,
        help="Fail when --dedupe-latest-case finds fewer unique cases than this count.",
    )
    args = parser.parse_args()

    for run_dir in args.run_dirs:
        if not run_dir.is_dir():
            raise SystemExit(f"run dir not found: {run_dir}")
    summary = summarize_run_dirs(
        args.run_dirs,
        max(args.max_examples, 0),
        dedupe_latest_case=args.dedupe_latest_case,
    )
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True, indent=2))
    if args.require_items and summary["route_delta_items"] <= 0:
        return 2
    if args.expect_case_count:
        case_count = safe_int(dict_path(summary, "case_dedupe", "case_count"))
        if case_count < args.expect_case_count:
            return 3
    if summary["parse_errors"] and not args.allow_parse_errors:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
