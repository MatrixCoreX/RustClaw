#!/usr/bin/env python3
"""Audit fresh NL argument recall against actual producer and tool observations."""
import argparse
import json
from pathlib import Path


def audit(run: Path) -> dict:
    rows = [json.loads(line) for line in (run / "summary.jsonl").read_text().splitlines() if line]
    names = ["seed_exact_read", "recall_exact_arguments"]
    if [row.get("case_name") for row in rows] != names or any(row.get("assertion") != "pass" for row in rows):
        raise ValueError("argument_case_assertions_incomplete")
    finals = []
    for name in names:
        paths = list(run.glob(f"case_*_{name}/final.json"))
        if len(paths) != 1:
            raise ValueError("argument_case_final_missing")
        data = json.loads(paths[0].read_text())["data"]
        if data["status"] != "succeeded":
            raise ValueError("argument_task_not_succeeded")
        finals.append(data["result_json"])
    source, recalled = finals
    observed = source["task_journal"]["trace"]["capability_results"]
    if len(observed) != 1 or observed[0]["capability"] != "filesystem.read_text_range" or observed[0]["status"] != "ok":
        raise ValueError("argument_source_execution_invalid")
    provenance = observed[0]["provenance"]
    args = provenance["continuation_args"]
    if not isinstance(args.get("path"), str) or not args["path"] or args.get("mode") != "head" or args.get("n") != 1:
        raise ValueError("argument_producer_fields_missing")
    projection = provenance["continuation_args_projection"]
    if projection.get("schema_version") != 2 or projection.get("truncated") is not False:
        raise ValueError("argument_producer_projection_incomplete")
    calls = recalled["task_journal"]["trace"]["capability_results"]
    found = False
    for call in calls:
        if call["capability"] != "task_control.recall_context" or call["status"] != "ok":
            raise ValueError("argument_recall_replayed_or_failed")
        extra = call["data"]["extra"]
        if (extra["source_task_id"] != rows[0]["task_id"] or extra["instruction_authority"] != "none"
                or extra["current_execution_evidence"] is not False):
            raise ValueError("argument_recall_authority_invalid")
        if "/capabilities/0/args" in extra["fields"]:
            if extra["fields"]["/capabilities/0/args"] != args:
                raise ValueError("argument_recall_not_exact")
            found = True
    if not found or json.loads(recalled["text"]) != args:
        raise ValueError("argument_recall_presentation_not_exact")
    return {"status": "passed", "cases": 2, "source_task_id": rows[0]["task_id"],
            "recall_task_id": rows[1]["task_id"], "argument_keys": sorted(args),
            "source_invocations": 1, "recall_invocations": len(calls), "historical_authority": "none"}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    options = parser.parse_args()
    try:
        print(json.dumps(audit(options.run), sort_keys=True))
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"status": "failed", "error_code": "exact_argument_acceptance_failed",
                          "error_type": type(error).__name__}))
        raise SystemExit(1)
