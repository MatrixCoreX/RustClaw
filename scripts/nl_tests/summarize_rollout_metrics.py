#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


EXTERNAL_SKILL_HINTS = {
    "crypto",
    "stock",
    "weather",
    "web_search_extract",
    "rss_news",
    "x",
}
COUNTER_FIELDS = (
    "status_counts",
    "final_status_counts",
    "attribution_counts",
    "first_layer_decision_counts",
    "route_gate_counts",
    "failure_attribution_counts",
    "verifier_pass_counts",
    "provider_counts",
    "vendor_counts",
    "budget_profile_counts",
    "language_counts",
    "semantic_kind_counts",
    "contract_match_counts",
    "final_answer_shape_counts",
    "capability_counts",
    "planner_first_action_counts",
    "delivery_consistent_counts",
    "rollout_switch_counts",
    "rollout_event_counts",
    "rollout_reason_counts",
    "configured_migration_class_counts",
    "eligible_migration_class_counts",
    "selected_migration_class_counts",
    "agent_loop_eligibility_bucket_counts",
    "agent_loop_eligibility_blocked_reason_counts",
    "agent_loop_authority_enabled_counts",
    "semantic_routing_activation_state_counts",
    "semantic_routing_authority_counts",
    "semantic_routing_chosen_authority_counts",
    "semantic_routing_runtime_default_authority_counts",
    "semantic_routing_normalizer_role_counts",
    "semantic_routing_post_route_role_counts",
    "semantic_routing_direct_answer_gate_role_counts",
)
CLARIFICATION_FINAL_STATUS_KEYS = {"clarify", "clarification_requested"}
VERIFIER_BLOCK_KEYS = {"False", "false"}


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as err:
        return {"_parse_error": str(err)}
    return value if isinstance(value, dict) else {"_parse_error": "top level is not object"}


def get_path(obj: dict[str, Any], *keys: str) -> Any:
    cur: Any = obj
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def safe_int(value: Any) -> int:
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    return 0


def ratio(numerator: int, denominator: int) -> float:
    if denominator <= 0:
        return 0.0
    return round(numerator / denominator, 6)


def rate_from_counts(counts: Any, keys: set[str], total: int) -> float:
    if not isinstance(counts, dict) or total <= 0:
        return 0.0
    return ratio(sum(safe_int(counts.get(key)) for key in keys), total)


def relative_increase(candidate: float, baseline: float) -> float | None:
    if baseline <= 0:
        return None
    return round((candidate - baseline) / baseline, 6)


def threshold_result(
    passed: bool,
    baseline: float,
    candidate: float,
    observed: float | None,
    threshold: float,
) -> dict[str, Any]:
    return {
        "baseline": baseline,
        "candidate": candidate,
        "observed": observed,
        "threshold": threshold,
        "passed": passed,
    }


