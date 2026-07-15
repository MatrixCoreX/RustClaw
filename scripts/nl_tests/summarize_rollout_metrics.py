#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import tempfile
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]

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
    "stop_signal_counts",
    "guard_signal_counts",
    "round_owner_layer_counts",
    "execution_surface_owner_counts",
    "repair_signal_status_counts",
    "validation_status_counts",
    "lifecycle_state_counts",
    "event_type_counts",
    "checkpoint_kind_counts",
    "provider_blocker_counts",
    "agent_loop_eligibility_bucket_counts",
    "agent_loop_eligibility_blocked_reason_counts",
)
CLARIFICATION_FINAL_STATUS_KEYS = {"clarify", "clarification_requested"}
VERIFIER_BLOCK_KEYS = {"False", "false"}
CASE_FILE_RE = re.compile(r"^turn_(?P<turn>\d+)_case_(?P<case>\d+)\.json$")


def portable_path_ref(value: Path | str) -> str:
    text = str(value)
    if text in {"", "multiple", "external_path"}:
        return text
    try:
        resolved = Path(text).resolve()
    except OSError:
        return "external_path"
    try:
        return resolved.relative_to(ROOT).as_posix()
    except ValueError:
        return "external_path"


def rollout_metrics_ok_line(output: Path, result: dict[str, Any]) -> str:
    return (
        "ROLLOUT_METRICS_OK "
        f"output={portable_path_ref(output)} "
        f"turns={result['turns_total']} pass_rate={result['pass_rate']}"
    )


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


def route_legacy_first_layer(route: Any) -> str:
    if not isinstance(route, dict):
        return ""
    return str(
        route.get("legacy_first_layer_decision")
        or route.get("first_layer_decision")
        or ""
    )


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


def optional_threshold_result(
    passed: bool,
    observed: float,
    threshold: float,
) -> dict[str, Any]:
    return {
        "observed": observed,
        "threshold": threshold,
        "passed": passed,
    }


