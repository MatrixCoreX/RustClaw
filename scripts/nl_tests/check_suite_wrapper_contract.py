#!/usr/bin/env python3
"""Statically validate wrapped NL suite recovery contract wiring."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
RUN_SUITE = ROOT / "scripts/nl_tests/run_suite.sh"
RUN_MULTI_TURN_SUITE = ROOT / "scripts/nl_tests/run_multi_turn_suite.sh"
SUITE_ARTIFACT_CONTRACT = ROOT / "scripts/nl_tests/check_suite_artifact_contract.py"

RUN_SUITE_REQUIRED_SNIPPETS = {
    "write_artifact_index_fn": "write_artifact_index()",
    "write_suite_summary_fn": "write_suite_summary()",
    "write_suite_artifact_contract_report_fn": "write_suite_artifact_contract_report()",
    "contract_report_validate_content_arg": 'local validate_content="${2:-0}"',
    "contract_report_checker_args_array": "local checker_args=(. --json --require-contract-report)",
    "contract_report_validate_content_flag": "--validate-contract-report-content",
    "contract_report_require_checked_arg": 'local require_content_checked="${3:-0}"',
    "contract_report_require_checked_flag": "--require-contract-report-content-checked",
    "contract_report_final_require_checked_write": 'write_suite_artifact_contract_report "$run_dir" 1 1',
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
    "checker_uses_dot": 'check_suite_artifact_contract.py" "${checker_args[@]}"',
    "checker_requires_contract_report": "--require-contract-report",
    "contract_report_pending_placeholder": "contract_report_pending",
    "path_ref_fn": "path_ref()",
    "path_ref_run_dir_ref": 'print("run_dir" if str(rel) == "." else f"run_dir/{rel.as_posix()}")',
    "run_dir_ref_printed": 'echo "  run_dir_ref: $(path_ref "$run_dir" "$run_dir")"',
    "run_log_ref_printed": 'echo "  run_log_ref: $(path_ref "$run_dir" "$run_log")"',
    "artifact_run_dir_ref_printed": 'echo "  - run_dir_ref=$(path_ref "$run_dir" "$run_dir")"',
    "artifact_run_log_ref_printed": 'echo "  - run_log_ref=$(path_ref "$run_dir" "$run_log")"',
    "artifact_index_ref_printed": 'echo "  - artifact_index_ref=$(path_ref "$run_dir" "$artifact_index")"',
    "suite_summary_ref_printed": 'echo "  - suite_summary_ref=$(path_ref "$run_dir" "${run_dir}/suite_summary.env")"',
    "contract_report_ref_printed": 'echo "  - suite_artifact_contract_ref=$(path_ref "$run_dir" "$contract_report")"',
    "clarify_context_prompt_block": "run_mode_clarify_context_prompt()",
    "clarify_context_prompt_codex_header": "==== Paste this to Codex ====",
    "clarify_context_clarify_run_dir_ref": 'printf "clarify_run_dir_ref: %s\\n" "$(path_ref "$SCRIPT_DIR" "$latest_clarify")"',
    "clarify_context_clarify_run_log_ref": 'printf "clarify_run_log_ref: %s\\n" "$(path_ref "$SCRIPT_DIR" "${latest_clarify}/run.log")"',
    "clarify_context_clarify_summary_ref": 'printf "clarify_summary_jsonl_ref: %s\\n" "$(path_ref "$SCRIPT_DIR" "${latest_clarify}/summary.jsonl")"',
    "clarify_context_context_run_dir_ref": 'printf "context_run_dir_ref: %s\\n" "$(path_ref "$SCRIPT_DIR" "$latest_context")"',
    "clarify_context_context_run_log_ref": 'printf "context_run_log_ref: %s\\n" "$(path_ref "$SCRIPT_DIR" "${latest_context}/run.log")"',
    "clarify_context_context_summary_ref": 'printf "context_summary_jsonl_ref: %s\\n" "$(path_ref "$SCRIPT_DIR" "${latest_context}/summary.jsonl")"',
    "trap_captures_exit_code": "trap 'exit_code=$?",
    "trap_preserves_exit_code": 'exit "$exit_code"',
    "finalizer_does_not_replace_exit": 'finalize_wrapped_suite "$name" "$run_dir" "$run_log" "$suite_status" "$exit_code" || true',
}

RUN_SUITE_FORBIDDEN_SNIPPETS = {
    "run_dir_absolute_print": 'echo "  run_dir: ${run_dir}"',
    "run_log_absolute_print": 'echo "  run_log: ${run_log}"',
    "artifact_run_dir_absolute_print": 'echo "  - ${run_dir}"',
    "artifact_run_log_absolute_print": 'echo "  - ${run_log}"',
    "artifact_index_absolute_print": 'echo "  - ${artifact_index}"',
    "suite_summary_absolute_print": 'echo "  - ${run_dir}/suite_summary.env"',
    "contract_report_absolute_print": 'echo "  - ${contract_report}"',
    "clarify_run_dir_absolute_printf": 'printf "clarify_run_dir: %s\\n" "$latest_clarify"',
    "clarify_run_log_absolute_printf": 'printf "clarify_run_log: %s/run.log\\n" "$latest_clarify"',
    "clarify_summary_absolute_printf": 'printf "clarify_summary_jsonl: %s/summary.jsonl\\n" "$latest_clarify"',
    "context_run_dir_absolute_printf": 'printf "context_run_dir: %s\\n" "$latest_context"',
    "context_run_log_absolute_printf": 'printf "context_run_log: %s/run.log\\n" "$latest_context"',
    "context_summary_absolute_printf": 'printf "context_summary_jsonl: %s/summary.jsonl\\n" "$latest_context"',
}

RUN_MULTI_TURN_REQUIRED_SNIPPETS = {
    "multi_turn_path_ref_fn": "path_ref()",
    "multi_turn_path_ref_run_dir_ref": 'print("run_dir" if str(rel) == "." else f"run_dir/{rel.as_posix()}")',
    "multi_turn_run_dir_ref_printed": 'echo "  run_dir_ref: $(path_ref "$RUN_DIR" "$RUN_DIR")"',
    "multi_turn_run_log_ref_printed": 'echo "  run_log_ref: $(path_ref "$RUN_DIR" "$RUN_LOG")"',
}

RUN_MULTI_TURN_FORBIDDEN_SNIPPETS = {
    "multi_turn_run_dir_absolute_print": 'echo "  run_dir:    ${RUN_DIR}"',
    "multi_turn_run_log_absolute_print": 'echo "  run_log:    ${RUN_LOG}"',
}

SUITE_ARTIFACT_CONTRACT_REQUIRED_SNIPPETS = {
    "agent_parity_required_artifacts": "AGENT_PARITY_GATE_REQUIRED_ARTIFACTS",
    "agent_parity_required_flags": "AGENT_PARITY_GATE_REQUIRED_FLAGS",
    "agent_parity_required_machine_fields": "AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS",
    "agent_parity_dynamic_machine_fields": "AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS",
    "agent_parity_runner_path_ref_artifact": "agent_parity_gate/runner_path_ref_contract.json",
    "agent_parity_runner_path_ref_flag": '"runner_path_ref_contract": "1"',
    "agent_parity_runner_path_ref_json_ok": "runner_path_ref_contract.json",
    "agent_parity_env_file_state_dynamic_field": "chinese_provider_env_file_state",
    "agent_parity_env_file_source_dynamic_field": "chinese_provider_env_file_source",
    "agent_parity_text_content_tokens": "AGENT_PARITY_GATE_TEXT_CONTENT_TOKENS",
    "agent_parity_json_ok_artifacts": "AGENT_PARITY_GATE_JSON_OK_ARTIFACTS",
    "agent_parity_optional_artifacts": "AGENT_PARITY_GATE_OPTIONAL_ARTIFACTS_BY_FLAG",
    "agent_parity_chinese_providers": "AGENT_PARITY_CHINESE_MODEL_PROVIDERS",
    "agent_parity_suite_artifact_self_test": "suite_artifact_contract_self_test",
    "agent_parity_suite_artifact_self_test_token": "SUITE_ARTIFACT_CONTRACT_SELF_TEST ok",
    "agent_parity_rollout_metrics_contract": "rollout_metrics_contract",
    "agent_parity_rollout_metrics_contract_token": "ROLLOUT_METRICS_SELF_TEST ok",
    "agent_parity_self_test_stored_contract": "stored_agent_contract",
    "agent_parity_self_test_report_override": "stored_report_override",
    "agent_parity_text_token_validator": "validate_text_artifact_tokens",
    "agent_parity_artifact_decode_failed": "agent_parity_gate_artifact_decode_failed",
    "agent_parity_json_ok_validator": "validate_json_artifact_ok",
    "agent_parity_json_ok_bad_shape_finding": "agent_parity_gate_artifact_bad_shape",
    "agent_parity_json_ok_bad_shape_self_test": "json-ok-artifact-bad-shape",
    "agent_parity_compact_coverage_validator": "validate_compact_coverage_artifact",
    "agent_parity_chinese_catalog_validator": "validate_chinese_model_catalog_artifact",
    "agent_parity_chinese_catalog_self_test_artifact": "chinese_model_catalog_self_test.txt",
    "agent_parity_chinese_catalog_self_test_token": "CHINESE_MODEL_CATALOG_SELF_TEST ok",
    "agent_parity_chinese_catalog_bad_catalog_shape": "agent_parity_gate_chinese_model_catalog_bad_catalog_shape",
    "agent_parity_chinese_catalog_bad_catalog_shape_self_test": "chinese-model-catalog-bad-catalog-shape",
    "agent_parity_provider_smoke_validator": "validate_provider_smoke_artifacts",
    "agent_parity_provider_smoke_bad_providers_shape": "agent_parity_gate_provider_smoke_bad_providers_shape",
    "agent_parity_provider_smoke_bad_providers_shape_self_test": "provider-smoke-bad-providers-shape",
    "agent_parity_provider_smoke_case_coverage_validator": "validate_provider_smoke_case_coverage",
    "agent_parity_provider_case_coverage_bad_tags": "agent_parity_gate_provider_smoke_case_coverage_bad_provider_tags",
    "agent_parity_provider_case_coverage_bad_tags_self_test": "provider-case-coverage-bad-provider-tags",
    "agent_parity_provider_case_coverage_bad_case_file": "agent_parity_gate_provider_smoke_case_coverage_bad_case_file",
    "agent_parity_provider_case_coverage_bad_case_file_self_test": "provider-case-coverage-bad-case-file",
    "agent_parity_provider_summary_jsonl_parser": "parse_provider_summary_jsonl",
    "agent_parity_provider_summary_bad_json_line": "agent_parity_gate_provider_summary_bad_json_line",
    "agent_parity_provider_summary_bad_row": "agent_parity_gate_provider_summary_bad_row",
    "agent_parity_provider_summary_row_self_test": "provider-summary-jsonl-row-errors",
    "agent_parity_provider_path_ref_validator": "validate_provider_smoke_path_refs",
    "agent_parity_provider_path_ref_finding": "agent_parity_gate_provider_smoke_bad_path_ref",
    "agent_parity_provider_path_ref_self_test": "provider_path_ref_errors",
    "agent_parity_provider_live_scope_parser": "parse_live_provider_scope",
    "agent_parity_provider_live_scope_validator": "validate_live_provider_scope",
    "agent_parity_provider_live_scope_bad_finding": "agent_parity_gate_summary_bad_live_provider_scope",
    "agent_parity_provider_live_scope_missing_finding": "agent_parity_gate_summary_missing_live_provider_scope",
    "agent_parity_provider_live_scope_self_test": "live_provider_scope",
    "agent_parity_provider_live_scope_helper": "expected_live_scope_providers",
    "agent_parity_provider_scope_skip_reason": "provider_not_in_live_scope",
    "agent_parity_env_file_summary_validator": "validate_chinese_provider_env_file_summary",
    "agent_parity_env_file_state_bad_finding": "agent_parity_gate_summary_bad_env_file_state",
    "agent_parity_env_file_source_bad_finding": "agent_parity_gate_summary_bad_env_file_source",
    "agent_parity_env_file_summary_self_test": "env_file_summary",
    "agent_parity_summary_path_validator": "validate_gate_summary_no_host_paths",
    "agent_parity_summary_path_finding": "agent_parity_gate_summary_host_path",
    "agent_parity_summary_legacy_out_dir_finding": "agent_parity_gate_summary_legacy_out_dir",
    "agent_parity_summary_bad_out_dir_ref_finding": "agent_parity_gate_summary_bad_out_dir_ref",
    "agent_parity_summary_path_self_test": "gate-summary-host-path",
    "agent_parity_summary_out_dir_ref": "out_dir_ref",
    "agent_parity_run_log_no_host_path": 'validate_text_artifact_no_host_paths(run_dir, "run.log")',
    "agent_parity_run_log_host_path_finding": "agent_parity_gate_artifact_host_path:run.log",
    "agent_parity_run_log_host_path_self_test": "agent-parity-run-log-host-path",
    "agent_parity_rollout_metrics_validator": "validate_rollout_metrics_artifact",
    "agent_parity_rollout_metrics_host_path": "agent_parity_gate_metrics_host_path",
    "agent_parity_rollout_metrics_bad_source": "agent_parity_gate_metrics_bad_source_run_dir",
    "agent_parity_rollout_metrics_text_host_path": "rollout_metrics_text_host_path",
    "agent_parity_json_loader": "load_json_artifact",
    "agent_parity_json_loader_bad_shape_self_test": "load-json-artifact-bad-shape",
    "agent_parity_text_artifact_decode_self_test": "text-artifact-decode-failed",
    "agent_parity_json_loader_decode_self_test": "load-json-artifact-decode-failed",
    "agent_parity_summary_decode_failed": "summary_decode_failed",
    "agent_parity_artifact_index_decode_failed": "artifact_index_decode_failed",
    "agent_parity_summary_decode_self_test": "summary-decode-failed",
    "agent_parity_artifact_index_decode_self_test": "artifact-index-decode-failed",
    "agent_parity_live_metrics_summary_flag": '"live_metrics"',
    "agent_parity_live_metrics_strict_gate": 'live_metrics_enabled = gate_summary.get("live_metrics") == "1"',
    "agent_parity_bad_machine_field_finding": "agent_parity_gate_summary_bad_machine_field",
    "contract_report_content_validator": "validate_existing_contract_report",
    "contract_report_validate_cli_arg": "--validate-contract-report-content",
    "contract_report_require_checked_cli_arg": "--require-contract-report-content-checked",
    "contract_report_content_checked": '"contract_report_content_checked"',
    "contract_report_missing": "contract_report_missing",
    "contract_report_read_failed": "contract_report_read_failed",
    "contract_report_decode_failed": "contract_report_decode_failed",
    "contract_report_bad_json": "contract_report_bad_json",
    "contract_report_bad_shape": "contract_report_bad_shape",
    "contract_report_not_ok": "contract_report_not_ok",
    "contract_report_bad_run_dir": "contract_report_bad_run_dir",
    "contract_report_bad_require_contract_report": "contract_report_bad_require_contract_report",
    "contract_report_findings_not_empty": "contract_report_findings_not_empty",
    "contract_report_content_checked_not_true": "contract_report_content_checked_not_true",
    "contract_report_summary_mismatch": "contract_report_summary_mismatch",
    "contract_report_agent_contract_mismatch": "contract_report_agent_parity_contract_mismatch",
    "contract_report_unexpected_agent_contract": "contract_report_unexpected_agent_parity_contract",
    "contract_report_unexpected_agent_self_test": "unexpected_agent_contract",
    "contract_report_missing_self_test": "missing-contract-report",
    "contract_report_read_failed_self_test": "read-failed",
    "contract_report_decode_self_test": "contract-report-decode-failed",
    "contract_report_bad_json_self_test": "bad-json",
    "contract_report_bad_shape_self_test": "bad-shape",
    "agent_parity_optional_validator": "validate_enabled_agent_parity_optional_artifacts",
    "agent_parity_nested_validator": "validate_agent_parity_gate_artifacts",
    "agent_parity_summary_missing_finding": "agent_parity_gate_summary_missing",
    "agent_parity_summary_missing_structured_return": "return findings, content_checks",
    "agent_parity_summary_missing_self_test": "agent-parity-missing-gate-summary",
    "agent_parity_suite_branch": 'summary.get("suite") == "agent_parity_gate"',
    "agent_parity_contract_report_field": '"agent_parity_gate_contract"',
    "agent_parity_checked_flag": '"checked": True',
    "agent_parity_required_machine_field_count": '"required_machine_field_count"',
    "agent_parity_content_check_count": '"content_check_count"',
}

SUITE_ARTIFACT_CONTRACT_FORBIDDEN_SNIPPETS = {
    "agent_parity_live_metrics_missing_field_fallback": '"live_metrics" not in gate_summary',
    "agent_parity_live_metrics_run_dir_count_fallback": 'safe_int(gate_summary.get("run_dir_count")) > 0',
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


def check_forbidden_snippets(
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
        if snippet in text:
            findings.append(f"forbidden_snippet:{finding_prefix}:{label}")
    return findings, len(snippets)


def build_report() -> dict[str, Any]:
    findings: list[str] = []
    checked_count = 0
    for path, snippets, prefix in (
        (RUN_SUITE, RUN_SUITE_REQUIRED_SNIPPETS, "run_suite"),
        (RUN_MULTI_TURN_SUITE, RUN_MULTI_TURN_REQUIRED_SNIPPETS, "run_multi_turn_suite"),
        (
            SUITE_ARTIFACT_CONTRACT,
            SUITE_ARTIFACT_CONTRACT_REQUIRED_SNIPPETS,
            "suite_artifact_contract",
        ),
    ):
        path_findings, path_checked_count = check_required_snippets(path, snippets, prefix)
        findings.extend(path_findings)
        checked_count += path_checked_count
    run_suite_forbidden_findings, run_suite_forbidden_count = check_forbidden_snippets(
        RUN_SUITE,
        RUN_SUITE_FORBIDDEN_SNIPPETS,
        "run_suite",
    )
    findings.extend(run_suite_forbidden_findings)
    checked_count += run_suite_forbidden_count
    run_multi_turn_forbidden_findings, run_multi_turn_forbidden_count = check_forbidden_snippets(
        RUN_MULTI_TURN_SUITE,
        RUN_MULTI_TURN_FORBIDDEN_SNIPPETS,
        "run_multi_turn_suite",
    )
    findings.extend(run_multi_turn_forbidden_findings)
    checked_count += run_multi_turn_forbidden_count
    forbidden_findings, forbidden_checked_count = check_forbidden_snippets(
        SUITE_ARTIFACT_CONTRACT,
        SUITE_ARTIFACT_CONTRACT_FORBIDDEN_SNIPPETS,
        "suite_artifact_contract",
    )
    findings.extend(forbidden_findings)
    checked_count += forbidden_checked_count

    return {
        "ok": not findings,
        "paths": [
            str(RUN_SUITE.relative_to(ROOT)),
            str(RUN_MULTI_TURN_SUITE.relative_to(ROOT)),
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