def compare_with_baseline(
    candidate: dict[str, Any],
    baseline: dict[str, Any],
    max_pass_rate_drop: float,
    max_clarification_rate_rise: float,
    max_verifier_block_rate: float,
    max_avg_llm_calls_rise: float,
    max_avg_elapsed_rise: float,
) -> dict[str, Any]:
    candidate_turns = safe_int(candidate.get("turns_total"))
    baseline_turns = safe_int(baseline.get("turns_total"))
    candidate_pass_rate = float(candidate.get("pass_rate") or 0.0)
    baseline_pass_rate = float(baseline.get("pass_rate") or 0.0)
    candidate_clarification_rate = rate_from_counts(
        candidate.get("final_status_counts"),
        CLARIFICATION_FINAL_STATUS_KEYS,
        candidate_turns,
    )
    baseline_clarification_rate = rate_from_counts(
        baseline.get("final_status_counts"),
        CLARIFICATION_FINAL_STATUS_KEYS,
        baseline_turns,
    )
    candidate_verifier_block_rate = rate_from_counts(
        candidate.get("verifier_pass_counts"),
        VERIFIER_BLOCK_KEYS,
        candidate_turns,
    )
    candidate_avg_llm_calls = float(get_path(candidate, "llm", "avg_calls_per_turn") or 0.0)
    baseline_avg_llm_calls = float(get_path(baseline, "llm", "avg_calls_per_turn") or 0.0)
    candidate_avg_elapsed = float(
        get_path(candidate, "llm", "avg_elapsed_ms_per_turn") or 0.0
    )
    baseline_avg_elapsed = float(
        get_path(baseline, "llm", "avg_elapsed_ms_per_turn") or 0.0
    )

    pass_rate_drop = round(baseline_pass_rate - candidate_pass_rate, 6)
    clarification_rate_rise = round(
        candidate_clarification_rate - baseline_clarification_rate,
        6,
    )
    avg_llm_calls_rise = relative_increase(
        candidate_avg_llm_calls,
        baseline_avg_llm_calls,
    )
    avg_elapsed_rise = relative_increase(candidate_avg_elapsed, baseline_avg_elapsed)
    comparisons = {
        "pass_rate_drop": threshold_result(
            pass_rate_drop <= max_pass_rate_drop,
            baseline_pass_rate,
            candidate_pass_rate,
            pass_rate_drop,
            max_pass_rate_drop,
        ),
        "clarification_rate_rise": threshold_result(
            clarification_rate_rise <= max_clarification_rate_rise,
            baseline_clarification_rate,
            candidate_clarification_rate,
            clarification_rate_rise,
            max_clarification_rate_rise,
        ),
        "verifier_block_rate": threshold_result(
            candidate_verifier_block_rate <= max_verifier_block_rate,
            0.0,
            candidate_verifier_block_rate,
            candidate_verifier_block_rate,
            max_verifier_block_rate,
        ),
        "avg_llm_calls_relative_increase": threshold_result(
            avg_llm_calls_rise is not None and avg_llm_calls_rise <= max_avg_llm_calls_rise,
            baseline_avg_llm_calls,
            candidate_avg_llm_calls,
            avg_llm_calls_rise,
            max_avg_llm_calls_rise,
        ),
        "avg_elapsed_relative_increase": threshold_result(
            avg_elapsed_rise is not None and avg_elapsed_rise <= max_avg_elapsed_rise,
            baseline_avg_elapsed,
            candidate_avg_elapsed,
            avg_elapsed_rise,
            max_avg_elapsed_rise,
        ),
    }
    failures = [
        key for key, value in comparisons.items() if value.get("passed") is not True
    ]
    return {
        "baseline_source": baseline.get("source_run_dir") or baseline.get("source_run_dirs"),
        "candidate_source": candidate.get("source_run_dir")
        or candidate.get("source_run_dirs"),
        "candidate_turns": candidate_turns,
        "baseline_turns": baseline_turns,
        "comparisons": comparisons,
        "failures": failures,
        "passed": not failures,
    }


def looks_like_language_neutral_artifact_token(token: str) -> bool:
    token = token.strip().strip("\"'`()[]{}<>")
    if not token or not any(ch.isalpha() for ch in token):
        return False
    if any("\u3040" <= ch <= "\u30ff" or "\u4e00" <= ch <= "\u9fff" or "\uac00" <= ch <= "\ud7af" for ch in token):
        return False
    if (
        "://" in token
        or token.startswith(("/", "./", "../", "~/"))
        or "/" in token
        or "\\" in token
        or "_" in token
        or "-" in token
    ):
        return True
    if "." in token:
        head, _, ext = token.rpartition(".")
        return bool(head and ext and len(ext) <= 12 and ext.isalnum())
    return token.isupper() and len(token) >= 2


def text_is_language_neutral_artifact_only(text: str) -> bool:
    tokens = [token for token in text.split() if token.strip()]
    return bool(tokens) and all(looks_like_language_neutral_artifact_token(token) for token in tokens)


def language_bucket(text: Any) -> str:
    if not isinstance(text, str) or not text.strip():
        return "unknown"
    if text_is_language_neutral_artifact_only(text):
        return "language_neutral"
    counts = {
        "zh": 0,
        "ja": 0,
        "ko": 0,
        "latin": 0,
        "cyrillic": 0,
        "arabic": 0,
        "other_alpha": 0,
    }
    for ch in text:
        code = ord(ch)
        if "\u3040" <= ch <= "\u30ff":
            counts["ja"] += 1
        elif "\uac00" <= ch <= "\ud7af":
            counts["ko"] += 1
        elif "\u4e00" <= ch <= "\u9fff":
            counts["zh"] += 1
        elif ch.isascii() and ch.isalpha():
            counts["latin"] += 1
        elif 0x0400 <= code <= 0x04FF:
            counts["cyrillic"] += 1
        elif 0x0600 <= code <= 0x06FF:
            counts["arabic"] += 1
        elif ch.isalpha():
            counts["other_alpha"] += 1
    if counts["ja"]:
        return "ja"
    if counts["ko"]:
        return "ko"
    if counts["zh"]:
        return "zh-CN" if counts["zh"] >= counts["latin"] else "mixed"
    if counts["cyrillic"]:
        return "und-Cyrl"
    if counts["arabic"]:
        return "und-Arab"
    if counts["other_alpha"]:
        return "und"
    if counts["latin"]:
        return "en"
    return "language_neutral"