def compare_with_absolute_thresholds(
    result: dict[str, Any],
    min_pass_rate: float | None,
    max_avg_llm_calls: float | None,
    max_prompt_bytes_before: int | None,
    max_prompt_truncations: int | None,
    max_provider_final_errors: int | None,
    max_provider_retryable_errors: int | None,
    max_verifier_calls: int | None,
) -> dict[str, Any]:
    comparisons: dict[str, Any] = {}
    if min_pass_rate is not None:
        observed = float(result.get("pass_rate") or 0.0)
        comparisons["min_pass_rate"] = optional_threshold_result(
            observed >= min_pass_rate,
            observed,
            min_pass_rate,
        )
    if max_avg_llm_calls is not None:
        observed = float(get_path(result, "llm", "avg_calls_per_turn") or 0.0)
        comparisons["max_avg_llm_calls"] = optional_threshold_result(
            observed <= max_avg_llm_calls,
            observed,
            max_avg_llm_calls,
        )
    if max_prompt_bytes_before is not None:
        observed = safe_int(get_path(result, "llm", "prompt_bytes_before_max"))
        comparisons["max_prompt_bytes_before"] = optional_threshold_result(
            observed <= max_prompt_bytes_before,
            float(observed),
            float(max_prompt_bytes_before),
        )
    if max_prompt_truncations is not None:
        observed = safe_int(get_path(result, "llm", "prompt_truncation_count"))
        comparisons["max_prompt_truncations"] = optional_threshold_result(
            observed <= max_prompt_truncations,
            float(observed),
            float(max_prompt_truncations),
        )
    if max_provider_final_errors is not None:
        observed = safe_int(get_path(result, "llm", "provider_final_error_count"))
        comparisons["max_provider_final_errors"] = optional_threshold_result(
            observed <= max_provider_final_errors,
            float(observed),
            float(max_provider_final_errors),
        )
    if max_provider_retryable_errors is not None:
        observed = safe_int(get_path(result, "llm", "provider_retryable_error_count"))
        comparisons["max_provider_retryable_errors"] = optional_threshold_result(
            observed <= max_provider_retryable_errors,
            float(observed),
            float(max_provider_retryable_errors),
        )
    if max_verifier_calls is not None:
        observed = safe_int(get_path(result, "execution", "verifier_call_count"))
        comparisons["max_verifier_calls"] = optional_threshold_result(
            observed <= max_verifier_calls,
            float(observed),
            float(max_verifier_calls),
        )
    failures = [
        key for key, value in comparisons.items() if value.get("passed") is not True
    ]
    return {
        "configured": bool(comparisons),
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


def turn_case_id(path: Path) -> int | None:
    match = CASE_FILE_RE.match(path.name)
    if not match:
        return None
    return int(match.group("case"))


def turn_file_order(path: Path, run_order: dict[Path, int]) -> tuple[int, str, int, str]:
    match = CASE_FILE_RE.match(path.name)
    turn_number = int(match.group("turn")) if match else 0
    parent = path.parent.resolve()
    return (
        run_order.get(parent, -1),
        parent.name,
        turn_number,
        path.name,
    )


def latest_valid_case_paths(run_dirs: list[Path]) -> tuple[list[Path], dict[str, Any]]:
    run_order = {run_dir.resolve(): index for index, run_dir in enumerate(run_dirs)}
    latest: dict[int, tuple[tuple[int, str, int, str], Path]] = {}
    skipped_parse_errors: list[dict[str, str]] = []
    ignored_without_case_id = 0
    for run_dir in run_dirs:
        for path in turn_json_paths(run_dir):
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
        "selected_run_dir_counts": dict(sorted(selected_run_counts.items())),
        "skipped_parse_error_count": len(skipped_parse_errors),
        "skipped_parse_error_examples": skipped_parse_errors[:10],
        "ignored_without_case_id": ignored_without_case_id,
    }


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


def task_observations(trace: dict[str, Any]) -> list[dict[str, Any]]:
    observations = trace.get("task_observations")
    if not isinstance(observations, list):
        return []
    return [item for item in observations if isinstance(item, dict)]


def trace_event_stream(trace: dict[str, Any]) -> list[dict[str, Any]]:
    events = trace.get("event_stream")
    if not isinstance(events, list):
        return []
    return [event for event in events if isinstance(event, dict)]


def journal_rollout_items(summary: dict[str, Any], trace: dict[str, Any]) -> list[dict[str, Any]]:
    items: list[dict[str, Any]] = []
    seen: set[str] = set()
    for source in (summary.get("rollout_attribution"), trace.get("rollout_attribution")):
        if not isinstance(source, list):
            continue
        for item in source:
            if not isinstance(item, dict):
                continue
            fingerprint = json.dumps(item, sort_keys=True, ensure_ascii=True)
            if fingerprint in seen:
                continue
            seen.add(fingerprint)
            items.append(item)
    return items


def first_trace_round_value(trace: dict[str, Any], key: str) -> str:
    for round_item in trace_rounds(trace):
        value = round_item.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return ""


def detected_budget_profile(
    summary: dict[str, Any],
    trace: dict[str, Any],
    fallback: str,
) -> str:
    round_budget = first_trace_round_value(trace, "budget_profile")
    if round_budget:
        return round_budget
    for item in journal_rollout_items(summary, trace):
        value = item.get("budget_profile")
        if isinstance(value, str) and value.strip():
            return value.strip()
    context_summary = summary.get("context_bundle_summary")
    return (
        machine_token_value(context_summary, "context_profile")
        or machine_token_value(context_summary, "budget_profile")
        or machine_token_value(context_summary, "execution_budget")
        or machine_token_value(context_summary, "route_budget")
        or fallback
    )


