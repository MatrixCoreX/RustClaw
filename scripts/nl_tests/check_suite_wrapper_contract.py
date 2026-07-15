#!/usr/bin/env python3
"""Statically validate wrapped NL suite recovery contract wiring."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
RUN_SUITE = ROOT / "scripts/nl_tests/run_suite.sh"
SUITE_ARTIFACT_CONTRACT = ROOT / "scripts/nl_tests/check_suite_artifact_contract.py"

RUN_SUITE_REQUIRED_SNIPPETS = {
    "write_artifact_index_fn": "write_artifact_index()",
    "write_suite_summary_fn": "write_suite_summary()",
    "write_suite_artifact_contract_report_fn": "write_suite_artifact_contract_report()",
    "finalize_wrapped_suite_fn": "finalize_wrapped_suite()",
    "summary_file": 'local summary="${run_dir}/suite_summary.env"',
    "artifact_index_file": 'local artifact_index="${run_dir}/artifact_index.txt"',
    "contract_report_file": 'local contract_report="${run_dir}/suite_artifact_contract.json"',
    "summary_suite": 'echo "suite=${suite_name}"',
    "summary_status": 'echo "status=${status}"',
    "summary_exit_code": 'echo "exit_code=${exit_code}"',
    "summary_artifact_finalize_status": 'echo "artifact_finalize_status=${artifact_finalize_status}"',
    "summary_run_log_relative": 'echo "run_log=run.log"',
    "summary_artifact_index_relative": 'echo "artifact_index=artifact_index.txt"',
    "artifact_index_relative_find": "-printf '%P\\n'",
    "artifact_index_excludes_self": '! -name "artifact_index.txt"',
    "checker_script": 'check_suite_artifact_contract.py',
    "checker_runs_from_run_root": 'cd "$run_dir"',
    "checker_uses_dot": 'check_suite_artifact_contract.py" . --json',
    "checker_requires_contract_report": "--require-contract-report",
    "contract_report_pending_placeholder": "contract_report_pending",
    "contract_report_printed": 'echo "  - ${contract_report}"',
    "suite_summary_printed": 'echo "  - ${run_dir}/suite_summary.env"',
    "trap_captures_exit_code": "trap 'exit_code=$?",
    "trap_preserves_exit_code": 'exit "$exit_code"',
    "finalizer_does_not_replace_exit": 'finalize_wrapped_suite "$name" "$run_dir" "$run_log" "$suite_status" "$exit_code" || true',
}

SUITE_ARTIFACT_CONTRACT_REQUIRED_SNIPPETS = {
    "agent_parity_required_artifacts": "AGENT_PARITY_GATE_REQUIRED_ARTIFACTS",
    "agent_parity_required_flags": "AGENT_PARITY_GATE_REQUIRED_FLAGS",
    "agent_parity_nested_validator": "validate_agent_parity_gate_artifacts",
    "agent_parity_suite_branch": 'summary.get("suite") == "agent_parity_gate"',
    "agent_parity_contract_report_field": '"agent_parity_gate_contract"',
    "agent_parity_checked_flag": '"checked": True',
}


def check_required_snippets(
    path: Path,
    snippets: dict[str, str],
    finding_prefix: str,
) -> tuple[list[str], int]:
    findings: list[str] = []
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        return [f"{finding_prefix}_read_failed:{exc.__class__.__name__}"], len(snippets)

    for label, snippet in snippets.items():
        if snippet not in text:
            findings.append(f"missing_snippet:{finding_prefix}:{label}")
    return findings, len(snippets)


def build_report() -> dict[str, Any]:
    findings: list[str] = []
    checked_count = 0
    for path, snippets, prefix in (
        (RUN_SUITE, RUN_SUITE_REQUIRED_SNIPPETS, "run_suite"),
        (
            SUITE_ARTIFACT_CONTRACT,
            SUITE_ARTIFACT_CONTRACT_REQUIRED_SNIPPETS,
            "suite_artifact_contract",
        ),
    ):
        path_findings, path_checked_count = check_required_snippets(path, snippets, prefix)
        findings.extend(path_findings)
        checked_count += path_checked_count

    return {
        "ok": not findings,
        "paths": [
            str(RUN_SUITE.relative_to(ROOT)),
            str(SUITE_ARTIFACT_CONTRACT.relative_to(ROOT)),
        ],
        "checked_count": checked_count,
        "findings": findings,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    report = build_report()
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    elif report["ok"]:
        print(f"SUITE_WRAPPER_CONTRACT ok checked_count={report['checked_count']}")
    else:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
