#!/usr/bin/env python3
"""Statically guard portable path refs in NL/regression runners."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
PATH_REF_HELPER = ROOT / "scripts/path_ref.py"
RUN_MULTI_TURN_SUITE = ROOT / "scripts/nl_tests/run_multi_turn_suite.sh"
RUN_FULL_SUITE = ROOT / "scripts/nl_tests/run_full_suite.sh"
RUN_MANUAL_TEST = ROOT / "scripts/nl_tests/run_manual_test.sh"
RUN_DYNAMIC_GUARD_ALL = ROOT / "scripts/nl_tests/run_dynamic_guard_all.sh"
RUN_CONTRACT_PROVIDER_AB = ROOT / "scripts/nl_tests/run_contract_provider_ab_suite.sh"
RUN_CLIENT_LIKE_CONTINUOUS = ROOT / "scripts/nl_tests/run_client_like_continuous_suite.sh"
RUN_RUNTIME_CAPABILITY_BOUNDARY = ROOT / "scripts/nl_tests/run_runtime_capability_boundary_regression.sh"
OPS_CLOSED_LOOP = ROOT / "scripts/regression_ops_closed_loop.sh"
LONG_TAIL_FLOWS = ROOT / "scripts/regression_long_tail_nl_flows.sh"
SENSITIVE_FLOWS = ROOT / "scripts/regression_sensitive_nl_flows.sh"
SELF_EXTENSION_RUNTIME_ENABLE = ROOT / "scripts/regression_self_extension_runtime_enable.sh"
SELF_EXTENSION_NL_HANDOFF = ROOT / "scripts/regression_self_extension_nl_handoff.sh"
CIRCUIT_BREAKER = ROOT / "scripts/nl_tests/test_circuit_breaker.sh"
TASK_TERMINATION = ROOT / "scripts/verify_task_termination.sh"
INSPECT_TASK = ROOT / "scripts/inspect_task.sh"
BASE_SKILL_RESPONSE_CONTRACTS = ROOT / "scripts/check_base_skill_response_contracts.sh"
SKILLS_UPGRADE_SUITE = ROOT / "scripts/regression_skills_upgrade_suite.sh"

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
    RUN_MULTI_TURN_SUITE: {
        "path_ref_fn": "path_ref()",
        "case_file_ref": "case_file_ref:",
        "run_dir_ref": "run_dir_ref:",
        "run_log_ref": "run_log_ref:",
    },
    RUN_MANUAL_TEST: {
        "path_ref_fn": "path_ref()",
        "run_dir_ref": "run_dir_ref:",
        "run_log_ref": "run_log_ref:",
        "summary_jsonl_ref": "summary_jsonl_ref:",
        "resume_placeholder_ref": "--resume-dir <run_dir_ref>",
    },
    RUN_DYNAMIC_GUARD_ALL: {
        "path_ref_fn": "path_ref()",
        "manual_run_dir_ref": "manual_run_dir_ref:",
        "clarify_run_dir_ref": "clarify_run_dir_ref:",
        "context_run_dir_ref": "context_run_dir_ref:",
        "semantic_report_ref": "semantic_report_ref:",
    },
    RUN_CONTRACT_PROVIDER_AB: {
        "path_ref_fn": "path_ref()",
        "prepare_out_dir_ref": "PROVIDER_AB_PREPARE_OK out_dir_ref=",
        "run_side_run_dir_ref": "run_dir_ref=$(path_ref",
        "metadata_case_jsonl_ref": '"case_jsonl_ref"',
        "metadata_output_file_ref": '"output_file_ref"',
    },
    RUN_CLIENT_LIKE_CONTINUOUS: {
        "path_ref_fn": "path_ref()",
        "log_dir_ref_value": 'echo "log_dir=$(path_ref "$RUN_DIR")"',
        "db_path_ref": "db_path_ref=",
        "case_file_ref": "case_file_ref=",
        "case_jsonl_ref": "case_jsonl_ref=",
        "ok_log_dir_ref": 'CLIENT_LIKE_CONTINUOUS_SUITE_OK turns=${turn} log_dir=$(path_ref "$RUN_DIR")',
    },
    RUN_RUNTIME_CAPABILITY_BOUNDARY: {
        "path_ref_fn": "path_ref()",
        "ok_log_dir_ref": "RUNTIME_CAPABILITY_BOUNDARY_REGRESSION_OK log_dir_ref=",
        "failure_path_ref": "Runtime capability regression run failed before expectation evaluation: $(path_ref",
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
    LONG_TAIL_FLOWS: {
        "path_ref_fn": "path_ref()",
        "log_dir_ref": "log_dir_ref=",
        "workspace_root_ref": "workspace_root_ref=",
    },
    SENSITIVE_FLOWS: {
        "path_ref_fn": "path_ref()",
        "log_dir_ref": "log_dir_ref=",
        "workspace_root_ref": "workspace_root_ref=",
    },
    SELF_EXTENSION_RUNTIME_ENABLE: {
        "path_ref_fn": "path_ref()",
        "workspace_root_ref": "workspace_root_ref=",
    },
    SELF_EXTENSION_NL_HANDOFF: {
        "path_ref_fn": "path_ref()",
        "workspace_root_ref": "workspace_root_ref=",
    },
    TASK_TERMINATION: {
        "path_ref_fn": "path_ref()",
        "run_path_ref_fn": "run_path_ref()",
        "run_dir_ref": "run_dir_ref=",
        "run_log_ref": "run_log_ref=",
        "summary_jsonl_ref": "summary_jsonl_ref=",
        "db_path_ref": "db_path_ref=",
    },
    INSPECT_TASK: {
        "path_ref_fn": "path_ref()",
        "db_path_ref": "db_path_ref",
        "model_io_log_ref": "model_io_log_ref",
        "tracing_log_ref": "tracing_log_ref",
    },
    BASE_SKILL_RESPONSE_CONTRACTS: {
        "path_ref_fn": "path_ref()",
        "log_link_fn": "log_link()",
        "logs_ref": "Logs ref:",
        "report_ref": "Report ref:",
        "stdout_log_link": "$(log_link stdout",
        "stderr_log_link": "$(log_link stderr",
    },
    SKILLS_UPGRADE_SUITE: {
        "path_ref_fn": "path_ref()",
        "wrapper_smoke_stdout_tmp": 'WRAPPER_SMOKE_STDOUT="$TMP_DIR/wrapper_smoke.log"',
        "wrapper_report_ref": "Wrapper smoke report ref:",
        "base_contract_ref": "Base contract report ref:",
        "report_saved_ref": "Report saved ref:",
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
    RUN_MULTI_TURN_SUITE: {
        "case_file_absolute_print": 'echo "  case_file:  ${CASE_FILE}"',
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
    RUN_DYNAMIC_GUARD_ALL: {
        "manual_run_dir_absolute_print": 'echo "  manual_run_dir: ${manual_latest}"',
        "manual_run_log_absolute_print": 'echo "  manual_run_log: ${manual_latest}/run.log"',
        "clarify_run_dir_absolute_print": 'echo "  clarify_run_dir: ${clarify_latest}"',
        "context_run_dir_absolute_print": 'echo "  context_run_dir: ${context_latest}"',
    },
    RUN_CONTRACT_PROVIDER_AB: {
        "prepare_absolute_print": 'echo "PROVIDER_AB_PREPARE_OK out_dir=${OUT_DIR} case_jsonl=${CASE_JSONL} expectations=${EXPECTATIONS_JSONL}"',
        "run_side_inconclusive_absolute_print": 'echo "PROVIDER_AB_RUN_SIDE_INCONCLUSIVE side=${SIDE} provider=${PROVIDER} attempts=${attempts_run} run_dir=${RUN_DIR}"',
        "run_side_ok_absolute_print": 'echo "PROVIDER_AB_RUN_SIDE_OK side=${SIDE} provider=${PROVIDER} attempts=${attempts_run} run_dir=${RUN_DIR}"',
    },
    RUN_CLIENT_LIKE_CONTINUOUS: {
        "db_path_absolute_print": 'echo "db_path=${DB_PATH_VALUE}"',
        "log_dir_absolute_print": 'echo "log_dir=${RUN_DIR}"',
        "case_file_absolute_print": 'echo "case_file=${CASE_FILE_VALUE:-<none>}"',
        "case_jsonl_absolute_print": 'echo "case_jsonl=${CASE_JSONL_VALUE:-<none>}"',
        "ok_log_dir_absolute_print": 'echo "CLIENT_LIKE_CONTINUOUS_SUITE_OK turns=${turn} log_dir=${RUN_DIR}"',
    },
    RUN_RUNTIME_CAPABILITY_BOUNDARY: {
        "ok_log_dir_absolute_print": 'echo "RUNTIME_CAPABILITY_BOUNDARY_REGRESSION_OK log_dir=${RUN_DIR}"',
        "failure_absolute_print": 'echo "Runtime capability regression run failed before expectation evaluation: ${RUN_DIR}"',
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
    LONG_TAIL_FLOWS: {
        "log_dir_absolute_print": 'echo "log_dir=${LOG_DIR}"',
        "workspace_root_absolute_print": 'echo "workspace_root=${TEMP_WORKSPACE}"',
    },
    SENSITIVE_FLOWS: {
        "log_dir_absolute_print": 'echo "log_dir=${LOG_DIR}"',
        "workspace_root_absolute_print": 'echo "workspace_root=${TEMP_WORKSPACE}"',
    },
    SELF_EXTENSION_RUNTIME_ENABLE: {
        "workspace_root_absolute_print": 'echo "workspace_root=${TEMP_WORKSPACE}"',
    },
    SELF_EXTENSION_NL_HANDOFF: {
        "workspace_root_absolute_print": 'echo "workspace_root=${TEMP_WORKSPACE}"',
    },
    TASK_TERMINATION: {
        "run_dir_absolute_print": 'echo "run_dir=${RUN_DIR}"',
        "db_path_absolute_print": 'echo "db_path=${DB_PATH}"',
    },
    INSPECT_TASK: {
        "db_path_absolute_print": 'echo "db_path     : ${DB_PATH}"',
        "model_io_absolute_print": 'echo "model_io_log: ${MODEL_IO_LOG}"',
        "tracing_absolute_print": 'echo "tracing_log : ${TRACING_LOG}"',
        "missing_db_absolute_print": 'echo "[inspect_task] sqlite db missing: ${DB_PATH}"',
        "missing_model_io_absolute_print": 'echo "[inspect_task] model_io log missing: $MODEL_IO_LOG"',
    },
    BASE_SKILL_RESPONSE_CONTRACTS: {
        "logs_absolute_print": 'echo "Logs: $LOG_DIR"',
        "report_absolute_print": 'echo "Report: $REPORT_PATH"',
        "logs_absolute_report": 'echo "- Logs: \\`$LOG_DIR\\`"',
        "stdout_absolute_link": "[stdout]($stdout_log)",
        "stderr_absolute_link": "[stderr]($stderr_log)",
    },
    SKILLS_UPGRADE_SUITE: {
        "wrapper_smoke_absolute_tmp": "/tmp/rustclaw_wrapper_smoke.log",
        "wrapper_smoke_absolute_report_message": 'pass "wrapper smoke completed successfully (report: $WRAPPER_SMOKE_REPORT)"',
        "wrapper_smoke_absolute_fail_message": 'fail "wrapper smoke reported failures (report: $WRAPPER_SMOKE_REPORT)"',
        "wrapper_report_absolute": 'echo "- Wrapper smoke report: \\`$WRAPPER_SMOKE_REPORT\\`"',
        "base_contract_absolute": 'echo "- Base contract report: \\`$BASE_CONTRACTS_REPORT\\`"',
        "report_saved_absolute": 'echo "Report saved: $REPORT_PATH"',
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