def step_output_json(step: dict[str, Any]) -> dict[str, Any]:
    output = step.get("output_excerpt")
    if not isinstance(output, str) or not output.strip():
        return {}
    try:
        value = json.loads(output)
    except Exception:
        return {}
    return value if isinstance(value, dict) else {}


def planner_first_action(trace: dict[str, Any]) -> str:
    for round_item in trace_rounds(trace):
        for key in ("first_action_capability_ref", "first_action_decision"):
            value = round_item.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
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


BACKGROUND_LIFECYCLE_STATES = {
    "background",
    "waiting",
    "needs_user",
    "provider_wait",
    "background_wait",
}

PROVIDER_BLOCKER_HINTS = {
    "provider_error",
    "provider_unavailable",
    "provider_timeout",
    "provider_rate_limited",
    "provider_quota_exceeded",
    "rate_limited",
    "quota_exceeded",
    "model_provider_error",
    "model_timeout",
}


def lifecycle_state(data: dict[str, Any], result: dict[str, Any], summary: dict[str, Any]) -> str:
    for source in (
        data.get("task_lifecycle"),
        result.get("task_lifecycle"),
        summary.get("task_lifecycle"),
        summary.get("lifecycle"),
    ):
        if isinstance(source, dict):
            state = source.get("state") or source.get("lifecycle_state")
            if isinstance(state, str) and state.strip():
                return state.strip()
    for key in ("lifecycle_state", "execution_state"):
        value = data.get(key) or result.get(key) or summary.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return "unknown"


def checkpoint_kind_from_event(event: dict[str, Any]) -> str:
    payload = event.get("payload")
    if not isinstance(payload, dict):
        payload = {}
    for source in (payload, event):
        kind = source.get("checkpoint_kind")
        if isinstance(kind, str) and kind.strip():
            return kind.strip()
    event_type = event.get("event_type")
    if isinstance(event_type, str) and "checkpoint" in event_type:
        return event_type.strip()
    return ""


def verifier_call_count(trace: dict[str, Any], summary: dict[str, Any]) -> int:
    count = sum(
        1
        for round_item in trace_rounds(trace)
        if isinstance(round_item.get("verify_result"), dict)
    )
    if isinstance(summary.get("answer_verifier_summary"), dict):
        count += 1
    return count


def provider_blocker_token(value: Any) -> str:
    if not isinstance(value, str):
        return ""
    token = value.strip()
    if not token:
        return ""
    lowered = token.lower()
    if lowered in PROVIDER_BLOCKER_HINTS:
        return lowered
    if "provider" in lowered or "rate_limit" in lowered or "quota" in lowered:
        return lowered
    return ""


REPEAT_GUARD_SIGNALS = {
    "repeat_completed_action",
    "repeat_action_limit",
    "registry_idempotency_repeat_completed_action",
    "registry_idempotency_repeat_action_limit",
}
GUARD_SIGNALS = REPEAT_GUARD_SIGNALS | {
    "max_tool_calls",
    "max_rounds",
    "multi_round_disabled",
    "no_actions",
    "no_progress",
}


def normalized_signal(value: Any) -> str:
    return value.strip() if isinstance(value, str) and value.strip() else ""


def guard_signal(value: Any) -> str:
    signal = normalized_signal(value)
    if signal in GUARD_SIGNALS:
        return signal
    return ""


