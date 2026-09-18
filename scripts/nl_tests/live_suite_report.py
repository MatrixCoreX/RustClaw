#!/usr/bin/env python3
"""Audit live NL runs against current cases and replay compact raw return fields."""
import argparse
from datetime import date
import json
from pathlib import Path

from manual_case_assertions import actual_call_steps, build_summary_row, successful_call_step
from manual_trace_evidence import restore_execution_streams
from print_llm_raw_trace import parse_json_text, raw_response_metadata


def case_index(path):
    cases = {}
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 4)
        cases[parts[1]] = {"source_line": number, "prompt": parts[3], "tags": parts[2],
                          "expectation": parts[4].removeprefix("expect=") if len(parts) > 4 else ""}
    return cases


def read_attempts(root, cases):
    attempts = []
    for summary in sorted(root.glob("**/summary.jsonl")):
        for line in summary.read_text().splitlines():
            if not line:
                continue
            row = json.loads(line)
            name = row["case_name"]
            if name not in cases:
                continue
            case = cases[name]
            finals = list(summary.parent.glob(f"case_*_{name}/final.json"))
            if len(finals) != 1:
                continue
            final = json.loads(finals[0].read_text())
            final, _ = restore_execution_streams(final, finals[0], row["task_id"])
            result = final.get("data", {}).get("result_json") or {}
            checked = build_summary_row(case["source_line"], name, case["tags"], row["prompt"],
                row["task_id"], str(finals[0]), row["status"], row["started_at"],
                row["ended_at"], case["expectation"], row["mode"])
            current = case["prompt"] == row["prompt"]
            calls = actual_call_steps(result)
            acceptance_path = next((d["acceptance_path"] for d in checked["assertion_details"]
                                    if d.get("ok") and "acceptance_path" in d),
                                   "terminal_failure_accepted" if row["status"] == "failed" else "successful_execution")
            attempts.append({"case_name": name, "task_id": row["task_id"], "run_dir": str(summary.parent.resolve()),
                "ended_at": row["ended_at"], "status": row["status"], "current_prompt": current,
                "assertion": checked["assertion"], "accepted": current and checked["assertion"] == "pass",
                "recorded_assertion": row.get("assertion"), "current_expectation": case["expectation"],
                "acceptance_path": acceptance_path if current and checked["assertion"] == "pass" else None,
                "failed_checks": [x for x in checked["assertion_details"] if not x.get("ok")],
                "llm_call_count": checked["efficiency"]["llm_call_count"],
                "llm_call_count_scope": "task_only", "wall_seconds": row["wall_seconds"],
                "result_text": row["text"], "error_text": row.get("error_text"),
                "successful_capabilities": sorted({s.get("resolved_capability") or s.get("requested_capability") for s in calls if successful_call_step(s)} - {None}),
                "successful_skills": sorted({s.get("executed_skill") or s.get("resolved_tool_or_skill") for s in calls if successful_call_step(s)} - {None})})
    return sorted(attempts, key=lambda a: (a["ended_at"], a["task_id"]))


def build_report(root, suite):
    cases = case_index(suite)
    attempts = read_attempts(root, cases)
    latest = {a["case_name"]: a for a in attempts}
    latest_current = {a["case_name"]: a for a in attempts if a["current_prompt"]}
    effective = {**latest, **latest_current}
    accepted_attempts = [a for a in latest_current.values() if a["accepted"]]
    accepted = {a["case_name"] for a in accepted_attempts}
    return {"schema_version": 1, "total": len(cases), "attempts": attempts,
            "attempt_count": len(attempts), "tested_distinct": len(latest),
            "accepted_distinct": len(accepted), "remaining": sorted(set(cases) - accepted),
            "not_yet_tested": sorted(set(cases) - set(latest)),
            "failed_current": [a for name, a in effective.items() if name not in accepted],
            "successful_skills": sorted({s for a in accepted_attempts for s in a["successful_skills"]}),
            "successful_capabilities": sorted({s for a in accepted_attempts for s in a["successful_capabilities"]})}


