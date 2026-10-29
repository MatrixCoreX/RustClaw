#!/usr/bin/env python3
"""Inspect real journals and validate generated project only inside a read-only sandbox."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

from manual_trace_evidence import restore_execution_streams


CASE_NAMES = ("create_project", "index_project", "amend_after_restart", "verify_retained_project")


def project_commands(project: Path) -> list[list[str]]:
    if platform.system() != "Linux" or not shutil.which("bwrap"):
        raise ValueError("independent_code_validation_sandbox_unavailable")
    command = ["bwrap", "--die-with-parent", "--unshare-all", "--new-session",
               "--ro-bind", "/usr", "/usr", "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]
    for path in ("/lib", "/lib64", "/bin", "/etc/ld.so.cache"):
        if Path(path).exists():
            command.extend(["--ro-bind", path, path])
    command.extend(["--ro-bind", str(project), "/workspace", "--chdir", "/workspace",
                    "--clearenv", "--setenv", "PATH", "/usr/bin:/bin", "--setenv", "HOME", "/tmp",
                    "--setenv", "PYTHONDONTWRITEBYTECODE", "1", "--"])
    return [command + ["/usr/bin/python3", "-m", "unittest", "discover", "-s", ".", "-p", "test_pricing.py"],
            command + ["/usr/bin/node", "test_pricing.mjs"],
            command + ["/usr/bin/python3", "-c",
                       "import pricing; assert pricing.price_total([2,-3,4]) == 6; assert pricing.price_total([-2]) == 0; assert pricing.price_total([]) == 0"],
            command + ["/usr/bin/node", "--input-type=module", "-e",
                       "import {price_total} from './pricing.mjs'; import assert from 'node:assert/strict'; assert.equal(price_total([2,-3,4]),6); assert.equal(price_total([-2]),0); assert.equal(price_total([]),0);"]]


def audit(state: Path) -> dict:
    os.environ["NL_RUNTIME_TRACE_LOG"] = str(state / "server.log")
    rows = []
    for summary in sorted((state / "results").glob("*/summary.jsonl")):
        for line in summary.read_text().splitlines():
            row = json.loads(line)
            row["run_dir"] = summary.parent
            rows.append(row)
    if tuple(row.get("case_name") for row in rows) != CASE_NAMES:
        raise ValueError("continuous_case_sequence_incomplete")
    if any(row.get("assertion") != "pass" or row.get("status") != "succeeded" for row in rows):
        raise ValueError("case_not_accepted")
    calls = [json.loads(line) for line in (state / "workspace/logs/model_io.log").read_text().splitlines()]
    journals = []
    for row in rows:
        matches = list(row["run_dir"].glob(f"case_*_{row['case_name']}/final.json"))
        if len(matches) != 1:
            raise ValueError("case_final_missing")
        original = json.loads(matches[0].read_text())
        restored, proof = restore_execution_streams(original, matches[0], row["task_id"])
        if proof and not proof.get("ok"):
            raise ValueError("execution_evidence_invalid")
        journals.append(restored["data"]["result_json"]["task_journal"]["trace"])
    indexed = journals[1]["capability_results"]
    required = {"code.find_definitions", "code.find_references", "code.list_tests"}
    assert required <= {item["capability"] for item in indexed if item["status"] == "ok"}
    index_reads = required | {"code.search_symbols", "code.retrieve_context",
                              "filesystem.read_text_range", "filesystem.stat_paths", "filesystem.list_paths"}
    assert all(item["capability"] in index_reads for item in indexed)
    project = state / "workspace/tmp/multilang_project_20261029"
    if project.is_symlink() or not project.resolve().is_relative_to((state / "workspace").resolve()):
        raise ValueError("project_path_unsafe")
    for name in ("pricing.py", "pricing.mjs", "test_pricing.py", "test_pricing.mjs", "red_phase.json"):
        file = project / name
        if file.is_symlink() or not file.is_file() or file.stat().st_size > 2 * 1024 * 1024 \
                or not file.resolve().is_relative_to(project.resolve()):
            raise ValueError("project_file_missing_or_unsafe")
    red = json.loads((project / "red_phase.json").read_text())
    assert isinstance(red["returncode"], int) and red["returncode"] != 0
    assert red["command"] and (red["stdout"] or red["stderr"])
    checks = []
    for command in project_commands(project):
        result = subprocess.run(command, text=True, capture_output=True, timeout=60)
        checks.append({"returncode": result.returncode, "stdout": result.stdout[-4096:], "stderr": result.stderr[-4096:]})
        if result.returncode != 0:
            raise ValueError("independent_project_verification_failed:" + result.stderr[-1024:])
    restart_log = (state / "restart-evidence.json").read_text()
    restart = json.loads(restart_log)
    assert restart["after_case"] == "index_project" and restart["old_pid"] != restart["new_pid"]
    return {"status": "passed", "cases": 4, "real_restart": restart,
            "llm_counts": {row["case_name"]: sum(call.get("task_id") == row["task_id"] for call in calls) for row in rows},
            "independent_read_only_sandbox_checks": checks}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(audit(args.state.resolve()), ensure_ascii=False, sort_keys=True))
    except (AssertionError, ValueError, KeyError, OSError, subprocess.TimeoutExpired) as error:
        print(json.dumps({"status": "failed", "error_type": type(error).__name__, "detail": str(error)}))
        raise SystemExit(1)