def repeat_action_guard_count(counts: Counter[str] | dict[str, int]) -> int:
    return sum(safe_int(counts.get(signal)) for signal in REPEAT_GUARD_SIGNALS)


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
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "input_tokens",
        "output_tokens",
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
    paths: list[Path] | None = None,
    attribution_run_dirs: list[Path] | None = None,
    source_run_dir: str | None = None,
) -> dict[str, Any]:
    paths = turn_json_paths(run_dir) if paths is None else sorted(paths)
    status_counts: Counter[str] = Counter()
    final_status_counts: Counter[str] = Counter()
    first_layer_counts: Counter[str] = Counter()
    route_gate_counts: Counter[str] = Counter()
    failure_attribution_counts: Counter[str] = Counter()
    verifier_pass_counts: Counter[str] = Counter()
    rollout_switch_counts: Counter[str] = Counter()
    rollout_event_counts: Counter[str] = Counter()
    rollout_reason_counts: Counter[str] = Counter()
    stop_signal_counts: Counter[str] = Counter()
    guard_signal_counts: Counter[str] = Counter()
    round_owner_layer_counts: Counter[str] = Counter()
    execution_surface_owner_counts: Counter[str] = Counter()
    repair_signal_status_counts: Counter[str] = Counter()
    validation_status_counts: Counter[str] = Counter()
    lifecycle_state_counts: Counter[str] = Counter()
    event_type_counts: Counter[str] = Counter()
    checkpoint_kind_counts: Counter[str] = Counter()
    provider_blocker_counts: Counter[str] = Counter()
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
    agent_loop_eligibility_bucket_counts: Counter[str] = Counter()
    agent_loop_eligibility_blocked_reason_counts: Counter[str] = Counter()
    parse_errors = 0
    total_llm_calls = 0
    total_llm_elapsed_ms = 0
    total_prompt_truncations = 0
    total_rounds = 0
    total_steps = 0
    total_tool_calls = 0
    total_external_tool_calls = 0
    total_verifier_calls = 0
    total_background_states = 0
    total_checkpoint_events = 0
    total_provider_blockers = 0

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
        state = lifecycle_state(data, result, summary)
        lifecycle_state_counts[state] += 1
        if state in BACKGROUND_LIFECYCLE_STATES:
            total_background_states += 1
        final_status_counts[str(summary.get("final_status") or "unknown")] += 1
        first_layer_counts[route_legacy_first_layer(route) or "unknown"] += 1
        route_gate_counts[str(route.get("route_gate_kind") or "unknown")] += 1
        failure_attribution_counts[str(summary.get("final_failure_attribution") or "none")] += 1
        if blocker := provider_blocker_token(summary.get("final_failure_attribution")):
            provider_blocker_counts[blocker] += 1
            total_provider_blockers += 1
        if verifier:
            verifier_pass_counts[str(verifier.get("pass"))] += 1
        else:
            verifier_pass_counts["missing"] += 1
        total_verifier_calls += verifier_call_count(trace, summary)
        final_stop_signal = normalized_signal(summary.get("final_stop_signal")) or normalized_signal(
            trace.get("final_stop_signal")
        )
        if final_stop_signal:
            stop_signal_counts[final_stop_signal] += 1
            if signal := guard_signal(final_stop_signal):
                guard_signal_counts[signal] += 1
        provider_counts[provider] += 1
        vendor_counts[vendor] += 1
        detected_budget = detected_budget_profile(summary, trace, budget_profile)
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
        for round_item in trace_rounds(trace):
            round_owner_layer_counts[
                str(round_item.get("owner_layer") or "not_recorded")
            ] += 1
            stop_signal = normalized_signal(round_item.get("stop_signal"))
            if stop_signal:
                stop_signal_counts[stop_signal] += 1
                if signal := guard_signal(stop_signal):
                    guard_signal_counts[signal] += 1
                if blocker := provider_blocker_token(stop_signal):
                    provider_blocker_counts[blocker] += 1
                    total_provider_blockers += 1
            for signal in round_item.get("repair_signals") or []:
                if isinstance(signal, dict):
                    repair_signal_status_counts[
                        str(signal.get("status_code") or "not_recorded")
                    ] += 1

        validation_result = dict_value(summary.get("validation_result")) or dict_value(
            trace.get("validation_result")
        )
        validation_status_counts[
            str(validation_result.get("latest_status") or "not_recorded")
        ] += 1

        for item in journal_rollout_items(summary, trace):
            event = item.get("event")
            reason = item.get("reason_code")
            if isinstance(event, str) and event.strip():
                rollout_event_counts[event.strip()] += 1
            if isinstance(reason, str) and reason.strip():
                rollout_reason_counts[reason.strip()] += 1
                if signal := guard_signal(reason):
                    guard_signal_counts[signal] += 1
                if blocker := provider_blocker_token(reason):
                    provider_blocker_counts[blocker] += 1
                    total_provider_blockers += 1
            boundary_context = dict_value(item.get("boundary_context"))
            boundary_budget = dict_value(boundary_context.get("budget"))
            agent_loop_eligibility_bucket_counts[
                str(boundary_budget.get("agent_loop_eligibility_bucket") or "unknown")
            ] += 1
            agent_loop_eligibility_blocked_reason_counts[
                str(
                    boundary_budget.get("agent_loop_eligibility_blocked_reason")
                    or "unknown"
                )
            ] += 1

        steps = step_results(trace)
        for step in steps:
            capability = (
                step.get("resolved_capability")
                or step.get("requested_capability")
                or step.get("resolved_tool_or_skill")
                or step.get("skill")
            )
            if isinstance(capability, str) and capability.strip():
                capability_counts[capability.strip()] += 1
            owner = step.get("execution_surface_owner") or step_output_json(step).get(
                "execution_surface_owner"
            )
            if isinstance(owner, str) and owner.strip():
                execution_surface_owner_counts[owner.strip()] += 1
            for key in ("error_kind", "failure_attribution"):
                if signal := guard_signal(step.get(key)):
                    guard_signal_counts[signal] += 1
                if blocker := provider_blocker_token(step.get(key)):
                    provider_blocker_counts[blocker] += 1
                    total_provider_blockers += 1
        for observation in task_observations(trace):
            owner = observation.get("execution_surface_owner")
            if isinstance(owner, str) and owner.strip():
                execution_surface_owner_counts[owner.strip()] += 1
        for event in trace_event_stream(trace):
            event_type = str(event.get("event_type") or "unknown")
            event_type_counts[event_type] += 1
            checkpoint_kind = checkpoint_kind_from_event(event)
            if checkpoint_kind:
                checkpoint_kind_counts[checkpoint_kind] += 1
                total_checkpoint_events += 1
        tool_calls, external_tool_calls = count_tool_calls(steps)
        total_tool_calls += tool_calls
        total_external_tool_calls += external_tool_calls

    total_turns = len(paths)
    succeeded = status_counts.get("succeeded", 0)
    attribution_counts: Counter[str] = Counter()
    for attribution_run_dir in attribution_run_dirs or [run_dir]:
        attribution_counts.update(load_attribution_counts(attribution_run_dir))
    by_prompt = dict(sorted(by_prompt_totals.items()))
    provider_attempt_count = sum_by_prompt_field(by_prompt, "provider_attempt_count")
    provider_retry_count = sum_by_prompt_field(by_prompt, "provider_retry_count")
    provider_retryable_error_count = sum_by_prompt_field(by_prompt, "provider_retryable_error_count")
    provider_final_error_count = sum_by_prompt_field(by_prompt, "provider_final_error_count")
    prompt_truncated_bytes_total = sum_by_prompt_field(by_prompt, "prompt_truncated_bytes_total")
    prompt_tokens = sum_by_prompt_field(by_prompt, "prompt_tokens")
    completion_tokens = sum_by_prompt_field(by_prompt, "completion_tokens")
    total_tokens = sum_by_prompt_field(by_prompt, "total_tokens")
    input_tokens = sum_by_prompt_field(by_prompt, "input_tokens")
    output_tokens = sum_by_prompt_field(by_prompt, "output_tokens")
    prompt_bytes_before_values = [
        safe_int(bucket.get("prompt_bytes_before_max"))
        for bucket in by_prompt.values()
        if isinstance(bucket, dict)
    ]
    prompt_bytes_after_values = [
        safe_int(bucket.get("prompt_bytes_after_max"))
        for bucket in by_prompt.values()
        if isinstance(bucket, dict)
    ]
    prompt_bytes_budget_values = [
        safe_int(bucket.get("prompt_bytes_budget_min"))
        for bucket in by_prompt.values()
        if isinstance(bucket, dict) and safe_int(bucket.get("prompt_bytes_budget_min")) > 0
    ]
    return {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source_run_dir": portable_path_ref(source_run_dir or run_dir),
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
        "stop_signal_counts": dict(sorted(stop_signal_counts.items())),
        "guard_signal_counts": dict(sorted(guard_signal_counts.items())),
        "round_owner_layer_counts": dict(sorted(round_owner_layer_counts.items())),
        "execution_surface_owner_counts": dict(
            sorted(execution_surface_owner_counts.items())
        ),
        "repair_signal_status_counts": dict(sorted(repair_signal_status_counts.items())),
        "validation_status_counts": dict(sorted(validation_status_counts.items())),
        "lifecycle_state_counts": dict(sorted(lifecycle_state_counts.items())),
        "event_type_counts": dict(sorted(event_type_counts.items())),
        "checkpoint_kind_counts": dict(sorted(checkpoint_kind_counts.items())),
        "provider_blocker_counts": dict(sorted(provider_blocker_counts.items())),
        "agent_loop_eligibility_bucket_counts": dict(
            sorted(agent_loop_eligibility_bucket_counts.items())
        ),
        "agent_loop_eligibility_blocked_reason_counts": dict(
            sorted(agent_loop_eligibility_blocked_reason_counts.items())
        ),
        "llm": {
            "total_calls": total_llm_calls,
            "total_elapsed_ms": total_llm_elapsed_ms,
            "provider_attempt_count": provider_attempt_count,
            "provider_retry_count": provider_retry_count,
            "provider_retryable_error_count": provider_retryable_error_count,
            "provider_final_error_count": provider_final_error_count,
            "prompt_bytes_before_max": max(prompt_bytes_before_values, default=0),
            "prompt_bytes_after_max": max(prompt_bytes_after_values, default=0),
            "prompt_bytes_budget_min": min(prompt_bytes_budget_values, default=0),
            "prompt_truncated_bytes_total": prompt_truncated_bytes_total,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
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
            "verifier_call_count": total_verifier_calls,
            "background_state_count": total_background_states,
            "checkpoint_event_count": total_checkpoint_events,
            "provider_blocker_count": total_provider_blockers,
            "repeat_action_guard_hit_count": repeat_action_guard_count(guard_signal_counts),
            "no_progress_stop_count": safe_int(guard_signal_counts.get("no_progress")),
        },
    }