def machine_token_value(text: Any, key: str) -> str:
    if not isinstance(text, str):
        return ""
    prefix = f"{key}="
    for token in text.split():
        if token.startswith(prefix):
            value = token[len(prefix) :].strip().strip(",;")
            if value:
                return value
    return ""


def load_attribution_counts(run_dir: Path) -> Counter[str]:
    counts: Counter[str] = Counter()
    path = run_dir / "attribution.jsonl"
    if not path.exists():
        return counts
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            item = json.loads(line)
        except Exception:
            counts["parse_error"] += 1
            continue
        attribution = item.get("attribution")
        if isinstance(attribution, str) and attribution.strip():
            counts[attribution.strip()] += 1
    return counts


def turn_json_paths(run_dir: Path) -> list[Path]:
    return sorted(run_dir.glob("turn*_case_*.json"))


def step_results(trace: dict[str, Any]) -> list[dict[str, Any]]:
    steps = trace.get("step_results")
    if not isinstance(steps, list):
        return []
    return [step for step in steps if isinstance(step, dict)]


def trace_rounds(trace: dict[str, Any]) -> list[dict[str, Any]]:
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return []
    return [round_item for round_item in rounds if isinstance(round_item, dict)]


def planner_first_action(trace: dict[str, Any]) -> str:
    for round_item in trace_rounds(trace):
        plan = dict_value(round_item.get("plan_result"))
        steps = plan.get("steps")
        if not isinstance(steps, list):
            continue
        for step in steps:
            if not isinstance(step, dict):
                continue
            for key in ("action_ref", "capability", "tool", "skill", "action"):
                value = step.get(key)
                if isinstance(value, str) and value.strip():
                    return value.strip()
            step_type = step.get("type")
            if isinstance(step_type, str) and step_type.strip():
                return step_type.strip()
    return "not_recorded"


