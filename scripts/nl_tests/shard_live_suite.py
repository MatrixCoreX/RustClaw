#!/usr/bin/env python3
"""Partition outstanding NL cases, validating prior results against current oracles."""
import argparse
import hashlib
import json
import os
from pathlib import Path

from manual_case_assertions import build_summary_row

ROOT = Path(__file__).resolve().parents[2]


def select_case_lines(lines, assigned_suites):
    if not assigned_suites:
        return lines
    names = {line.split("|", 4)[1] for path in assigned_suites
             for line in path.read_text().splitlines() if line and not line.startswith("#")}
    known = {line.split("|", 4)[1] for line in lines}
    if names - known:
        raise ValueError("assigned cases absent from current master suite")
    return [line for line in lines if line.split("|", 4)[1] in names]


def accepted_prior_results(lines, run_dirs):
    cases = {row.split("|", 4)[1]: row.split("|", 4) for row in lines}
    latest = {}
    for directory in run_dirs:
        summary = directory / "summary.jsonl"
        for raw in summary.read_text().splitlines():
            row = json.loads(raw)
            name = row["case_name"]
            if name not in cases:
                continue
            parts = cases[name]
            if parts[3] != row["prompt"]:
                continue
            finals = list(directory.glob(f"case_*_{name}/final.json"))
            if len(finals) != 1:
                continue
            expectation = parts[4].removeprefix("expect=") if len(parts) > 4 else ""
            checked = build_summary_row(row["source_line"], name, parts[2], parts[3],
                row["task_id"], str(finals[0]), row["status"], row["started_at"],
                row["ended_at"], expectation, row["mode"])
            order = (row["ended_at"], row["task_id"])
            if name not in latest or order > latest[name][0]:
                evidence = {
                    "task_id": row["task_id"],
                    "final_json": os.path.relpath(finals[0].resolve(), ROOT),
                } if checked["assertion"] == "pass" else None
                latest[name] = (order, evidence)
    return {name: evidence for name, (_, evidence) in latest.items() if evidence is not None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", type=Path, required=True)
    parser.add_argument("--completed-dir", type=Path, action="append", default=[])
    parser.add_argument("--select-from-suite", type=Path, action="append", default=[],
                        help="Resume only these stopped partitions, keeping current master prompts/oracles")
    parser.add_argument("--defer-case", action="append", default=[],
                        help="Record but do not rerun this case, for example a paid generation under investigation")
    parser.add_argument("--workers", type=int, choices=range(1, 5), default=4)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    text = args.suite.read_text()
    lines = [line for line in text.splitlines() if line and not line.startswith("#")]
    names = [line.split("|", 4)[1] for line in lines]
    if len(names) != len(set(names)):
        parser.error("duplicate case name")
    lines = select_case_lines(lines, args.select_from_suite)
    deferred = set(args.defer_case)
    if deferred - {line.split("|", 4)[1] for line in lines}:
        parser.error("deferred case absent from selected suite")
    accepted = accepted_prior_results(lines, args.completed_dir)
    pending = [line for line in lines if line.split("|", 4)[1] not in accepted.keys() | deferred]
    # Memory mutation needs an explicitly opted-in test principal; do not enable
    # automatic memory generation for unrelated cases or production accounts.
    memory = [line for line in pending if "capability:memory.save;" in line.split("|", 4)[2]]
    ordinary = [line for line in pending if line not in memory]
    partitions = {f"worker_{n+1}.txt": ordinary[n::args.workers] for n in range(args.workers)}
    if memory:
        partitions["memory_opt_in.txt"] = memory
    args.output_dir.mkdir(parents=True, exist_ok=True)
    manifest = {"schema_version": 1, "evidence_path_base": "workspace_root",
                "suite_sha256": hashlib.sha256(text.encode()).hexdigest(),
                "total_cases": len(lines), "accepted_prior": accepted,
                "deferred_cases": sorted(deferred), "partitions": {}}
    for filename, rows in partitions.items():
        (args.output_dir / filename).write_text("# Generated remainder; run each partition in its own isolated workspace.\n" + "\n".join(rows) + "\n")
        manifest["partitions"][filename] = {"count": len(rows), "enable_test_memory": filename == "memory_opt_in.txt",
                                            "names": [row.split("|", 4)[1] for row in rows]}
    (args.output_dir / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"total": len(lines), "accepted_prior": len(accepted), "pending": len(pending),
                      "deferred_cases": sorted(deferred),
                      "partitions": {name: len(rows) for name, rows in partitions.items()}}))


if __name__ == "__main__":
    main()
