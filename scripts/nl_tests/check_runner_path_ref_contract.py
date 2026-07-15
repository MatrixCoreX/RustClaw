#!/usr/bin/env python3
"""Statically guard portable path refs in NL/regression runners."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
PATH_REF_HELPER = ROOT / "scripts/path_ref.py"
RUN_FULL_SUITE = ROOT / "scripts/nl_tests/run_full_suite.sh"
RUN_MANUAL_TEST = ROOT / "scripts/nl_tests/run_manual_test.sh"
OPS_CLOSED_LOOP = ROOT / "scripts/regression_ops_closed_loop.sh"
CIRCUIT_BREAKER = ROOT / "scripts/nl_tests/test_circuit_breaker.sh"

REQUIRED_SNIPPETS: dict[Path, dict[str, str]] = {
    PATH_REF_HELPER: {
        "helper_fn": "def portable_path_ref(",
        "helper_self_test": "PATH_REF_SELF_TEST ok",
        "helper_run_dir_anchor": 'return anchor_name if str(rel) == "." else f"{anchor_name}/{rel.as_posix()}"',
    },
    RUN_FULL_SUITE: {
        "path_ref_fn": "path_ref()",
        "run_dir_ref": "run_dir_ref:",
        "run_log_ref": "run_log_ref:",
        "case_file_ref": "case_file_ref:",
        "trace_case_file_ref": "trace_case_file_ref:",
        "artifact_simple_log_ref": "simple_nl_log_ref=",
    },
    RUN_MANUAL_TEST: {
        "path_ref_fn": "path_ref()",
        "run_dir_ref": "run_dir_ref:",
        "run_log_ref": "run_log_ref:",
        "summary_jsonl_ref": "summary_jsonl_ref:",
        "resume_placeholder_ref": "--resume-dir <run_dir_ref>",
    },
    OPS_CLOSED_LOOP: {
        "path_ref_fn": "path_ref()",
        "run_dir_ref": "run_dir_ref:",
        "run_log_ref": "run_log_ref:",
        "artifact_run_dir_ref": "run_dir_ref=$(path_ref",
    },
    CIRCUIT_BREAKER: {
        "path_ref_fn": "path_ref()",
        "run_dir_ref": "run_dir_ref:",
        "run_log_ref": "run_log_ref:",
        "summary_json_ref": "summary_json_ref:",
    },
}

FORBIDDEN_SNIPPETS: dict[Path, dict[str, str]] = {
    RUN_FULL_SUITE: {
        "run_dir_absolute_print": 'echo "  run_dir:          $RUN_DIR"',
        "case_file_absolute_print": 'echo "  case_file:        $CASE_FILE"',
        "trace_case_file_absolute_print": 'echo "  trace_case_file:  $TRACE_CASE_FILE"',
        "artifact_run_log_absolute_print": 'echo "  - $RUN_LOG"',
        "artifact_simple_absolute_print": 'echo "  - ${RUN_DIR}/simple_nl.log"',
    },
    RUN_MANUAL_TEST: {
        "interrupt_run_dir_absolute_print": 'echo "  run_dir:              ${RUN_DIR:-<not-created>}"',
        "resume_absolute_hint": 'echo "  --resume-dir ${RUN_DIR:-<run_dir>} --resume-line ${LAST_COMPLETED_LINE:-0}"',
        "case_file_absolute_print": 'echo "  case_file:     $CASE_FILE"',
        "run_dir_absolute_print": 'echo "  run_dir:       $RUN_DIR"',
        "run_log_absolute_print": 'echo "  run_log:       $RUN_LOG"',
        "summary_absolute_print": 'echo "  summary_jsonl: $SUMMARY_JSONL"',
        "artifact_run_dir_absolute_print": 'echo "  - $RUN_DIR"',
        "artifact_run_log_absolute_print": 'echo "  - $RUN_LOG"',
        "artifact_summary_absolute_print": 'echo "  - $SUMMARY_JSONL"',
    },
    OPS_CLOSED_LOOP: {
        "run_dir_absolute_print": 'echo "  run_dir: ${RUN_DIR}"',
        "run_log_absolute_print": 'echo "  run_log: ${RUN_LOG}"',
        "artifact_run_dir_absolute_print": 'echo "  - ${RUN_DIR}"',
        "artifact_run_log_absolute_print": 'echo "  - ${RUN_LOG}"',
    },
    CIRCUIT_BREAKER: {
        "run_dir_absolute_print": 'echo "  run_dir:    $RUN_DIR"',
        "artifact_run_dir_absolute_print": 'echo "  - $RUN_DIR"',
        "artifact_run_log_absolute_print": 'echo "  - $RUN_LOG"',
        "artifact_summary_absolute_print": 'echo "  - $SUMMARY_JSON"',
    },
}


def read_text(path: Path, findings: list[str]) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        findings.append(f"read_failed:{path.relative_to(ROOT)}:{exc.__class__.__name__}")
    except UnicodeDecodeError:
        findings.append(f"decode_failed:{path.relative_to(ROOT)}")
    return ""


def build_report() -> dict[str, Any]:
    findings: list[str] = []
    checked_count = 0

    for path, snippets in REQUIRED_SNIPPETS.items():
        text = read_text(path, findings)
        for label, snippet in snippets.items():
            checked_count += 1
            if snippet not in text:
                findings.append(f"missing_snippet:{path.relative_to(ROOT)}:{label}")

    for path, snippets in FORBIDDEN_SNIPPETS.items():
        text = read_text(path, findings)
        for label, snippet in snippets.items():
            checked_count += 1
            if snippet in text:
                findings.append(f"forbidden_snippet:{path.relative_to(ROOT)}:{label}")

    return {
        "ok": not findings,
        "checked_count": checked_count,
        "paths": sorted(
            str(path.relative_to(ROOT))
            for path in set(REQUIRED_SNIPPETS) | set(FORBIDDEN_SNIPPETS)
        ),
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
        print(f"RUNNER_PATH_REF_CONTRACT ok checked_count={report['checked_count']}")
    else:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