def bool_token(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "missing"
    return str(value)


def count_tool_calls(steps: list[dict[str, Any]]) -> tuple[int, int]:
    tool_calls = 0
    external_tool_calls = 0
    for step in steps:
        skill = str(step.get("skill") or "").strip()
        if not skill or skill in {"synthesize_answer", "respond", "think"}:
            continue
        tool_calls += 1
        if skill in EXTERNAL_SKILL_HINTS:
            external_tool_calls += 1
    return tool_calls, external_tool_calls


def update_by_prompt_totals(
    totals: dict[str, dict[str, Any]],
    by_prompt: dict[str, Any],
) -> None:
    numeric_fields = (
        "count",
        "elapsed_ms",
        "provider_attempt_count",
        "provider_retry_count",
        "provider_retryable_error_count",
        "provider_final_error_count",
        "prompt_truncation_count",
        "prompt_truncated_bytes_total",
    )
    for label, bucket in by_prompt.items():
        if not isinstance(label, str) or not isinstance(bucket, dict):
            continue
        target = totals[label]
        for field in numeric_fields:
            target[field] = safe_int(target.get(field)) + safe_int(bucket.get(field))
        merge_nested_counter(target, "provider_last_retry_error_kinds", bucket.get("provider_last_retry_error_kinds"))
        merge_nested_counter(target, "provider_final_error_kinds", bucket.get("provider_final_error_kinds"))
        update_optional_max(target, "prompt_bytes_before_max", bucket.get("prompt_bytes_before_max"))
        update_optional_max(target, "prompt_bytes_after_max", bucket.get("prompt_bytes_after_max"))
        update_optional_min(target, "prompt_bytes_budget_min", bucket.get("prompt_bytes_budget_min"))


def merge_nested_counter(target: dict[str, Any], key: str, value: Any) -> None:
    if not isinstance(value, dict):
        return
    current = target.get(key)
    if not isinstance(current, dict):
        current = {}
        target[key] = current
    for item_key, item_value in value.items():
        current[str(item_key)] = safe_int(current.get(str(item_key))) + safe_int(item_value)


def sum_by_prompt_field(by_prompt: dict[str, dict[str, Any]], field: str) -> int:
    return sum(safe_int(bucket.get(field)) for bucket in by_prompt.values() if isinstance(bucket, dict))


def update_optional_max(target: dict[str, Any], key: str, value: Any) -> None:
    number = safe_int(value)
    if number <= 0:
        return
    current = safe_int(target.get(key))
    target[key] = max(current, number) if current > 0 else number


def update_optional_min(target: dict[str, Any], key: str, value: Any) -> None:
    number = safe_int(value)
    if number <= 0:
        return
    current = safe_int(target.get(key))
    target[key] = min(current, number) if current > 0 else number


def merge_counter_field(summaries: list[dict[str, Any]], key: str) -> dict[str, int]:
    counts: Counter[str] = Counter()
    for summary in summaries:
        value = summary.get(key)
        if not isinstance(value, dict):
            continue
        for item_key, item_value in value.items():
            counts[str(item_key)] += safe_int(item_value)
    return dict(sorted(counts.items()))


def merge_by_prompt(summaries: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    totals: dict[str, dict[str, Any]] = defaultdict(
        lambda: {"count": 0, "elapsed_ms": 0, "prompt_truncation_count": 0}
    )
    for summary in summaries:
        llm = summary.get("llm")
        if not isinstance(llm, dict):
            continue
        by_prompt = llm.get("by_prompt")
        if not isinstance(by_prompt, dict):
            continue
        update_by_prompt_totals(totals, by_prompt)
    return dict(sorted(totals.items()))


def prompt_latency_diagnostics(by_prompt: Any, limit: int = 5) -> dict[str, Any]:
    if not isinstance(by_prompt, dict):
        return {"top_by_total_elapsed_ms": [], "top_by_avg_elapsed_ms": []}
    rows: list[dict[str, Any]] = []
    for label, bucket in by_prompt.items():
        if not isinstance(label, str) or not isinstance(bucket, dict):
            continue
        count = safe_int(bucket.get("count"))
        elapsed_ms = safe_int(bucket.get("elapsed_ms"))
        rows.append(
            {
                "prompt": label,
                "count": count,
                "elapsed_ms": elapsed_ms,
                "avg_elapsed_ms": round(elapsed_ms / count, 3) if count else 0.0,
                "provider_attempt_count": safe_int(bucket.get("provider_attempt_count")),
                "provider_retry_count": safe_int(bucket.get("provider_retry_count")),
                "provider_retryable_error_count": safe_int(bucket.get("provider_retryable_error_count")),
                "provider_final_error_count": safe_int(bucket.get("provider_final_error_count")),
                "provider_last_retry_error_kinds": bucket.get("provider_last_retry_error_kinds"),
                "provider_final_error_kinds": bucket.get("provider_final_error_kinds"),
                "prompt_truncation_count": safe_int(bucket.get("prompt_truncation_count")),
                "prompt_bytes_before_max": bucket.get("prompt_bytes_before_max"),
                "prompt_bytes_after_max": bucket.get("prompt_bytes_after_max"),
                "prompt_bytes_budget_min": bucket.get("prompt_bytes_budget_min"),
            }
        )
    return {
        "top_by_total_elapsed_ms": sorted(
            rows,
            key=lambda row: (row["elapsed_ms"], row["count"], row["prompt"]),
            reverse=True,
        )[:limit],
        "top_by_avg_elapsed_ms": sorted(
            rows,
            key=lambda row: (row["avg_elapsed_ms"], row["elapsed_ms"], row["prompt"]),
            reverse=True,
        )[:limit],
    }


def summarize_run(
    run_dir: Path,
    provider: str = "unknown",
    vendor: str = "unknown",
    budget_profile: str = "unknown",
) -> dict[str, Any]:
    paths = turn_json_paths(run_dir)
    status_counts: Counter[str] = Counter()
    final_status_counts: Counter[str] = Counter()
    first_layer_counts: Counter[str] = Counter()
    route_gate_counts: Counter[str] = Counter()
    failure_attribution_counts: Counter[str] = Counter()
    verifier_pass_counts: Counter[str] = Counter()
    rollout_switch_counts: Counter[str] = Counter()
    rollout_event_counts: Counter[str] = Counter()
    rollout_reason_counts: Counter[str] = Counter()
    provider_counts: Counter[str] = Counter()
    vendor_counts: Counter[str] = Counter()
    budget_profile_counts: Counter[str] = Counter()
    language_counts: Counter[str] = Counter()
    semantic_kind_counts: Counter[str] = Counter()
    contract_match_counts: Counter[str] = Counter()
    final_answer_shape_counts: Counter[str] = Counter()
    capability_counts: Counter[str] = Counter()
    planner_first_action_counts: Counter[str] = Counter()
    delivery_consistent_counts: Counter[str] = Counter()
    by_prompt_totals: dict[str, dict[str, Any]] = defaultdict(
        lambda: {"count": 0, "elapsed_ms": 0, "prompt_truncation_count": 0}
    )
    configured_migration_class_counts: Counter[str] = Counter()
    eligible_migration_class_counts: Counter[str] = Counter()
    selected_migration_class_counts: Counter[str] = Counter()
    agent_loop_eligibility_bucket_counts: Counter[str] = Counter()
    agent_loop_eligibility_blocked_reason_counts: Counter[str] = Counter()
    agent_loop_authority_enabled_counts: Counter[str] = Counter()
    semantic_routing_activation_state_counts: Counter[str] = Counter()
    semantic_routing_authority_counts: Counter[str] = Counter()
    semantic_routing_chosen_authority_counts: Counter[str] = Counter()
    semantic_routing_runtime_default_authority_counts: Counter[str] = Counter()
    semantic_routing_normalizer_role_counts: Counter[str] = Counter()
    semantic_routing_post_route_role_counts: Counter[str] = Counter()
    semantic_routing_direct_answer_gate_role_counts: Counter[str] = Counter()
    parse_errors = 0
    total_llm_calls = 0
    total_llm_elapsed_ms = 0
    total_prompt_truncations = 0
    total_rounds = 0
    total_steps = 0
    total_tool_calls = 0
    total_external_tool_calls = 0

    for path in paths:
        obj = load_json(path)
        if obj.get("_parse_error"):
            parse_errors += 1
            continue
        data = obj.get("data") if isinstance(obj.get("data"), dict) else {}
        result = data.get("result_json") if isinstance(data.get("result_json"), dict) else {}
        journal = result.get("task_journal") if isinstance(result.get("task_journal"), dict) else {}
        summary = journal.get("summary") if isinstance(journal.get("summary"), dict) else {}
        trace = journal.get("trace") if isinstance(journal.get("trace"), dict) else {}
        route = summary.get("route_result") if isinstance(summary.get("route_result"), dict) else {}
        metrics = summary.get("task_metrics") if isinstance(summary.get("task_metrics"), dict) else {}
        verifier = (
            summary.get("answer_verifier_summary")
            if isinstance(summary.get("answer_verifier_summary"), dict)
            else {}
        )
        contract = trace.get("contract_matrix")
        if not isinstance(contract, dict):
            contract = get_path(trace, "runtime_contract_snapshot", "contract")
            if not isinstance(contract, dict):
                contract = {}

        status_counts[str(data.get("status") or "unknown")] += 1
        final_status_counts[str(summary.get("final_status") or "unknown")] += 1
        first_layer_counts[str(route.get("first_layer_decision") or "unknown")] += 1
        route_gate_counts[str(route.get("route_gate_kind") or "unknown")] += 1
        failure_attribution_counts[str(summary.get("final_failure_attribution") or "none")] += 1
        if verifier:
            verifier_pass_counts[str(verifier.get("pass"))] += 1
        else:
            verifier_pass_counts["missing"] += 1
        provider_counts[provider] += 1
        vendor_counts[vendor] += 1
        context_summary = summary.get("context_bundle_summary")
        detected_budget = (
            machine_token_value(context_summary, "budget_profile")
            or machine_token_value(context_summary, "execution_budget")
            or machine_token_value(context_summary, "route_budget")
            or budget_profile
        )
        budget_profile_counts[detected_budget] += 1
        language_counts[language_bucket(summary.get("input_text"))] += 1
        semantic_kind_counts[str(contract.get("semantic_kind") or "unknown")] += 1
        contract_match_counts[str(contract.get("contract_match") or "unknown")] += 1
        final_answer_shape_counts[str(contract.get("final_answer_shape") or "unknown")] += 1
        delivery_consistent_counts[bool_token(metrics.get("delivery_consistent"))] += 1
        planner_first_action_counts[planner_first_action(trace)] += 1

        total_rounds += safe_int(summary.get("round_count"))
        total_steps += safe_int(summary.get("step_count"))
        total_llm_calls += safe_int(metrics.get("llm_calls_per_task"))
        total_llm_elapsed_ms += safe_int(metrics.get("llm_elapsed_ms_per_task"))
        total_prompt_truncations += safe_int(metrics.get("prompt_truncation_count"))
        by_prompt = metrics.get("by_prompt")
        if isinstance(by_prompt, dict):
            update_by_prompt_totals(by_prompt_totals, by_prompt)

        switches = summary.get("rollout_switches_enabled")
        if isinstance(switches, list):
            for switch in switches:
                if isinstance(switch, str) and switch.strip():
                    rollout_switch_counts[switch.strip()] += 1
        attribution = summary.get("rollout_attribution")
        if isinstance(attribution, list):
            for item in attribution:
                if not isinstance(item, dict):
                    continue
                event = item.get("event")
                reason = item.get("reason_code")
                if isinstance(event, str) and event.strip():
                    rollout_event_counts[event.strip()] += 1
                if isinstance(reason, str) and reason.strip():
                    rollout_reason_counts[reason.strip()] += 1
                boundary_context = dict_value(item.get("boundary_context"))
                boundary_budget = dict_value(boundary_context.get("budget"))
                semantic_routing = dict_value(boundary_context.get("semantic_routing"))
                configured_migration_class_counts[
                    str(boundary_budget.get("agent_decides_migration_class") or "unknown")
                ] += 1
                eligible_migration_class_counts[
                    str(boundary_budget.get("eligible_migration_class") or "unknown")
                ] += 1
                selected_migration_class_counts[
                    str(boundary_budget.get("selected_migration_class") or "unknown")
                ] += 1
                agent_loop_eligibility_bucket_counts[
                    str(boundary_budget.get("agent_loop_eligibility_bucket") or "unknown")
                ] += 1
                agent_loop_eligibility_blocked_reason_counts[
                    str(
                        boundary_budget.get("agent_loop_eligibility_blocked_reason")
                        or "unknown"
                    )
                ] += 1
                agent_loop_authority_enabled_counts[
                    bool_token(semantic_routing.get("agent_loop_authority_enabled"))
                ] += 1
                semantic_routing_activation_state_counts[
                    str(semantic_routing.get("activation_state") or "not_recorded")
                ] += 1
                semantic_routing_authority_counts[
                    str(semantic_routing.get("ordinary_semantic_authority") or "not_recorded")
                ] += 1
                semantic_routing_chosen_authority_counts[
                    str(semantic_routing.get("chosen_authority") or "not_recorded")
                ] += 1
                semantic_routing_runtime_default_authority_counts[
                    str(semantic_routing.get("runtime_default_authority") or "not_recorded")
                ] += 1
                semantic_routing_normalizer_role_counts[
                    str(semantic_routing.get("normalizer_role") or "not_recorded")
                ] += 1
                semantic_routing_post_route_role_counts[
                    str(semantic_routing.get("post_route_role") or "not_recorded")
                ] += 1
                semantic_routing_direct_answer_gate_role_counts[
                    str(semantic_routing.get("direct_answer_gate_role") or "not_recorded")
                ] += 1

        steps = step_results(trace)
        for step in steps:
            capability = step.get("requested_capability") or step.get("skill")
            if isinstance(capability, str) and capability.strip():
                capability_counts[capability.strip()] += 1
        tool_calls, external_tool_calls = count_tool_calls(steps)
        total_tool_calls += tool_calls
        total_external_tool_calls += external_tool_calls

    total_turns = len(paths)
    succeeded = status_counts.get("succeeded", 0)
    attribution_counts = load_attribution_counts(run_dir)
    by_prompt = dict(sorted(by_prompt_totals.items()))
    provider_attempt_count = sum_by_prompt_field(by_prompt, "provider_attempt_count")
    provider_retry_count = sum_by_prompt_field(by_prompt, "provider_retry_count")
    provider_retryable_error_count = sum_by_prompt_field(by_prompt, "provider_retryable_error_count")
    provider_final_error_count = sum_by_prompt_field(by_prompt, "provider_final_error_count")
    return {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source_run_dir": str(run_dir),
        "turns_total": total_turns,
        "parse_errors": parse_errors,
        "pass_rate": ratio(succeeded, total_turns),
        "status_counts": dict(sorted(status_counts.items())),
        "final_status_counts": dict(sorted(final_status_counts.items())),
        "attribution_counts": dict(sorted(attribution_counts.items())),
        "first_layer_decision_counts": dict(sorted(first_layer_counts.items())),
        "route_gate_counts": dict(sorted(route_gate_counts.items())),
        "failure_attribution_counts": dict(sorted(failure_attribution_counts.items())),
        "verifier_pass_counts": dict(sorted(verifier_pass_counts.items())),
        "provider_counts": dict(sorted(provider_counts.items())),
        "vendor_counts": dict(sorted(vendor_counts.items())),
        "budget_profile_counts": dict(sorted(budget_profile_counts.items())),
        "language_counts": dict(sorted(language_counts.items())),
        "semantic_kind_counts": dict(sorted(semantic_kind_counts.items())),
        "contract_match_counts": dict(sorted(contract_match_counts.items())),
        "final_answer_shape_counts": dict(sorted(final_answer_shape_counts.items())),
        "capability_counts": dict(sorted(capability_counts.items())),
        "planner_first_action_counts": dict(sorted(planner_first_action_counts.items())),
        "delivery_consistent_counts": dict(sorted(delivery_consistent_counts.items())),
        "rollout_switch_counts": dict(sorted(rollout_switch_counts.items())),
        "rollout_event_counts": dict(sorted(rollout_event_counts.items())),
        "rollout_reason_counts": dict(sorted(rollout_reason_counts.items())),
        "configured_migration_class_counts": dict(
            sorted(configured_migration_class_counts.items())
        ),
        "eligible_migration_class_counts": dict(
            sorted(eligible_migration_class_counts.items())
        ),
        "selected_migration_class_counts": dict(
            sorted(selected_migration_class_counts.items())
        ),
        "agent_loop_eligibility_bucket_counts": dict(
            sorted(agent_loop_eligibility_bucket_counts.items())
        ),
        "agent_loop_eligibility_blocked_reason_counts": dict(
            sorted(agent_loop_eligibility_blocked_reason_counts.items())
        ),
        "agent_loop_authority_enabled_counts": dict(
            sorted(agent_loop_authority_enabled_counts.items())
        ),
        "semantic_routing_activation_state_counts": dict(
            sorted(semantic_routing_activation_state_counts.items())
        ),
        "semantic_routing_authority_counts": dict(
            sorted(semantic_routing_authority_counts.items())
        ),
        "semantic_routing_chosen_authority_counts": dict(
            sorted(semantic_routing_chosen_authority_counts.items())
        ),
        "semantic_routing_runtime_default_authority_counts": dict(
            sorted(semantic_routing_runtime_default_authority_counts.items())
        ),
        "semantic_routing_normalizer_role_counts": dict(
            sorted(semantic_routing_normalizer_role_counts.items())
        ),
        "semantic_routing_post_route_role_counts": dict(
            sorted(semantic_routing_post_route_role_counts.items())
        ),
        "semantic_routing_direct_answer_gate_role_counts": dict(
            sorted(semantic_routing_direct_answer_gate_role_counts.items())
        ),
        "llm": {
            "total_calls": total_llm_calls,
            "total_elapsed_ms": total_llm_elapsed_ms,
            "provider_attempt_count": provider_attempt_count,
            "provider_retry_count": provider_retry_count,
            "provider_retryable_error_count": provider_retryable_error_count,
            "provider_final_error_count": provider_final_error_count,
            "avg_calls_per_turn": round(total_llm_calls / total_turns, 3) if total_turns else 0.0,
            "avg_elapsed_ms_per_turn": round(total_llm_elapsed_ms / total_turns, 3)
            if total_turns
            else 0.0,
            "prompt_truncation_count": total_prompt_truncations,
            "by_prompt": by_prompt,
            "prompt_latency_diagnostics": prompt_latency_diagnostics(by_prompt),
        },
        "execution": {
            "round_count": total_rounds,
            "step_count": total_steps,
            "tool_call_count": total_tool_calls,
            "external_tool_call_count": total_external_tool_calls,
        },
    }


def summarize_run_dirs(
    run_dirs: list[Path],
    provider: str = "unknown",
    vendor: str = "unknown",
    budget_profile: str = "unknown",
) -> dict[str, Any]:
    if len(run_dirs) == 1:
        return summarize_run(
            run_dirs[0],
            provider=provider,
            vendor=vendor,
            budget_profile=budget_profile,
        )

    summaries = [
        summarize_run(
            run_dir,
            provider=provider,
            vendor=vendor,
            budget_profile=budget_profile,
        )
        for run_dir in run_dirs
    ]
    turns_total = sum(safe_int(summary.get("turns_total")) for summary in summaries)
    parse_errors = sum(safe_int(summary.get("parse_errors")) for summary in summaries)
    total_llm_calls = sum(
        safe_int(get_path(summary, "llm", "total_calls")) for summary in summaries
    )
    total_llm_elapsed_ms = sum(
        safe_int(get_path(summary, "llm", "total_elapsed_ms")) for summary in summaries
    )
    total_prompt_truncations = sum(
        safe_int(get_path(summary, "llm", "prompt_truncation_count"))
        for summary in summaries
    )
    provider_attempt_count = sum(
        safe_int(get_path(summary, "llm", "provider_attempt_count"))
        for summary in summaries
    )
    provider_retry_count = sum(
        safe_int(get_path(summary, "llm", "provider_retry_count"))
        for summary in summaries
    )
    provider_retryable_error_count = sum(
        safe_int(get_path(summary, "llm", "provider_retryable_error_count"))
        for summary in summaries
    )
    provider_final_error_count = sum(
        safe_int(get_path(summary, "llm", "provider_final_error_count"))
        for summary in summaries
    )
    total_rounds = sum(
        safe_int(get_path(summary, "execution", "round_count"))
        for summary in summaries
    )
    total_steps = sum(
        safe_int(get_path(summary, "execution", "step_count"))
        for summary in summaries
    )
    total_tool_calls = sum(
        safe_int(get_path(summary, "execution", "tool_call_count"))
        for summary in summaries
    )
    total_external_tool_calls = sum(
        safe_int(get_path(summary, "execution", "external_tool_call_count"))
        for summary in summaries
    )
    status_counts = merge_counter_field(summaries, "status_counts")
    by_prompt = merge_by_prompt(summaries)
    result: dict[str, Any] = {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source_run_dir": "multiple",
        "source_run_dirs": [str(run_dir) for run_dir in run_dirs],
        "turns_total": turns_total,
        "parse_errors": parse_errors,
        "pass_rate": ratio(status_counts.get("succeeded", 0), turns_total),
        "llm": {
            "total_calls": total_llm_calls,
            "total_elapsed_ms": total_llm_elapsed_ms,
            "provider_attempt_count": provider_attempt_count,
            "provider_retry_count": provider_retry_count,
            "provider_retryable_error_count": provider_retryable_error_count,
            "provider_final_error_count": provider_final_error_count,
            "avg_calls_per_turn": round(total_llm_calls / turns_total, 3)
            if turns_total
            else 0.0,
            "avg_elapsed_ms_per_turn": round(total_llm_elapsed_ms / turns_total, 3)
            if turns_total
            else 0.0,
            "prompt_truncation_count": total_prompt_truncations,
            "by_prompt": by_prompt,
            "prompt_latency_diagnostics": prompt_latency_diagnostics(by_prompt),
        },
        "execution": {
            "round_count": total_rounds,
            "step_count": total_steps,
            "tool_call_count": total_tool_calls,
            "external_tool_call_count": total_external_tool_calls,
        },
    }
    for key in COUNTER_FIELDS:
        result[key] = status_counts if key == "status_counts" else merge_counter_field(summaries, key)
    return result


def default_output_path(run_dirs: list[Path]) -> Path:
    if len(run_dirs) == 1:
        name = f"{run_dirs[0].name}_rollout_metrics.json"
    else:
        name = f"multi_{len(run_dirs)}_{run_dirs[0].name}_to_{run_dirs[-1].name}_rollout_metrics.json"
    return Path("logs/agent_rollout_metrics") / name


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Summarize client-like NL rollout metrics into stable JSON."
    )
    parser.add_argument("run_dirs", nargs="+", help="Client-like NL run directories")
    parser.add_argument(
        "--output",
        help="Output JSON path (default: logs/agent_rollout_metrics/<run>_rollout_metrics.json)",
    )
    parser.add_argument(
        "--provider",
        default=os.environ.get("RUSTCLAW_NL_PROVIDER", "unknown"),
        help="Provider bucket to attach when run JSON does not expose one",
    )
    parser.add_argument(
        "--vendor",
        default=os.environ.get("RUSTCLAW_NL_VENDOR", "unknown"),
        help="Vendor bucket to attach when run JSON does not expose one",
    )
    parser.add_argument(
        "--budget-profile",
        default=os.environ.get("RUSTCLAW_NL_BUDGET_PROFILE", "unknown"),
        help="Budget profile fallback when run JSON does not expose one",
    )
    parser.add_argument("--print-json", action="store_true", help="Also print JSON to stdout")
    parser.add_argument("--baseline", help="Existing rollout metrics JSON to compare against")
    parser.add_argument("--max-pass-rate-drop", type=float, default=0.02)
    parser.add_argument("--max-clarification-rate-rise", type=float, default=0.05)
    parser.add_argument("--max-verifier-block-rate", type=float, default=0.01)
    parser.add_argument("--max-avg-llm-calls-rise", type=float, default=0.15)
    parser.add_argument("--max-avg-elapsed-rise", type=float, default=0.20)
    args = parser.parse_args()

    run_dirs = [Path(run_dir).resolve() for run_dir in args.run_dirs]
    for run_dir in run_dirs:
        if not run_dir.is_dir():
            raise SystemExit(f"run dir not found: {run_dir}")
    output = Path(args.output) if args.output else default_output_path(run_dirs)
    output.parent.mkdir(parents=True, exist_ok=True)
    result = summarize_run_dirs(
        run_dirs,
        provider=args.provider.strip() or "unknown",
        vendor=args.vendor.strip() or "unknown",
        budget_profile=args.budget_profile.strip() or "unknown",
    )
    if args.baseline:
        baseline = load_json(Path(args.baseline))
        if baseline.get("_parse_error"):
            raise SystemExit(f"baseline parse error: {baseline['_parse_error']}")
        result["rollout_gate"] = compare_with_baseline(
            result,
            baseline,
            max_pass_rate_drop=args.max_pass_rate_drop,
            max_clarification_rate_rise=args.max_clarification_rate_rise,
            max_verifier_block_rate=args.max_verifier_block_rate,
            max_avg_llm_calls_rise=args.max_avg_llm_calls_rise,
            max_avg_elapsed_rise=args.max_avg_elapsed_rise,
        )
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    if args.print_json:
        print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    else:
        print(f"ROLLOUT_METRICS_OK output={output} turns={result['turns_total']} pass_rate={result['pass_rate']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
