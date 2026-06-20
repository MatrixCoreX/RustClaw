#!/usr/bin/env python3
"""Summarize client-like NL run logs into an auditable execution trace.

The input is a directory produced by run_client_like_continuous_suite.sh.
This script is intentionally offline: it does not call clawd or an LLM.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


CASE_RE = re.compile(r"turn_(?P<turn>\d+)_case_(?P<case>\d+)\.json$")


def get_path(obj: Any, *path: str) -> Any:
    cur = obj
    for key in path:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def compact(value: Any, max_chars: int) -> str:
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


def sort_key(path: Path) -> tuple[int, int, str]:
    match = CASE_RE.search(path.name)
    if not match:
        return (10**9, 10**9, path.name)
    return (int(match.group("case")), int(match.group("turn")), path.name)


def iter_case_files(run_dir: Path) -> list[Path]:
    return sorted(run_dir.glob("turn_*_case_*.json"), key=sort_key)


def extract_prompt(obj: dict[str, Any]) -> str:
    return compact(
        get_path(obj, "data", "result_json", "task_journal", "summary", "input_text")
        or get_path(obj, "data", "input_text")
        or get_path(obj, "input_text")
        or "",
        2000,
    )


def extract_final_reply(obj: dict[str, Any], max_chars: int) -> str:
    result = get_path(obj, "data", "result_json") or {}
    messages = result.get("messages") if isinstance(result, dict) else None
    if isinstance(messages, list) and messages:
        user_visible = [m for m in messages if isinstance(m, str)]
        if user_visible:
            return compact("\n\n".join(user_visible), max_chars)
    return compact(result.get("text") if isinstance(result, dict) else None, max_chars)


def extract_route(obj: dict[str, Any]) -> dict[str, Any]:
    return (
        get_path(obj, "data", "result_json", "task_journal", "summary", "route_result")
        or {}
    )


def route_legacy_first_layer(route: Any) -> str:
    if not isinstance(route, dict):
        return ""
    return str(
        route.get("legacy_first_layer_decision")
        or route.get("first_layer_decision")
        or ""
    )


def extract_trace(obj: dict[str, Any]) -> dict[str, Any]:
    return (
        get_path(obj, "data", "result_json", "task_journal", "trace")
        or {}
    )


def summarize_steps(rounds: Any) -> list[str]:
    out: list[str] = []
    if not isinstance(rounds, list):
        return out
    for round_obj in rounds:
        round_no = round_obj.get("round_no", "?") if isinstance(round_obj, dict) else "?"
        plan = round_obj.get("plan_result") if isinstance(round_obj, dict) else None
        steps = plan.get("steps") if isinstance(plan, dict) else None
        if not isinstance(steps, list):
            continue
        for step in steps:
            if not isinstance(step, dict):
                continue
            action = step.get("action_type") or step.get("type") or "?"
            skill = step.get("skill") or step.get("tool") or step.get("capability") or ""
            step_id = step.get("step_id") or "?"
            out.append(f"round={round_no} step={step_id} action={action} target={skill}")
    return out


def summarize_verifier(rounds: Any, max_chars: int) -> list[str]:
    out: list[str] = []
    if not isinstance(rounds, list):
        return out
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        verify = round_obj.get("verify_result")
        if not isinstance(verify, dict):
            continue
        approved = verify.get("approved")
        mode = verify.get("mode")
        issues = verify.get("issues") or []
        blocked = verify.get("blocked_reason")
        out.append(
            "round={round_no} approved={approved} mode={mode} blocked={blocked} issues={issues}".format(
                round_no=round_obj.get("round_no", "?"),
                approved=approved,
                mode=mode,
                blocked=compact(blocked, max_chars),
                issues=compact(issues, max_chars),
            )
        )
    return out


def summarize_execution(trace: dict[str, Any], max_chars: int) -> list[str]:
    out: list[str] = []
    step_results = trace.get("step_results")
    if not isinstance(step_results, list):
        return out
    for result in step_results:
        if not isinstance(result, dict):
            continue
        requested = result.get("requested_capability") or result.get("requested_action_type") or ""
        executed = result.get("executed_skill") or result.get("skill") or ""
        status = result.get("status") or ""
        output = result.get("output_excerpt") or result.get("error_excerpt")
        out.append(
            "step={step_id} requested={requested} executed={executed} status={status} output={output}".format(
                step_id=result.get("step_id", "?"),
                requested=requested,
                executed=executed,
                status=status,
                output=compact(output, max_chars),
            )
        )
    return out


def summarize_metrics(obj: dict[str, Any]) -> str:
    metrics = (
        get_path(obj, "data", "result_json", "task_journal", "summary", "task_metrics")
        or {}
    )
    if not isinstance(metrics, dict):
        return ""
    return "llm_calls={calls} llm_elapsed_ms={elapsed} rounds={rounds} prompt_truncations={trunc}".format(
        calls=metrics.get("llm_calls_per_task", ""),
        elapsed=metrics.get("llm_elapsed_ms_per_task", ""),
        rounds=get_path(obj, "data", "result_json", "task_journal", "summary", "round_count") or "",
        trunc=metrics.get("prompt_truncation_count", ""),
    )


def print_case(path: Path, obj: dict[str, Any], max_chars: int) -> None:
    match = CASE_RE.search(path.name)
    case_id = match.group("case") if match else path.stem
    turn_id = match.group("turn") if match else "?"
    status = get_path(obj, "data", "status") or get_path(obj, "status") or ""
    route = extract_route(obj)
    trace = extract_trace(obj)
    rounds = trace.get("rounds")

    print(f"CASE {case_id} turn={turn_id} status={status}")
    print(f"PROMPT: {extract_prompt(obj)}")
    print(
        "ROUTE: gate={gate} routed_mode={mode} legacy_first_layer={first_layer} reason={reason}".format(
            gate=route.get("route_gate_kind", ""),
            mode=route.get("routed_mode", ""),
            first_layer=route_legacy_first_layer(route),
            reason=compact(route.get("route_reason"), max_chars),
        )
    )
    print("PLANNER_STEPS:")
    for line in summarize_steps(rounds) or ["(none)"]:
        print(f"  - {line}")
    print("VERIFIER:")
    for line in summarize_verifier(rounds, max_chars) or ["(none)"]:
        print(f"  - {line}")
    print("EXECUTION:")
    for line in summarize_execution(trace, max_chars) or ["(none)"]:
        print(f"  - {line}")
    metrics = summarize_metrics(obj)
    if metrics:
        print(f"METRICS: {metrics}")
    print(f"FINAL_REPLY: {extract_final_reply(obj, max_chars)}")
    print()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path, help="client-like continuous run log directory")
    parser.add_argument("--limit", type=int, default=0, help="maximum cases to print")
    parser.add_argument("--max-chars", type=int, default=1200, help="max chars per long field")
    args = parser.parse_args()

    if not args.run_dir.is_dir():
        raise SystemExit(f"Not a directory: {args.run_dir}")

    files = iter_case_files(args.run_dir)
    if args.limit > 0:
        files = files[: args.limit]
    if not files:
        raise SystemExit(f"No turn_*_case_*.json files found under {args.run_dir}")

    for path in files:
        try:
            obj = json.loads(path.read_text(encoding="utf-8"))
        except Exception as exc:  # noqa: BLE001 - diagnostic script
            print(f"CASE {path.name} parse_error={exc}")
            continue
        if isinstance(obj, dict):
            print_case(path, obj, args.max_chars)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
