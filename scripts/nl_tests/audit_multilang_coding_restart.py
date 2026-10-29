#!/usr/bin/env python3
"""Inspect real journals and validate generated project only inside a read-only sandbox."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile

from manual_trace_evidence import restore_execution_streams


CASE_NAMES = ("create_project", "index_project", "amend_after_restart", "verify_retained_project")
MAX_VALIDATOR_OUTPUT_BYTES = 64 * 1024


def run_validator(command: list[str]) -> subprocess.CompletedProcess:
    # These commands already require the Linux read-only, network-isolated sandbox.
    import resource

    def limit_output():
        resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_VALIDATOR_OUTPUT_BYTES, MAX_VALIDATOR_OUTPUT_BYTES))

    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        result = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=60, preexec_fn=limit_output)
        stdout.seek(0)
        stderr.seek(0)
        out = stdout.read(MAX_VALIDATOR_OUTPUT_BYTES)
        err = stderr.read(MAX_VALIDATOR_OUTPUT_BYTES)
        if len(out) >= MAX_VALIDATOR_OUTPUT_BYTES or len(err) >= MAX_VALIDATOR_OUTPUT_BYTES:
            raise ValueError("independent_validator_output_budget_exceeded")
        return subprocess.CompletedProcess(command, result.returncode,
            out.decode("utf-8", errors="replace"), err.decode("utf-8", errors="replace"))


def initial_implementations(journal: dict) -> dict[str, str]:
    sources = {}
    for item in journal.get("capability_results", []):
        if item.get("capability") != "filesystem.write_file" or item.get("status") != "ok":
            continue
        provenance = item.get("provenance") or {}
        content = (provenance.get("continuation_args") or {}).get("content")
        projection = provenance.get("continuation_args_projection") or {}
        if not isinstance(content, str) or projection.get("truncated"):
            continue
        data = item.get("data") or {}
        for after in (data.get("output") or {}).get("after", []):
            path = Path(after.get("path", ""))
            if path.name not in ("pricing.py", "pricing.mjs") or path.parts[-3:-1] != ("tmp", "multilang_project_20261029"):
                continue
            raw = content.encode("utf-8")
            if len(raw) > 2 * 1024 * 1024 or after.get("size_bytes") != len(raw) \
                    or after.get("sha256") != "sha256:" + hashlib.sha256(raw).hexdigest():
                raise ValueError("initial_implementation_evidence_digest_mismatch")
            sources[path.name] = content
    if set(sources) != {"pricing.py", "pricing.mjs"}:
        raise ValueError("initial_implementation_execution_evidence_missing")
    return sources


def check_red_suite(project: Path, initial: dict[str, str]) -> dict:
    with tempfile.TemporaryDirectory(prefix="agent-independent-red-") as directory:
        staged = Path(directory)
        for name, content in initial.items():
            (staged / name).write_text(content, encoding="utf-8")
        for name in ("test_pricing.py", "test_pricing.mjs"):
            (staged / name).write_bytes((project / name).read_bytes())
        # Only the existing read-only, network-isolated validator executes code.
        result = run_validator(project_commands(staged)[0])
        if result.returncode != 1:
            raise ValueError("updated_tests_do_not_reject_initial_implementation")
        return {"returncode": result.returncode, "stdout": result.stdout[-4096:],
                "stderr": result.stderr[-4096:], "authority": "independent_read_only_sandbox",
                "historical_probe_chronology_verified": False}


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
    index_reads = required | {"code.refresh_index", "code.search_symbols", "code.retrieve_context",
                              "filesystem.read_text_range", "filesystem.stat_paths", "filesystem.list_paths",
                              "task.plan_read"}
    if any(item["capability"] not in index_reads for item in indexed):
        raise ValueError("index_phase_non_read_or_unknown_capability")
    if any(item.get("effect") != "observe" or item.get("action") != "refresh"
           for item in indexed if item["capability"] == "code.refresh_index"):
        raise ValueError("index_refresh_contract_invalid")
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
    independent_red = check_red_suite(project, initial_implementations(journals[0]))
    checks = []
    for command in project_commands(project):
        result = run_validator(command)
        checks.append({"returncode": result.returncode, "stdout": result.stdout[-4096:], "stderr": result.stderr[-4096:]})
        if result.returncode != 0:
            raise ValueError("independent_project_verification_failed:" + result.stderr[-1024:])
    restart_log = (state / "restart-evidence.json").read_text()
    restart = json.loads(restart_log)
    assert restart["after_case"] == "index_project" and restart["old_pid"] != restart["new_pid"]
    return {"status": "passed", "cases": 4, "real_restart": restart,
            "llm_counts": {row["case_name"]: sum(call.get("task_id") == row["task_id"] for call in calls) for row in rows},
            "independent_read_only_sandbox_checks": checks,
            "independent_red_suite": independent_red}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(audit(args.state.resolve()), ensure_ascii=False, sort_keys=True))
    except (AssertionError, ValueError, KeyError, OSError, subprocess.TimeoutExpired) as error:
        print(json.dumps({"status": "failed", "error_type": type(error).__name__, "detail": str(error)}))
        raise SystemExit(1)