def excerpt(value, limit):
    if not isinstance(value, str) or len(value) <= limit:
        return value
    half = limit // 2
    return {"chars": len(value), "head": value[:half], "tail": value[-half:]}


def model_log_files(path):
    archives = []
    for candidate in path.parent.glob(path.name + ".*"):
        suffix = candidate.name[len(path.name) + 1:]
        try:
            day = date.fromisoformat(suffix)
        except ValueError:
            continue
        if suffix == day.isoformat() and candidate.is_file():
            archives.append(candidate)
    return sorted(archives) + ([path] if path.is_file() else [])


def replay(attempt, limit):
    log = Path(attempt["run_dir"]) / "run.log"
    model_path = None
    with log.open() as handle:
        for line in handle:
            if line.strip().startswith("log_path="):
                model_path = Path(line.strip().split("=", 1)[1])
                break
    output = dict(attempt)
    output["result_text"] = excerpt(output["result_text"], limit)
    output["model_log"] = str(model_path)
    output["llm_returns"] = []
    paths = model_log_files(model_path) if model_path is not None else []
    if not paths:
        output["trace_unavailable"] = True
        return output
    output["model_logs"] = [str(path) for path in paths]
    task_marker = attempt["task_id"].encode()
    for path in paths:
        offset = 0
        with path.open("rb") as handle:
            for raw in handle:
                row_offset = offset
                offset += len(raw)
                # Active logs may end mid-record; parsed identity is authoritative.
                if not raw.endswith(b"\n"):
                    break
                if task_marker not in raw:
                    continue
                row = json.loads(raw)
                task_id = row.get("task_id")
                descendant = isinstance(task_id, str) and task_id.startswith(attempt["task_id"] + ":child:")
                if task_id != attempt["task_id"] and row.get("parent_task_id") != attempt["task_id"] and not descendant:
                    continue
                response = row.get("clean_response") or row.get("response")
                parsed = parse_json_text(response)
                finish, usage = raw_response_metadata(row.get("raw_response"))
                output["llm_returns"].append({"number": len(output["llm_returns"]) + 1,
                    "model_log": str(path), "row_offset": row_offset, "task_id": row.get("task_id"),
                    "logical_call_index": row.get("logical_call_index"),
                    "prompt_label": row.get("prompt_label") or row.get("prompt_source"),
                    "logical_prompt_path": row.get("logical_prompt_path") or row.get("prompt_source"),
                    "provider": row.get("provider"), "model": row.get("model"),
                    "finish_reason": row.get("finish_reason") or finish,
                    "usage": row.get("usage") or usage, "error": row.get("error"),
                    "response_text": excerpt(response, limit),
                    "parsed_json": excerpt(json.dumps(parsed, ensure_ascii=False), limit),
                    "tool_calls": parsed.get("tool_calls") if isinstance(parsed, dict) else None})
    output["recorded_llm_returns"] = len(output["llm_returns"])
    task_calls = sum(row["task_id"] == attempt["task_id"] for row in output["llm_returns"])
    output["llm_call_counts"] = {"task": task_calls,
        "descendants": output["recorded_llm_returns"] - task_calls,
        "recorded_total": output["recorded_llm_returns"]}
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-root", type=Path, required=True)
    parser.add_argument("--suite", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--task-id")
    parser.add_argument("--response-chars", type=int, default=320)
    args = parser.parse_args()
    report = build_report(args.run_root, args.suite)
    if args.output:
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    if args.task_id:
        for attempt in report["attempts"]:
            if attempt["task_id"] == args.task_id:
                print(json.dumps(replay(attempt, max(160, args.response_chars)), ensure_ascii=False))
                return
        parser.error("task not found in completed summaries")
    print(json.dumps({k: v for k, v in report.items() if k not in {"attempts", "remaining", "not_yet_tested", "failed_current"}}, ensure_ascii=False))
    for attempt in report["failed_current"]:
        print(json.dumps({k: attempt[k] for k in ("case_name", "task_id", "status", "assertion", "failed_checks")}, ensure_ascii=False))


if __name__ == "__main__":
    main()