def summarize_run_dirs(
    run_dirs: list[Path],
    provider: str = "unknown",
    vendor: str = "unknown",
    budget_profile: str = "unknown",
    dedupe_latest_case: bool = False,
) -> dict[str, Any]:
    if dedupe_latest_case:
        selected_paths, case_dedupe = latest_valid_case_paths(run_dirs)
        unique_run_dirs = sorted({path.parent for path in selected_paths})
        result = summarize_run(
            run_dirs[0],
            provider=provider,
            vendor=vendor,
            budget_profile=budget_profile,
            paths=selected_paths,
            attribution_run_dirs=unique_run_dirs,
            source_run_dir="multiple",
        )
        result["source_run_dirs"] = [portable_path_ref(run_dir) for run_dir in run_dirs]
        result["case_dedupe"] = case_dedupe
        return result

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
    prompt_truncated_bytes_total = sum(
        safe_int(get_path(summary, "llm", "prompt_truncated_bytes_total"))
        for summary in summaries
    )
    prompt_tokens = sum(
        safe_int(get_path(summary, "llm", "prompt_tokens")) for summary in summaries
    )
    completion_tokens = sum(
        safe_int(get_path(summary, "llm", "completion_tokens")) for summary in summaries
    )
    total_tokens = sum(
        safe_int(get_path(summary, "llm", "total_tokens")) for summary in summaries
    )
    input_tokens = sum(
        safe_int(get_path(summary, "llm", "input_tokens")) for summary in summaries
    )
    output_tokens = sum(
        safe_int(get_path(summary, "llm", "output_tokens")) for summary in summaries
    )
    prompt_bytes_before_max = max(
        (safe_int(get_path(summary, "llm", "prompt_bytes_before_max")) for summary in summaries),
        default=0,
    )
    prompt_bytes_after_max = max(
        (safe_int(get_path(summary, "llm", "prompt_bytes_after_max")) for summary in summaries),
        default=0,
    )
    prompt_bytes_budget_values = [
        safe_int(get_path(summary, "llm", "prompt_bytes_budget_min"))
        for summary in summaries
        if safe_int(get_path(summary, "llm", "prompt_bytes_budget_min")) > 0
    ]
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
    total_verifier_calls = sum(
        safe_int(get_path(summary, "execution", "verifier_call_count"))
        for summary in summaries
    )
    total_background_states = sum(
        safe_int(get_path(summary, "execution", "background_state_count"))
        for summary in summaries
    )
    total_checkpoint_events = sum(
        safe_int(get_path(summary, "execution", "checkpoint_event_count"))
        for summary in summaries
    )
    total_provider_blockers = sum(
        safe_int(get_path(summary, "execution", "provider_blocker_count"))
        for summary in summaries
    )
    repeat_action_guard_hit_count = sum(
        safe_int(get_path(summary, "execution", "repeat_action_guard_hit_count"))
        for summary in summaries
    )
    no_progress_stop_count = sum(
        safe_int(get_path(summary, "execution", "no_progress_stop_count"))
        for summary in summaries
    )
    status_counts = merge_counter_field(summaries, "status_counts")
    by_prompt = merge_by_prompt(summaries)
    result: dict[str, Any] = {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source_run_dir": "multiple",
        "source_run_dirs": [portable_path_ref(run_dir) for run_dir in run_dirs],
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
            "prompt_bytes_before_max": prompt_bytes_before_max,
            "prompt_bytes_after_max": prompt_bytes_after_max,
            "prompt_bytes_budget_min": min(prompt_bytes_budget_values, default=0),
            "prompt_truncated_bytes_total": prompt_truncated_bytes_total,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
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
            "verifier_call_count": total_verifier_calls,
            "background_state_count": total_background_states,
            "checkpoint_event_count": total_checkpoint_events,
            "provider_blocker_count": total_provider_blockers,
            "repeat_action_guard_hit_count": repeat_action_guard_hit_count,
            "no_progress_stop_count": no_progress_stop_count,
        },
    }
    for key in COUNTER_FIELDS:
        result[key] = status_counts if key == "status_counts" else merge_counter_field(summaries, key)
    return result


