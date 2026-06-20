#!/usr/bin/env python3
"""Extract exact replay inputs and lightweight expectations from client-like logs."""

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


def sort_key(path: Path) -> tuple[int, int, str]:
    match = CASE_RE.search(path.name)
    if not match:
        return (10**9, 10**9, path.name)
    return (int(match.group("case")), int(match.group("turn")), path.name)


def route_legacy_first_layer(route: Any) -> str:
    if not isinstance(route, dict):
        return ""
    return str(
        route.get("legacy_first_layer_decision")
        or route.get("first_layer_decision")
        or ""
    )


def compact_tag(value: str) -> str:
    value = re.sub(r"[^A-Za-z0-9_.-]+", "-", value.strip().lower()).strip("-")
    return value or "unknown"


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
    return list(dict.fromkeys(targets))


def collect_planned_actions(trace: dict[str, Any]) -> list[str]:
    actions: list[str] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return actions
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
            action_ref = step.get("action_ref")
            if isinstance(action_ref, str) and action_ref.strip():
                actions.append(action_ref.strip())
                continue
            action_type = step.get("action_type")
            skill = step.get("skill")
            action = step.get("action")
            if isinstance(skill, str) and isinstance(action, str) and skill.strip() and action.strip():
                actions.append(f"{skill.strip()}.{action.strip()}")
            elif isinstance(action_type, str) and action_type.strip():
                actions.append(action_type.strip())
    return list(dict.fromkeys(actions))


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
    return list(dict.fromkeys(executed))


def collect_step_field(trace: dict[str, Any], field: str) -> list[str]:
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
    return list(dict.fromkeys(values))


def compact_text(value: str, max_chars: int = 500) -> str:
    value = re.sub(r"\s+", " ", value).strip()
    if len(value) <= max_chars:
        return value
    return value[: max_chars - 15].rstrip() + "...(truncated)"


def string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [str(item) for item in value if item is not None]


def collect_verifier_approved(trace: dict[str, Any]) -> bool | None:
    approvals: list[bool] = []
    rounds = trace.get("rounds")
    if not isinstance(rounds, list):
        return None
    for round_obj in rounds:
        if not isinstance(round_obj, dict):
            continue
        verify = round_obj.get("verify_result")
        if isinstance(verify, dict) and isinstance(verify.get("approved"), bool):
            approvals.append(bool(verify["approved"]))
    if not approvals:
        return None
    return any(approvals)


@dataclass
class ExtractedCase:
    source_case: int
    source_turn: int
    source_file: str
    task_id: str
    prompt: str
    status: str
    first_layer: str
    route_gate: str
    routed_mode: str
    contract_match: str
    contract_semantic_kind: str
    contract_final_answer_shape: str
    required_evidence: list[str]
    observed_evidence: list[str]
    missing_evidence: list[str]
    plan_targets: list[str]
    planned_actions: list[str]
    requested_actions: list[str]
    executed: list[str]
    error_kinds: list[str]
    failure_attributions: list[str]
    verifier_approved: bool | None
    final_shape: str
    final_preview: str


def extract_file(path: Path) -> ExtractedCase | None:
    match = CASE_RE.search(path.name)
    source_case = int(match.group("case")) if match else -1
    source_turn = int(match.group("turn")) if match else -1
    obj = json.loads(path.read_text(encoding="utf-8"))
    result = get_path(obj, "data", "result_json") or {}
    journal = result.get("task_journal") if isinstance(result, dict) else {}
    summary = journal.get("summary") if isinstance(journal, dict) else {}
    trace = journal.get("trace") if isinstance(journal, dict) else {}
    route = summary.get("route_result") if isinstance(summary, dict) else {}
    contract_matrix = trace.get("contract_matrix") if isinstance(trace, dict) else {}
    evidence_coverage = trace.get("evidence_coverage") if isinstance(trace, dict) else {}
    prompt = summary.get("input_text") if isinstance(summary, dict) else None
    if not isinstance(prompt, str) or not prompt.strip():
        return None
    text = final_text(result if isinstance(result, dict) else {}, summary if isinstance(summary, dict) else {})
    return ExtractedCase(
        source_case=source_case,
        source_turn=source_turn,
        source_file=path.name,
        task_id=str(get_path(obj, "data", "task_id") or summary.get("task_id") or ""),
        prompt=prompt,
        status=str(get_path(obj, "data", "status") or ""),
        first_layer=route_legacy_first_layer(route),
        route_gate=str(route.get("route_gate_kind") or "") if isinstance(route, dict) else "",
        routed_mode=str(route.get("routed_mode") or "") if isinstance(route, dict) else "",
        contract_match=str(contract_matrix.get("contract_match") or "") if isinstance(contract_matrix, dict) else "",
        contract_semantic_kind=str(contract_matrix.get("semantic_kind") or "")
        if isinstance(contract_matrix, dict)
        else "",
        contract_final_answer_shape=str(contract_matrix.get("final_answer_shape") or "")
        if isinstance(contract_matrix, dict)
        else "",
        required_evidence=string_list(
            evidence_coverage.get("required_evidence") if isinstance(evidence_coverage, dict) else []
        ),
        observed_evidence=string_list(
            (
                (evidence_coverage.get("observed_fields") or [])
                + (evidence_coverage.get("observed_canonical") or [])
            )
            if isinstance(evidence_coverage, dict)
            else []
        ),
        missing_evidence=string_list(
            evidence_coverage.get("missing_evidence") if isinstance(evidence_coverage, dict) else []
        ),
        plan_targets=collect_plan_targets(trace if isinstance(trace, dict) else {}),
        planned_actions=collect_planned_actions(trace if isinstance(trace, dict) else {}),
        requested_actions=collect_step_field(trace if isinstance(trace, dict) else {}, "requested_action_ref"),
        executed=collect_executed(trace if isinstance(trace, dict) else {}),
        error_kinds=collect_step_field(trace if isinstance(trace, dict) else {}, "error_kind"),
        failure_attributions=collect_step_field(
            trace if isinstance(trace, dict) else {}, "failure_attribution"
        ),
        verifier_approved=collect_verifier_approved(trace if isinstance(trace, dict) else {}),
        final_shape=final_shape(text),
        final_preview=compact_text(text),
    )


