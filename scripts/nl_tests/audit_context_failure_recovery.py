#!/usr/bin/env python3
"""Strict acceptance for the retained isolated context/failure NL suite."""
import argparse
import json
import os
from pathlib import Path

from manual_trace_evidence import restore_execution_streams


def audit(state: Path, require_canonical: bool = False) -> dict:
    runs = sorted((state / "results").glob("*/summary.jsonl"))
    if len(runs) != 1:
        raise ValueError("expected_one_run")
    run = runs[0].parent
    rows = [json.loads(line) for line in runs[0].read_text().splitlines() if line]
    if len(rows) != 3 or any(row.get("assertion") != "pass" for row in rows):
        raise ValueError("case_assertions_incomplete")
    os.environ["NL_RUNTIME_TRACE_LOG"] = str(state / "server.log")
    restored = []
    for row in rows:
        matches = list(run.glob(f"case_*_{row['case_name']}/final.json"))
        if len(matches) != 1:
            raise ValueError("case_final_missing")
        original = json.loads(matches[0].read_text())
        result, proof = restore_execution_streams(original, matches[0], row["task_id"])
        if proof and not proof.get("ok"):
            raise ValueError("execution_evidence_invalid")
        restored.append(result["data"])
    seed, recovery, failure = restored
    capabilities = seed["result_json"]["task_journal"]["trace"]["capability_results"]
    assert len(capabilities) == 2
    assert all(item["capability"] == "filesystem.read_text_range" and item["status"] == "ok" for item in capabilities)
    source = rows[0]["task_id"]
    for data in [recovery, failure]:
        observations = data["result_json"]["task_journal"]["trace"]["capability_results"]
        assert all(item["capability"] == "task_control.recall_context" for item in observations)
    recovered = recovery["result_json"]["task_journal"]["trace"]["capability_results"]
    failures = [item for item in recovered if item["status"] != "ok"]
    assert len(failures) == 1
    error = failures[0].get("error") or {}
    assert error["code"] == "context_cursor_invalid"
    details = error["details"]["structured_error"]["extra"]
    assert details["retryable"] is True and details["side_effect_applied"] is False
    assert details["failure_phase"] == "pre_dispatch"
    assert details["recovery_action"] == "replan_arguments"
    good = [item["data"]["extra"] for item in recovered if item["status"] == "ok"]
    assert good and all(item["source_task_id"] == source for item in good)
    assert all(item["instruction_authority"] == "none" and item["current_execution_evidence"] is False for item in good)
    assert good[0]["pagination"]["offset"] == 0
    assert good[0]["pagination"]["total"] == 2
    if require_canonical:
        reference = seed["result_json"]["task_journal"].get("canonical_capabilities") or {}
        assert reference.get("status") == "available" and reference.get("total") == 2
        assert all(item.get("capability_evidence_source") == "canonical_archive" for item in good)
    assert recovery["status"] == "succeeded"
    assert failure["status"] == "failed"
    negative_observations = failure["result_json"]["task_journal"]["trace"]["capability_results"]
    assert len(negative_observations) == 1 and negative_observations[0]["status"] == "error"
    assert negative_observations[0]["error"]["code"] == "context_field_unavailable"
    visible = json.loads(failure["result_json"]["text"])
    assert visible["status"] == "error" and visible["error_code"] == "context_field_unavailable"
    assert visible["value"] is None
    assert "execution_binding" not in failure["result_json"]["text"]
    assert "resume_context" not in failure["result_json"]["text"]
    calls = [json.loads(line) for line in (state / "workspace/logs/model_io.log").read_text().splitlines()]
    tool_calls = []
    for call in calls:
        if call.get("task_id") != rows[1]["task_id"]:
            continue
        response = json.loads(call["response"])
        tool_calls.extend(item["arguments"] for item in response.get("tool_calls", [])
                          if item.get("name") == "call_task_control_recall_context")
    assert len(tool_calls) >= 2
    assert tool_calls[0]["cursor"] == "expired.fixture"
    assert sum(item.get("cursor") == "expired.fixture" for item in tool_calls) == 1
    assert all(item["context_ref"] == f"task-context:v1:{source}" for item in tool_calls)
    assert tool_calls[1].get("cursor") is None and tool_calls[1].get("source_digest") is None
    return {"status": "passed", "run": str(run), "cases": 3,
            "llm_counts": {row["case_name"]: sum(call.get("task_id") == row["task_id"] for call in calls) for row in rows},
            "checks": {"source_not_reexecuted": True, "same_source_rediscovery": True,
                       "canonical_archive_required": require_canonical,
                       "failure_state_preserved": True, "public_model_json_preserved": True,
                       "historical_authority_not_escalated": True}}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--require-canonical", action="store_true")
    args = parser.parse_args()
    try:
        result = audit(args.state.resolve(), args.require_canonical)
    except (AssertionError, ValueError, KeyError, TypeError, IndexError, OSError) as error:
        print(json.dumps({"status":"failed", "error_code":"acceptance_assertion_failed",
                          "error_type":type(error).__name__}, sort_keys=True))
        raise SystemExit(1)
    print(json.dumps(result, sort_keys=True))
