#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path


ATTRIBUTION_LABELS = {
    "pass",
    "normalizer",
    "route_miss",
    "planner",
    "planner_bypass_without_evidence",
    "plan_missing_target",
    "skill_schema",
    "skill_output",
    "finalizer",
    "finalizer_overwrite",
    "verifier",
    "evidence_missing_field",
    "verifier_should_retry_not_applied",
    "runtime_bug",
}


def load_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception as err:
        return {
            "ok": False,
            "data": {"status": "parse_error", "error_text": str(err)},
            "error": str(err),
        }


def find_step_results(trace: dict) -> list[dict]:
    direct = trace.get("step_results")
    if isinstance(direct, list):
        return [item for item in direct if isinstance(item, dict)]
    return []


def find_plan_steps(trace: dict) -> list[dict]:
    steps: list[dict] = []
    for round_item in trace.get("rounds") or []:
        if not isinstance(round_item, dict):
            continue
        plan = round_item.get("plan_result") or {}
        if not isinstance(plan, dict):
            continue
        for step in plan.get("steps") or []:
            if isinstance(step, dict):
                steps.append(step)
    return steps


def latest_ok_synthesis_output(steps: list[dict]) -> str:
    for step in reversed(steps):
        if str(step.get("skill") or "") != "synthesize_answer":
            continue
        if str(step.get("status") or "").lower() != "ok":
            continue
        output = str(step.get("output_excerpt") or step.get("output") or "").strip()
        if output:
            return output
    return ""


def final_answer_text(result: dict, summary: dict) -> str:
    for value in (
        summary.get("final_answer"),
        result.get("text"),
        result.get("message"),
    ):
        if isinstance(value, str) and value.strip():
            return value.strip()
    messages = result.get("messages")
    if isinstance(messages, list):
        for item in reversed(messages):
            if isinstance(item, str) and item.strip():
                return item.strip()
    return ""


def structured_error_kind(step: dict) -> str:
    for key in ("error_kind",):
        value = step.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    raw = str(step.get("error") or step.get("error_excerpt") or "")
    marker = "__RC_SKILL_ERROR__:"
    if marker in raw:
        try:
            payload = raw.split(marker, 1)[1]
            return str(json.loads(payload).get("error_kind") or "").strip()
        except Exception:
            return "malformed_structured_error"
    return ""


def classify(obj: dict) -> tuple[str, str]:
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    journal = result.get("task_journal") or {}
    summary = journal.get("summary") or {}
    trace = journal.get("trace") or {}
    route = summary.get("route_result") or {}
    status = str(data.get("status") or "").strip()
    verifier = journal.get("answer_verifier_summary") or summary.get("answer_verifier_summary")
    steps = find_step_results(trace)
    plan_steps = find_plan_steps(trace)
    finalizer_summary = summary.get("finalizer_summary") or {}
    latest_synthesis = latest_ok_synthesis_output(steps)
    final_answer = final_answer_text(result, summary)

    if (
        status == "succeeded"
        and latest_synthesis
        and final_answer
        and latest_synthesis.strip() != final_answer.strip()
        and isinstance(finalizer_summary, dict)
        and str(finalizer_summary.get("stage") or "").lower() == "observed_generic"
    ):
        return "finalizer_overwrite", "latest synthesized answer was replaced by observed-generic finalizer output"

    if isinstance(verifier, dict) and verifier.get("pass") is False:
        reason = str(verifier.get("answer_incomplete_reason") or "answer verifier rejected final answer")
        missing = verifier.get("missing_evidence_fields") or []
        if verifier.get("should_retry") is True and status == "succeeded":
            return "verifier_should_retry_not_applied", reason
        if missing:
            return "evidence_missing_field", reason
        return "verifier", reason

    if status == "succeeded":
        return "pass", "terminal status succeeded"

    failed_steps = [step for step in steps if str(step.get("status") or "").lower() == "error"]
    if failed_steps:
        last = failed_steps[-1]
        kind = structured_error_kind(last)
        if kind in {
            "unsupported_action",
            "invalid_input",
            "invalid_args",
            "schema_error",
            "missing_required_field",
        }:
            return "skill_schema", f"skill schema/argument failure: {kind}"
        if kind in {"not_found", "ambiguous_target", "permission_denied", "policy_block"}:
            if kind in {"not_found", "ambiguous_target"}:
                return "plan_missing_target", f"plan selected unresolved or ambiguous target: {kind}"
            return "skill_output", f"skill returned terminal/recoverable domain error: {kind}"
        if kind in {"spawn_failed", "wait_failed", "output_read_failed", "timeout", "idle_timeout"}:
            return "runtime_bug", f"runtime/runner failure: {kind}"
        return "skill_output", kind or "skill execution failed without structured error_kind"

    if status != "succeeded" and route:
        route_gate = str(route.get("route_gate_kind") or "").lower()
        execute_gates = {"execute", "act", "planner_execute"}
        if not plan_steps and route_gate in execute_gates:
            if not steps and final_answer:
                return (
                    "planner_bypass_without_evidence",
                    "execute route produced a final answer without planner/tool evidence",
                )
            return "planner", "execute route produced no executable plan"
        if route.get("needs_clarify") is True:
            return "normalizer", "route requested clarification"
        if route_gate not in execute_gates:
            return "route_miss", "request did not reach execute/act route"

    if status != "succeeded":
        if not route:
            return "runtime_bug", "missing route/journal context for failed task"
        return "finalizer", "task failed after routing without failed step evidence"

    label = "runtime_bug"
    return label, "unclassified terminal state"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: tag_run_outcomes.py <nl_suite_run_dir>", file=sys.stderr)
        return 2
    run_dir = Path(sys.argv[1]).resolve()
    if not run_dir.is_dir():
        print(f"run dir not found: {run_dir}", file=sys.stderr)
        return 2
    output_path = run_dir / "attribution.jsonl"
    rows = []
    for path in sorted(run_dir.glob("turn*.json")):
        obj = load_json(path)
        label, reason = classify(obj)
        if label not in ATTRIBUTION_LABELS:
            label = "runtime_bug"
        data = obj.get("data") or {}
        rows.append(
            {
                "file": path.name,
                "task_id": data.get("task_id"),
                "status": data.get("status"),
                "attribution": label,
                "reason": reason,
            }
        )
    output_path.write_text(
        "".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in rows),
        encoding="utf-8",
    )
    counts: dict[str, int] = {}
    for row in rows:
        counts[row["attribution"]] = counts.get(row["attribution"], 0) + 1
    print(
        "NL_ATTRIBUTION_OK "
        + " ".join(f"{key}={counts[key]}" for key in sorted(counts))
        + f" output={output_path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