def expectation_for_case(index: int, item: ExtractedCase, expect_status: str) -> dict[str, Any]:
    row: dict[str, Any] = {
        "case": index,
        "status": item.status if expect_status == "observed" else expect_status,
        "_source_case": item.source_case,
        "_source_file": item.source_file,
        "_previous_status": item.status,
    }
    if item.first_layer:
        row["first_layer"] = item.first_layer
    if item.route_gate:
        row["route_gate"] = item.route_gate
    if item.routed_mode:
        row["routed_mode"] = item.routed_mode
    if item.plan_targets:
        row["capability_any"] = item.plan_targets[:3]
    if item.executed:
        row["executed_any"] = item.executed[:3]
    if item.verifier_approved is not None:
        row["verifier_approved"] = item.verifier_approved
    if item.final_shape:
        row["final_shape"] = item.final_shape
    return row


def min_repro_for_case(index: int, item: ExtractedCase) -> dict[str, Any]:
    row: dict[str, Any] = {
        "case": index,
        "source_case": item.source_case,
        "source_turn": item.source_turn,
        "source_file": item.source_file,
        "source_task_id": item.task_id,
        "request": item.prompt,
        "status": item.status,
        "route": {
            "first_layer": item.first_layer,
            "route_gate": item.route_gate,
            "routed_mode": item.routed_mode,
        },
        "route_contract": {
            "contract_match": item.contract_match,
            "semantic_kind": item.contract_semantic_kind,
            "final_answer_shape": item.contract_final_answer_shape,
            "required_evidence": item.required_evidence,
        },
        "planned_actions": item.planned_actions,
        "requested_actions": item.requested_actions,
        "executed": item.executed,
        "observed_evidence": item.observed_evidence,
        "missing_evidence": item.missing_evidence,
        "error_kinds": item.error_kinds,
        "failure_attributions": item.failure_attributions,
        "final_shape": item.final_shape,
        "final_answer_preview": item.final_preview,
    }
    return row


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path)
    parser.add_argument("--case-jsonl", type=Path, required=True)
    parser.add_argument("--expectations", type=Path, required=True)
    parser.add_argument(
        "--min-repro",
        type=Path,
        help="write sanitized minimal reproduction JSONL with request, route contract, actions, evidence, and final answer preview",
    )
    parser.add_argument("--suite", default="client_like_replay")
    parser.add_argument(
        "--filter",
        choices=["all", "failed", "succeeded"],
        default="all",
        help="Which source cases to extract.",
    )
    parser.add_argument(
        "--expect-status",
        default="observed",
        help="Expected status to write. Use 'observed' to preserve source status.",
    )
    args = parser.parse_args()

    paths = sorted(args.run_dir.glob("turn_*_case_*.json"), key=sort_key)
    rows: list[ExtractedCase] = []
    for path in paths:
        item = extract_file(path)
        if item is None:
            continue
        if args.filter == "failed" and item.status == "succeeded":
            continue
        if args.filter == "succeeded" and item.status != "succeeded":
            continue
        rows.append(item)

    args.case_jsonl.parent.mkdir(parents=True, exist_ok=True)
    args.expectations.parent.mkdir(parents=True, exist_ok=True)
    if args.min_repro is not None:
        args.min_repro.parent.mkdir(parents=True, exist_ok=True)

    with args.case_jsonl.open("w", encoding="utf-8") as case_out, args.expectations.open(
        "w", encoding="utf-8"
    ) as expect_out:
        min_repro_out = (
            args.min_repro.open("w", encoding="utf-8") if args.min_repro is not None else None
        )
        for idx, item in enumerate(rows, 1):
            name = f"replay_case_{item.source_case if item.source_case >= 0 else idx}"
            tags = [
                "replay",
                f"source_status_{compact_tag(item.status)}",
            ]
            if item.first_layer:
                tags.append(f"first_layer_{compact_tag(item.first_layer)}")
            case_out.write(
                json.dumps(
                    {
                        "suite": args.suite,
                        "name": name,
                        "tags": tags,
                        "prompt": item.prompt,
                        "source_file": item.source_file,
                        "source_task_id": item.task_id,
                    },
                    ensure_ascii=False,
                    sort_keys=True,
                )
                + "\n"
            )
            expect_out.write(
                json.dumps(
                    expectation_for_case(idx, item, args.expect_status),
                    ensure_ascii=False,
                    sort_keys=True,
                )
                + "\n"
            )
            if min_repro_out is not None:
                min_repro_out.write(
                    json.dumps(
                        min_repro_for_case(idx, item),
                        ensure_ascii=False,
                        sort_keys=True,
                    )
                    + "\n"
                )
        if min_repro_out is not None:
            min_repro_out.close()

    print(
        f"EXTRACT_REPLAY_OK total={len(rows)} case_jsonl={args.case_jsonl} expectations={args.expectations}"
        + (f" min_repro={args.min_repro}" if args.min_repro is not None else "")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