def default_output_path(run_dirs: list[Path], dedupe_latest_case: bool = False) -> Path:
    if len(run_dirs) == 1:
        name = f"{run_dirs[0].name}_rollout_metrics.json"
    else:
        name = f"multi_{len(run_dirs)}_{run_dirs[0].name}_to_{run_dirs[-1].name}_rollout_metrics.json"
    if dedupe_latest_case:
        name = name.removesuffix(".json") + "_dedupe_latest_case.json"
    return Path("logs/agent_rollout_metrics") / name


def run_self_test() -> int:
    fixture_ref = portable_path_ref(
        ROOT / "scripts/nl_tests/fixtures/client_like_runs/coding_loop_repair"
    )
    if fixture_ref.startswith("/") or "rustclaw" in fixture_ref.split("/")[:1]:
        print(f"SELF_TEST_FAIL fixture_ref:{fixture_ref}")
        return 1
    with tempfile.TemporaryDirectory(prefix="rollout-metrics-") as tmp:
        external_ref = portable_path_ref(Path(tmp) / "run")
        if external_ref != "external_path":
            print(f"SELF_TEST_FAIL external_ref:{external_ref}")
            return 1
        line = rollout_metrics_ok_line(
            Path(tmp) / "metrics.json",
            {"turns_total": 1, "pass_rate": 1.0},
        )
        if tmp in line or "/tmp/" in line or "output=external_path" not in line:
            print(f"SELF_TEST_FAIL ok_line:{line}")
            return 1
    if portable_path_ref("multiple") != "multiple":
        print("SELF_TEST_FAIL multiple_ref")
        return 1
    print("ROLLOUT_METRICS_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Summarize client-like NL rollout metrics into stable JSON."
    )
    parser.add_argument("run_dirs", nargs="*", help="Client-like NL run directories")
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
    parser.add_argument(
        "--min-pass-rate",
        type=float,
        help="Absolute gate: fail when pass_rate is below this value.",
    )
    parser.add_argument(
        "--max-avg-llm-calls",
        type=float,
        help="Absolute gate: fail when average LLM calls per turn exceeds this value.",
    )
    parser.add_argument(
        "--max-prompt-bytes-before",
        type=int,
        help="Absolute gate: fail when max prompt_bytes_before exceeds this value.",
    )
    parser.add_argument(
        "--max-prompt-truncations",
        type=int,
        help="Absolute gate: fail when prompt truncation count exceeds this value.",
    )
    parser.add_argument(
        "--max-provider-final-errors",
        type=int,
        help="Absolute gate: fail when provider final error count exceeds this value.",
    )
    parser.add_argument(
        "--max-provider-retryable-errors",
        type=int,
        help="Absolute gate: fail when provider retryable error count exceeds this value.",
    )
    parser.add_argument(
        "--max-verifier-calls",
        type=int,
        help="Absolute gate: fail when verifier-call count exceeds this value.",
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
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()
    if not args.run_dirs:
        parser.error("at least one run_dir is required unless --self-test is used")

    run_dirs = [Path(run_dir).resolve() for run_dir in args.run_dirs]
    for run_dir in run_dirs:
        if not run_dir.is_dir():
            raise SystemExit(f"run dir not found: {run_dir}")
    output = (
        Path(args.output)
        if args.output
        else default_output_path(run_dirs, dedupe_latest_case=args.dedupe_latest_case)
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    result = summarize_run_dirs(
        run_dirs,
        provider=args.provider.strip() or "unknown",
        vendor=args.vendor.strip() or "unknown",
        budget_profile=args.budget_profile.strip() or "unknown",
        dedupe_latest_case=args.dedupe_latest_case,
    )
    if args.expect_case_count:
        case_count = safe_int(get_path(result, "case_dedupe", "case_count"))
        if case_count < args.expect_case_count:
            raise SystemExit(
                f"deduped case count {case_count} below expected {args.expect_case_count}"
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
    result["metric_gate"] = compare_with_absolute_thresholds(
        result,
        min_pass_rate=args.min_pass_rate,
        max_avg_llm_calls=args.max_avg_llm_calls,
        max_prompt_bytes_before=args.max_prompt_bytes_before,
        max_prompt_truncations=args.max_prompt_truncations,
        max_provider_final_errors=args.max_provider_final_errors,
        max_provider_retryable_errors=args.max_provider_retryable_errors,
        max_verifier_calls=args.max_verifier_calls,
    )
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    if args.print_json:
        print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    else:
        print(rollout_metrics_ok_line(output, result))
    if result["metric_gate"]["configured"] and not result["metric_gate"]["passed"]:
        print(
            "ROLLOUT_METRICS_GATE_FAIL "
            f"failures={','.join(result['metric_gate']['failures'])}",
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
