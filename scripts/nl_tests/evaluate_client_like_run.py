#!/usr/bin/env python3
"""Evaluate client-like NL run logs against lightweight expectations.

This is an offline regression helper for runs produced by
run_client_like_continuous_suite.sh. It does not call clawd or any model.

Expectation JSONL rows are intentionally small and optional. Supported fields:

  {
    "case": 12,
    "status": "succeeded",
    "first_layer": "planner_execute",
    "first_layer_any": ["direct_answer", "planner_execute"],
    "routed_mode": "Act",
    "route_gate_any": ["chat", "execute"],
    "capability_any": ["filesystem.list_entries", "fs_basic"],
    "executed_any": ["fs_basic"],
    "verifier_approved": true,
    "verifier_issue_any": ["MissingRequiredArg"],
    "needs_confirmation": false,
    "final_contains": ["README.md"],
    "final_shape": "path|file_token|integer|list|non_empty|empty",
    "finalizer_stage": "observed_generic",
    "finalizer_fallback": "raw_text",
    "finalizer_grounded_ok": true
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


@dataclass
class Observation:
    case: int
    turn: int
    file: str
    prompt: str
    status: str
    first_layer: str
    routed_mode: str
    route_gate: str
    plan_targets: list[str]
    executed: list[str]
    verifier_approvals: list[bool]
    verifier_issue_kinds: list[str]
    verifier_needs_confirmation: list[bool]
    finalizer_stage: str
    finalizer_fallback: str
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
    text = final_text(result if isinstance(result, dict) else {}, summary if isinstance(summary, dict) else {})
    return Observation(
        case=case_id,
        turn=turn_id,
        file=path.name,
        prompt=compact(summary.get("input_text") if isinstance(summary, dict) else "", 1000),
        status=str(get_path(obj, "data", "status") or ""),
        first_layer=str(route.get("first_layer_decision") or "") if isinstance(route, dict) else "",
        routed_mode=str(route.get("routed_mode") or "") if isinstance(route, dict) else "",
        route_gate=str(route.get("route_gate_kind") or "") if isinstance(route, dict) else "",
        plan_targets=collect_plan_targets(trace if isinstance(trace, dict) else {}),
        executed=collect_executed(trace if isinstance(trace, dict) else {}),
        verifier_approvals=collect_verifier_approved(trace if isinstance(trace, dict) else {}),
        verifier_issue_kinds=collect_verifier_issue_kinds(trace if isinstance(trace, dict) else {}),
        verifier_needs_confirmation=collect_verifier_needs_confirmation(trace if isinstance(trace, dict) else {}),
        finalizer_stage=str(finalizer.get("stage") or "") if isinstance(finalizer, dict) else "",
        finalizer_fallback=str(finalizer.get("fallback") or "") if isinstance(finalizer, dict) else "",
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
        "first_layer": obs.first_layer,
        "routed_mode": obs.routed_mode,
        "route_gate": obs.route_gate,
        "final_shape": obs.final_shape,
        "finalizer_stage": obs.finalizer_stage,
        "finalizer_fallback": obs.finalizer_fallback,
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
    if "executed_any" in expected and not any_expected_present(expected["executed_any"], obs.executed):
        failures.append(f"executed_any: expected one of {expected['executed_any']!r}, got {obs.executed!r}")
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
    if "finalizer_used_evidence_ids_min" in expected:
        wanted = int(expected["finalizer_used_evidence_ids_min"])
        observed = obs.finalizer_used_evidence_ids_count
        if observed is None or observed < wanted:
            failures.append(
                f"finalizer_used_evidence_ids_min: expected >= {wanted}, got {observed!r}"
            )
    if "final_contains" in expected:
        values = expected["final_contains"]
        values = values if isinstance(values, list) else [values]
        for value in values:
            if str(value) not in obs.final_text:
                failures.append(f"final_contains: missing {value!r}")
    return failures


def baseline_row(obs: Observation) -> dict[str, Any]:
    return {
        "case": obs.case,
        "turn": obs.turn,
        "prompt": obs.prompt,
        "status": obs.status,
        "first_layer": obs.first_layer,
        "routed_mode": obs.routed_mode,
        "route_gate": obs.route_gate,
        "plan_targets": obs.plan_targets,
        "executed": obs.executed,
        "verifier_approvals": obs.verifier_approvals,
        "verifier_issue_kinds": obs.verifier_issue_kinds,
        "verifier_needs_confirmation": obs.verifier_needs_confirmation,
        "finalizer_stage": obs.finalizer_stage,
        "finalizer_fallback": obs.finalizer_fallback,
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
