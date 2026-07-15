#!/usr/bin/env python3
"""Validate wrapped NL suite artifact contracts."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any


REQUIRED_SUMMARY_FIELDS = {
    "artifact_finalize_status",
    "artifact_index",
    "exit_code",
    "run_log",
    "status",
    "suite",
}

ALLOWED_STATUSES = {"ok", "error"}
ALLOWED_ARTIFACT_FINALIZE_STATUSES = {"ok", "error"}
MACHINE_TOKEN_RE = re.compile(r"^[a-z0-9_.-]+$")

AGENT_PARITY_GATE_REQUIRED_ARTIFACTS = {
    "agent_parity_gate/gate_summary.env",
    "agent_parity_gate/runtime_hard_reply_baseline.txt",
    "agent_parity_gate/no_agent_mode_payload.txt",
    "agent_parity_gate/agent_loop_static_contracts.txt",
    "agent_parity_gate/secret_scan_contract.json",
    "agent_parity_gate/suite_wrapper_contract.json",
    "agent_parity_gate/suite_artifact_contract_self_test.txt",
    "agent_parity_gate/llm_raw_trace_runner_contract.txt",
}

AGENT_PARITY_GATE_REQUIRED_FLAGS = {
    "no_agent_mode_payload": "1",
    "agent_loop_static_contracts": "1",
    "secret_scan_contract": "1",
    "suite_wrapper_contract": "1",
    "suite_artifact_contract_self_test": "1",
    "llm_raw_trace_runner_contract": "1",
}

AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS = {
    "live_metrics": {"0", "1"},
}

AGENT_PARITY_GATE_TEXT_CONTENT_TOKENS = {
    "agent_parity_gate/no_agent_mode_payload.txt": {
        "SELF_TEST_OK",
        "NO_AGENT_MODE_PAYLOAD ok",
    },
    "agent_parity_gate/agent_loop_static_contracts.txt": {
        "ROUTE_AUTHORITY_LEGACY_KEY_CHECK findings=0",
        "LEGACY_ROUTE_BOUNDARY_CHECK findings=0",
        "PRE_PLANNER_EXIT_REMOVAL_CHECK findings=0",
        "NL_HARDMATCH_SCAN unknown=0 known_legacy=0",
        "HISTORICAL_HARDCODED_LANGUAGE_SCAN total=",
    },
    "agent_parity_gate/llm_raw_trace_runner_contract.txt": {
        "SELF_TEST_OK",
        "LLM_RAW_TRACE_RUNNER_CONTRACT ok",
    },
    "agent_parity_gate/suite_artifact_contract_self_test.txt": {
        "SUITE_ARTIFACT_CONTRACT_SELF_TEST ok",
    },
}

AGENT_PARITY_GATE_JSON_OK_ARTIFACTS = {
    "agent_parity_gate/secret_scan_contract.json",
    "agent_parity_gate/suite_wrapper_contract.json",
}

AGENT_PARITY_GATE_OPTIONAL_ARTIFACTS_BY_FLAG = {
    "coverage": {
        "agent_parity_gate/compact_coverage.json",
    },
    "model_catalog": {
        "agent_parity_gate/chinese_model_catalog.json",
    },
    "provider_smoke": {
        "agent_parity_gate/chinese_provider_smoke.txt",
        "agent_parity_gate/chinese_provider_smoke/case_coverage.json",
        "agent_parity_gate/chinese_provider_smoke/matrix_summary.json",
        "agent_parity_gate/chinese_provider_smoke/provider_summary.jsonl",
    },
    "coding_fixture": {
        "agent_parity_gate/coding_loop_repair_eval.txt",
        "agent_parity_gate/coding_loop_repair_metrics.json",
        "agent_parity_gate/coding_loop_repair_metrics.txt",
    },
}

AGENT_PARITY_CHINESE_MODEL_PROVIDERS = {"deepseek", "mimo", "minimax", "qwen"}


def parse_env_file(path: Path) -> tuple[dict[str, str], list[str]]:
    values: dict[str, str] = {}
    findings: list[str] = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        return values, [f"summary_read_failed:{exc.__class__.__name__}"]
    for lineno, raw in enumerate(lines, 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            findings.append(f"summary_bad_line:{lineno}")
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if not key or not MACHINE_TOKEN_RE.fullmatch(key):
            findings.append(f"summary_bad_key:{lineno}")
            continue
        values[key] = value
    return values, findings


def is_safe_relative_path(value: str) -> bool:
    if not value or value.startswith("/") or "\\" in value:
        return False
    path = PurePosixPath(value)
    return all(part not in {"", ".", ".."} for part in path.parts)


def validate_relative_file(run_dir: Path, field: str, value: str) -> list[str]:
    findings: list[str] = []
    if not is_safe_relative_path(value):
        findings.append(f"path_not_run_root_relative:{field}")
        return findings
    if not (run_dir / value).is_file():
        findings.append(f"path_missing:{field}:{value}")
    return findings


def validate_summary(run_dir: Path, summary: dict[str, str]) -> list[str]:
    findings: list[str] = []
    missing = sorted(REQUIRED_SUMMARY_FIELDS - set(summary))
    if missing:
        findings.append(f"summary_missing_fields:{','.join(missing)}")
        return findings

    suite = summary["suite"]
    status = summary["status"]
    artifact_finalize_status = summary["artifact_finalize_status"]
    exit_code_text = summary["exit_code"]

    if not MACHINE_TOKEN_RE.fullmatch(suite):
        findings.append("summary_bad_suite_token")
    if status not in ALLOWED_STATUSES:
        findings.append(f"summary_bad_status:{status}")
    if artifact_finalize_status not in ALLOWED_ARTIFACT_FINALIZE_STATUSES:
        findings.append(f"summary_bad_artifact_finalize_status:{artifact_finalize_status}")

    try:
        exit_code = int(exit_code_text)
    except ValueError:
        findings.append(f"summary_bad_exit_code:{exit_code_text}")
    else:
        if exit_code < 0:
            findings.append(f"summary_negative_exit_code:{exit_code}")
        if status == "ok" and exit_code != 0:
            findings.append(f"summary_status_exit_code_mismatch:ok:{exit_code}")
        if status == "error" and exit_code == 0:
            findings.append("summary_status_exit_code_mismatch:error:0")

    findings.extend(validate_relative_file(run_dir, "run_log", summary["run_log"]))
    findings.extend(validate_relative_file(run_dir, "artifact_index", summary["artifact_index"]))
    return findings


def read_artifact_index(run_dir: Path, artifact_index_rel: str) -> tuple[set[str], list[str]]:
    findings: list[str] = []
    if not is_safe_relative_path(artifact_index_rel):
        return set(), [f"path_not_run_root_relative:artifact_index"]
    artifact_index = run_dir / artifact_index_rel
    try:
        entries = artifact_index.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        return set(), [f"artifact_index_read_failed:{exc.__class__.__name__}"]

    seen: set[str] = set()
    for lineno, raw in enumerate(entries, 1):
        entry = raw.strip()
        if not entry:
            findings.append(f"artifact_index_bad_line:{lineno}")
            continue
        if not is_safe_relative_path(entry):
            findings.append(f"artifact_index_entry_not_relative:{lineno}")
            continue
        seen.add(entry)
        if not (run_dir / entry).is_file():
            findings.append(f"artifact_index_entry_missing:{entry}")
    return seen, findings


def validate_artifact_index_entries(
    entries: set[str],
    require_contract_report: bool,
) -> list[str]:
    findings: list[str] = []

    required_entries = ["run.log", "suite_summary.env"]
    if require_contract_report:
        required_entries.append("suite_artifact_contract.json")
    for required in required_entries:
        if required not in entries:
            findings.append(f"artifact_index_missing_required:{required}")
    return findings


def validate_existing_contract_report(
    run_dir: Path,
    expected_report: dict[str, Any],
    require_content_checked: bool = False,
) -> list[str]:
    findings: list[str] = []
    report_path = run_dir / "suite_artifact_contract.json"
    try:
        payload = json.loads(report_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return ["contract_report_missing"]
    except OSError as exc:
        return [f"contract_report_read_failed:{exc.__class__.__name__}"]
    except json.JSONDecodeError:
        return ["contract_report_bad_json"]
    if not isinstance(payload, dict):
        return ["contract_report_bad_shape"]

    if payload.get("ok") is not True:
        findings.append(f"contract_report_not_ok:{payload.get('ok')}")
    if payload.get("run_dir") != ".":
        findings.append(f"contract_report_bad_run_dir:{payload.get('run_dir')}")
    if payload.get("require_contract_report") is not True:
        findings.append(
            f"contract_report_bad_require_contract_report:{payload.get('require_contract_report')}"
        )
    if payload.get("findings") != []:
        findings.append("contract_report_findings_not_empty")
    if require_content_checked and payload.get("contract_report_content_checked") is not True:
        findings.append(
            f"contract_report_content_checked_not_true:{payload.get('contract_report_content_checked')}"
        )
    if payload.get("summary") != expected_report.get("summary"):
        findings.append("contract_report_summary_mismatch")

    expected_agent_contract = expected_report.get("agent_parity_gate_contract")
    actual_agent_contract = payload.get("agent_parity_gate_contract")
    if expected_agent_contract is not None:
        if actual_agent_contract != expected_agent_contract:
            findings.append("contract_report_agent_parity_contract_mismatch")
    elif actual_agent_contract is not None:
        findings.append("contract_report_unexpected_agent_parity_contract")
    return findings


def read_text_artifact(path: Path, label: str) -> tuple[str, list[str]]:
    try:
        return path.read_text(encoding="utf-8"), []
    except OSError as exc:
        return "", [f"agent_parity_gate_artifact_read_failed:{label}:{exc.__class__.__name__}"]
    except UnicodeDecodeError:
        return "", [f"agent_parity_gate_artifact_decode_failed:{label}"]


def validate_text_artifact_tokens(run_dir: Path, rel_path: str, tokens: set[str]) -> list[str]:
    text, findings = read_text_artifact(run_dir / rel_path, rel_path)
    if findings:
        return findings
    for token in sorted(tokens):
        if token not in text:
            findings.append(f"agent_parity_gate_artifact_missing_token:{rel_path}:{token}")
    return findings


def validate_json_artifact_ok(run_dir: Path, rel_path: str) -> list[str]:
    text, findings = read_text_artifact(run_dir / rel_path, rel_path)
    if findings:
        return findings
    try:
        payload = json.loads(text)
    except json.JSONDecodeError:
        return [f"agent_parity_gate_artifact_bad_json:{rel_path}"]
    if payload.get("ok") is not True:
        return [f"agent_parity_gate_artifact_not_ok:{rel_path}:{payload.get('ok')}"]
    return []


def safe_int(value: Any, default: int = 0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def safe_float(value: Any, default: float = 0.0) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


def nested_get(obj: Any, *path: str) -> Any:
    cur = obj
    for key in path:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def load_json_artifact(run_dir: Path, rel_path: str) -> tuple[Any, list[str]]:
    text, findings = read_text_artifact(run_dir / rel_path, rel_path)
    if findings:
        return None, findings
    try:
        return json.loads(text), []
    except json.JSONDecodeError:
        return None, [f"agent_parity_gate_artifact_bad_json:{rel_path}"]


def parse_provider_summary_jsonl(run_dir: Path) -> tuple[list[dict[str, Any]], list[str]]:
    rel_path = "agent_parity_gate/chinese_provider_smoke/provider_summary.jsonl"
    text, findings = read_text_artifact(run_dir / rel_path, rel_path)
    if findings:
        return [], findings
    rows: list[dict[str, Any]] = []
    for lineno, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line:
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            findings.append(f"agent_parity_gate_provider_summary_bad_json_line:{lineno}")
            continue
        if not isinstance(payload, dict):
            findings.append(f"agent_parity_gate_provider_summary_bad_row:{lineno}")
            continue
        rows.append(payload)
    return rows, findings


def expected_live_scope_providers(gate_summary: dict[str, str]) -> set[str]:
    raw = (gate_summary.get("chinese_provider_live_providers") or "").strip().lower()
    if raw in {"", "all"}:
        return set(AGENT_PARITY_CHINESE_MODEL_PROVIDERS)
    return {item.strip() for item in raw.split(",") if item.strip()}


def validate_compact_coverage_artifact(run_dir: Path) -> tuple[list[str], int]:
    rel_path = "agent_parity_gate/compact_coverage.json"
    payload, findings = load_json_artifact(run_dir, rel_path)
    checks = 0
    if findings:
        return findings, checks
    checks += 5
    if safe_int(payload.get("case_count")) <= 0:
        findings.append("agent_parity_gate_compact_coverage_bad_case_count")
    if payload.get("missing") != {}:
        findings.append("agent_parity_gate_compact_coverage_missing_tags")
    if payload.get("forbidden_live_publish_rows") != []:
        findings.append("agent_parity_gate_compact_coverage_forbidden_live_publish")
    if payload.get("media_rows_without_dry_run") != []:
        findings.append("agent_parity_gate_compact_coverage_media_without_dry_run")
    covered = payload.get("covered") if isinstance(payload, dict) else None
    if not isinstance(covered, dict) or not {
        "agent_parity",
        "chinese_model_adapter",
    }.issubset(covered):
        findings.append("agent_parity_gate_compact_coverage_missing_core_groups")
    return findings, checks


def validate_chinese_model_catalog_artifact(run_dir: Path) -> tuple[list[str], int]:
    rel_path = "agent_parity_gate/chinese_model_catalog.json"
    payload, findings = load_json_artifact(run_dir, rel_path)
    checks = 0
    if findings:
        return findings, checks
    checks += 4
    if payload.get("status") != "ok":
        findings.append(f"agent_parity_gate_chinese_model_catalog_bad_status:{payload.get('status')}")
    if safe_int(payload.get("finding_count")) != 0:
        findings.append("agent_parity_gate_chinese_model_catalog_findings_nonzero")
    if payload.get("findings") != []:
        findings.append("agent_parity_gate_chinese_model_catalog_findings_not_empty")
    providers = {
        entry.get("provider")
        for entry in payload.get("catalog", [])
        if isinstance(entry, dict)
    }
    missing = sorted(AGENT_PARITY_CHINESE_MODEL_PROVIDERS - providers)
    if missing:
        findings.append(f"agent_parity_gate_chinese_model_catalog_missing_providers:{','.join(missing)}")
    return findings, checks


def validate_provider_smoke_case_coverage(run_dir: Path) -> tuple[list[str], int]:
    rel_path = "agent_parity_gate/chinese_provider_smoke/case_coverage.json"
    payload, findings = load_json_artifact(run_dir, rel_path)
    checks = 0
    if findings:
        return findings, checks
    checks += 5
    if payload.get("ok") is not True:
        findings.append("agent_parity_gate_provider_smoke_case_coverage_not_ok")
    if payload.get("missing_coverage_tags") != []:
        findings.append("agent_parity_gate_provider_smoke_case_coverage_missing_tags")
    if payload.get("missing_provider_tags") != []:
        findings.append("agent_parity_gate_provider_smoke_case_coverage_missing_provider_tags")
    if payload.get("forbidden_live_tag_hits") != []:
        findings.append("agent_parity_gate_provider_smoke_case_coverage_forbidden_live_tags")
    provider_tags = set(payload.get("provider_tags") or [])
    missing = sorted(AGENT_PARITY_CHINESE_MODEL_PROVIDERS - provider_tags)
    if missing:
        findings.append(f"agent_parity_gate_provider_smoke_case_coverage_missing_providers:{','.join(missing)}")
    return findings, checks


def validate_provider_smoke_artifacts(
    run_dir: Path,
    gate_summary: dict[str, str],
) -> tuple[list[str], int]:
    findings: list[str] = []
    checks = 0
    payload, json_findings = load_json_artifact(
        run_dir, "agent_parity_gate/chinese_provider_smoke/matrix_summary.json"
    )
    findings.extend(json_findings)
    if not json_findings:
        checks += 8
        if safe_int(payload.get("provider_count")) < len(AGENT_PARITY_CHINESE_MODEL_PROVIDERS):
            findings.append("agent_parity_gate_provider_smoke_bad_provider_count")
        provider_rows = [
            entry
            for entry in payload.get("providers", [])
            if isinstance(entry, dict)
        ]
        providers = {entry.get("provider") for entry in provider_rows}
        missing = sorted(AGENT_PARITY_CHINESE_MODEL_PROVIDERS - providers)
        if missing:
            findings.append(f"agent_parity_gate_provider_smoke_missing_providers:{','.join(missing)}")
        if not isinstance(payload.get("status_counts"), dict):
            findings.append("agent_parity_gate_provider_smoke_missing_status_counts")
        if not isinstance(payload.get("reason_code_counts"), dict):
            findings.append("agent_parity_gate_provider_smoke_missing_reason_code_counts")
        if not isinstance(payload.get("credential_state_counts"), dict):
            findings.append("agent_parity_gate_provider_smoke_missing_credential_state_counts")
        live_scope = expected_live_scope_providers(gate_summary)
        for provider in sorted(AGENT_PARITY_CHINESE_MODEL_PROVIDERS):
            row = next((item for item in provider_rows if item.get("provider") == provider), None)
            if row is None:
                continue
            if safe_int(row.get("exit_code"), -1) != 0:
                findings.append(f"agent_parity_gate_provider_smoke_bad_exit_code:{provider}")
            if provider in live_scope:
                if row.get("live_scope") not in {"included", "all"}:
                    findings.append(f"agent_parity_gate_provider_smoke_bad_in_scope_marker:{provider}:{row.get('live_scope')}")
                if row.get("status") != "planned" or row.get("reason_code") != "dry_run":
                    findings.append(f"agent_parity_gate_provider_smoke_bad_in_scope_status:{provider}:{row.get('status')}:{row.get('reason_code')}")
            else:
                if row.get("live_scope") != "excluded":
                    findings.append(f"agent_parity_gate_provider_smoke_bad_out_scope_marker:{provider}:{row.get('live_scope')}")
                if row.get("status") != "skipped" or row.get("reason_code") != "provider_not_in_live_scope":
                    findings.append(f"agent_parity_gate_provider_smoke_bad_out_scope_status:{provider}:{row.get('status')}:{row.get('reason_code')}")
        reason_counts = payload.get("reason_code_counts")
        if isinstance(reason_counts, dict) and "dry_run" not in reason_counts:
            findings.append("agent_parity_gate_provider_smoke_missing_dry_run_reason")
        rows, row_findings = parse_provider_summary_jsonl(run_dir)
        findings.extend(row_findings)
        row_providers = {row.get("provider") for row in rows}
        row_missing = sorted(AGENT_PARITY_CHINESE_MODEL_PROVIDERS - row_providers)
        if row_missing:
            findings.append(f"agent_parity_gate_provider_summary_missing_providers:{','.join(row_missing)}")
        checks += 1
    coverage_findings, coverage_checks = validate_provider_smoke_case_coverage(run_dir)
    findings.extend(coverage_findings)
    checks += coverage_checks
    text, text_findings = read_text_artifact(
        run_dir / "agent_parity_gate/chinese_provider_smoke.txt",
        "agent_parity_gate/chinese_provider_smoke.txt",
    )
    findings.extend(text_findings)
    if not text_findings:
        checks += 1
        if "CHINESE_PROVIDER_SMOKE_MATRIX" not in text:
            findings.append("agent_parity_gate_provider_smoke_missing_runner_token")
    return findings, checks


def validate_rollout_metrics_artifact(
    run_dir: Path,
    rel_path: str,
    gate_summary: dict[str, str],
) -> tuple[list[str], int]:
    payload, findings = load_json_artifact(run_dir, rel_path)
    checks = 0
    if findings:
        return findings, checks
    min_pass_rate = safe_float(gate_summary.get("min_pass_rate"), 1.0)
    max_avg_llm_calls = safe_float(gate_summary.get("max_avg_llm_calls"), 4.0)
    max_prompt_truncations = safe_int(gate_summary.get("max_prompt_truncations"), 0)
    max_provider_final_errors = safe_int(gate_summary.get("max_provider_final_errors"), 0)
    checks += 7
    if safe_int(payload.get("turns_total")) <= 0:
        findings.append(f"agent_parity_gate_metrics_bad_turns:{rel_path}")
    if safe_float(payload.get("pass_rate")) < min_pass_rate:
        findings.append(f"agent_parity_gate_metrics_pass_rate_below_threshold:{rel_path}")
    if nested_get(payload, "metric_gate", "passed") is not True:
        findings.append(f"agent_parity_gate_metrics_gate_not_passed:{rel_path}")
    if safe_int(payload.get("parse_errors")) != 0:
        findings.append(f"agent_parity_gate_metrics_parse_errors:{rel_path}")
    if safe_float(nested_get(payload, "llm", "avg_calls_per_turn")) > max_avg_llm_calls:
        findings.append(f"agent_parity_gate_metrics_avg_llm_calls_above_threshold:{rel_path}")
    if safe_int(nested_get(payload, "llm", "prompt_truncation_count")) > max_prompt_truncations:
        findings.append(f"agent_parity_gate_metrics_prompt_truncations_above_threshold:{rel_path}")
    if safe_int(nested_get(payload, "llm", "provider_final_error_count")) > max_provider_final_errors:
        findings.append(f"agent_parity_gate_metrics_provider_final_errors_above_threshold:{rel_path}")
    return findings, checks


def validate_coding_fixture_artifacts(
    run_dir: Path,
    gate_summary: dict[str, str],
) -> tuple[list[str], int]:
    findings: list[str] = []
    checks = 0
    for rel_path, token in (
        ("agent_parity_gate/coding_loop_repair_eval.txt", "CLIENT_LIKE_EVAL_OK"),
        ("agent_parity_gate/coding_loop_repair_metrics.txt", "ROLLOUT_METRICS_OK"),
    ):
        artifact_findings = validate_text_artifact_tokens(run_dir, rel_path, {token})
        findings.extend(artifact_findings)
        checks += 0 if artifact_findings else 1
    metrics_findings, metrics_checks = validate_rollout_metrics_artifact(
        run_dir,
        "agent_parity_gate/coding_loop_repair_metrics.json",
        gate_summary,
    )
    findings.extend(metrics_findings)
    checks += metrics_checks
    return findings, checks


def validate_enabled_agent_parity_optional_artifacts(
    run_dir: Path,
    entries: set[str],
    gate_summary: dict[str, str],
) -> tuple[list[str], int]:
    findings: list[str] = []
    checks = 0
    for flag, required_paths in sorted(AGENT_PARITY_GATE_OPTIONAL_ARTIFACTS_BY_FLAG.items()):
        if gate_summary.get(flag) != "1":
            continue
        for required in sorted(required_paths):
            checks += 1
            if required not in entries:
                findings.append(f"agent_parity_gate_enabled_artifact_missing:{flag}:{required}")
    if gate_summary.get("coverage") == "1":
        coverage_findings, coverage_checks = validate_compact_coverage_artifact(run_dir)
        findings.extend(coverage_findings)
        checks += coverage_checks
    if gate_summary.get("model_catalog") == "1":
        catalog_findings, catalog_checks = validate_chinese_model_catalog_artifact(run_dir)
        findings.extend(catalog_findings)
        checks += catalog_checks
    if gate_summary.get("provider_smoke") == "1":
        smoke_findings, smoke_checks = validate_provider_smoke_artifacts(run_dir, gate_summary)
        findings.extend(smoke_findings)
        checks += smoke_checks
    if gate_summary.get("coding_fixture") == "1":
        coding_findings, coding_checks = validate_coding_fixture_artifacts(run_dir, gate_summary)
        findings.extend(coding_findings)
        checks += coding_checks
    live_metrics_enabled = gate_summary.get("live_metrics") == "1" or (
        "live_metrics" not in gate_summary
        and gate_summary.get("metrics") == "1"
        and safe_int(gate_summary.get("run_dir_count")) > 0
    )
    if live_metrics_enabled:
        for rel_path, token in (
            ("agent_parity_gate/run_metrics.txt", "ROLLOUT_METRICS_OK"),
        ):
            artifact_findings = validate_text_artifact_tokens(run_dir, rel_path, {token})
            findings.extend(artifact_findings)
            checks += 0 if artifact_findings else 1
        metrics_findings, metrics_checks = validate_rollout_metrics_artifact(
            run_dir,
            "agent_parity_gate/run_metrics.json",
            gate_summary,
        )
        findings.extend(metrics_findings)
        checks += metrics_checks
    return findings, checks


def validate_agent_parity_gate_artifacts(run_dir: Path, entries: set[str]) -> tuple[list[str], int]:
    findings: list[str] = []
    content_checks = 0
    for required in sorted(AGENT_PARITY_GATE_REQUIRED_ARTIFACTS):
        if required not in entries:
            findings.append(f"agent_parity_gate_artifact_missing:{required}")

    summary_path = run_dir / "agent_parity_gate/gate_summary.env"
    if not summary_path.is_file():
        findings.append("agent_parity_gate_summary_missing")
        return findings

    gate_summary, parse_findings = parse_env_file(summary_path)
    findings.extend(f"agent_parity_gate_{finding}" for finding in parse_findings)
    for key, expected in sorted(AGENT_PARITY_GATE_REQUIRED_FLAGS.items()):
        content_checks += 1
        actual = gate_summary.get(key)
        if actual != expected:
            findings.append(f"agent_parity_gate_summary_bad_flag:{key}:{actual}")
    for key, allowed_values in sorted(AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS.items()):
        content_checks += 1
        actual = gate_summary.get(key)
        if actual not in allowed_values:
            findings.append(f"agent_parity_gate_summary_bad_machine_field:{key}:{actual}")
    for rel_path, tokens in sorted(AGENT_PARITY_GATE_TEXT_CONTENT_TOKENS.items()):
        token_findings = validate_text_artifact_tokens(run_dir, rel_path, tokens)
        findings.extend(token_findings)
        content_checks += 0 if token_findings else len(tokens)
    for rel_path in sorted(AGENT_PARITY_GATE_JSON_OK_ARTIFACTS):
        ok_findings = validate_json_artifact_ok(run_dir, rel_path)
        findings.extend(ok_findings)
        content_checks += 0 if ok_findings else 1
    optional_findings, optional_checks = validate_enabled_agent_parity_optional_artifacts(
        run_dir,
        entries,
        gate_summary,
    )
    findings.extend(optional_findings)
    content_checks += optional_checks
    return findings, content_checks


def validate_run_dir(
    run_dir: Path,
    require_contract_report: bool = False,
    validate_contract_report_content: bool = False,
    require_contract_report_content_checked: bool = False,
) -> dict[str, Any]:
    findings: list[str] = []
    if not run_dir.exists():
        findings.append("run_dir_missing")
        return {"ok": False, "run_dir": str(run_dir), "findings": findings}
    if not run_dir.is_dir():
        findings.append("run_dir_not_directory")
        return {"ok": False, "run_dir": str(run_dir), "findings": findings}

    summary_path = run_dir / "suite_summary.env"
    if not summary_path.is_file():
        findings.append("summary_missing")
        return {"ok": False, "run_dir": str(run_dir), "findings": findings}

    summary, parse_findings = parse_env_file(summary_path)
    findings.extend(parse_findings)
    findings.extend(validate_summary(run_dir, summary))
    artifact_index_rel = summary.get("artifact_index")
    artifact_entries: set[str] = set()
    agent_parity_gate_contract: dict[str, Any] | None = None
    if artifact_index_rel:
        artifact_entries, index_findings = read_artifact_index(run_dir, artifact_index_rel)
        findings.extend(index_findings)
        findings.extend(validate_artifact_index_entries(artifact_entries, require_contract_report))

    if summary.get("suite") == "agent_parity_gate":
        agent_parity_findings, content_check_count = validate_agent_parity_gate_artifacts(
            run_dir, artifact_entries
        )
        findings.extend(agent_parity_findings)
        agent_parity_gate_contract = {
            "checked": True,
            "required_artifact_count": len(AGENT_PARITY_GATE_REQUIRED_ARTIFACTS),
            "required_flag_count": len(AGENT_PARITY_GATE_REQUIRED_FLAGS),
            "required_machine_field_count": len(AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS),
            "content_check_count": content_check_count,
        }

    report = {
        "ok": not findings,
        "run_dir": str(run_dir),
        "require_contract_report": require_contract_report,
        "contract_report_content_checked": False,
        "summary": summary,
        "findings": findings,
    }
    if agent_parity_gate_contract is not None:
        report["agent_parity_gate_contract"] = agent_parity_gate_contract
    if validate_contract_report_content:
        expected_report = dict(report)
        expected_report["findings"] = list(findings)
        contract_report_findings = validate_existing_contract_report(
            run_dir,
            expected_report,
            require_content_checked=require_contract_report_content_checked,
        )
        findings.extend(contract_report_findings)
        report["ok"] = not findings
        report["contract_report_content_checked"] = True
        report["findings"] = findings
    return report


def write_minimal_self_test_run(
    run_dir: Path,
    *,
    content_checked: bool,
    stored_summary_override: dict[str, str] | None = None,
    stored_agent_contract: dict[str, Any] | None = None,
    stored_report_override: dict[str, Any] | None = None,
) -> dict[str, str]:
    summary = {
        "suite": "manual",
        "status": "ok",
        "exit_code": "0",
        "artifact_finalize_status": "ok",
        "run_log": "run.log",
        "artifact_index": "artifact_index.txt",
    }
    run_dir.mkdir(parents=True, exist_ok=True)
    (run_dir / "run.log").write_text("", encoding="utf-8")
    (run_dir / "suite_summary.env").write_text(
        "\n".join(f"{key}={value}" for key, value in summary.items()) + "\n",
        encoding="utf-8",
    )
    (run_dir / "artifact_index.txt").write_text(
        "artifact_index.txt\nrun.log\nsuite_artifact_contract.json\nsuite_summary.env\n",
        encoding="utf-8",
    )
    report_summary = dict(summary)
    if stored_summary_override:
        report_summary.update(stored_summary_override)
    stored_report = {
        "ok": True,
        "run_dir": ".",
        "require_contract_report": True,
        "contract_report_content_checked": content_checked,
        "summary": report_summary,
        "findings": [],
    }
    if stored_agent_contract is not None:
        stored_report["agent_parity_gate_contract"] = stored_agent_contract
    if stored_report_override:
        stored_report.update(stored_report_override)
    (run_dir / "suite_artifact_contract.json").write_text(
        json.dumps(
            stored_report,
            ensure_ascii=False,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    return summary


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="suite-artifact-contract-") as tmp:
        root = Path(tmp)

        positive_run = root / "positive"
        write_minimal_self_test_run(positive_run, content_checked=True)
        positive_report = validate_run_dir(
            positive_run,
            require_contract_report=True,
            validate_contract_report_content=True,
            require_contract_report_content_checked=True,
        )
        if not positive_report.get("ok"):
            print(
                f"SELF_TEST_FAIL positive:{positive_report.get('findings')}",
                file=sys.stderr,
            )
            return 1

        unchecked_run = root / "unchecked"
        write_minimal_self_test_run(unchecked_run, content_checked=False)
        unchecked_report = validate_run_dir(
            unchecked_run,
            require_contract_report=True,
            validate_contract_report_content=True,
            require_contract_report_content_checked=True,
        )
        unchecked_findings = set(unchecked_report.get("findings") or [])
        if (
            unchecked_report.get("ok")
            or "contract_report_content_checked_not_true:False" not in unchecked_findings
        ):
            print(
                f"SELF_TEST_FAIL unchecked:{unchecked_report.get('findings')}",
                file=sys.stderr,
            )
            return 1

        mismatch_run = root / "summary-mismatch"
        write_minimal_self_test_run(
            mismatch_run,
            content_checked=True,
            stored_summary_override={"status": "error"},
        )
        mismatch_report = validate_run_dir(
            mismatch_run,
            require_contract_report=True,
            validate_contract_report_content=True,
            require_contract_report_content_checked=True,
        )
        mismatch_findings = set(mismatch_report.get("findings") or [])
        if mismatch_report.get("ok") or "contract_report_summary_mismatch" not in mismatch_findings:
            print(
                f"SELF_TEST_FAIL summary_mismatch:{mismatch_report.get('findings')}",
                file=sys.stderr,
            )
            return 1

        agent_contract_run = root / "agent-contract-mismatch"
        write_minimal_self_test_run(
            agent_contract_run,
            content_checked=True,
            stored_agent_contract={"checked": False, "content_check_count": 0},
        )
        agent_contract_findings = validate_existing_contract_report(
            agent_contract_run,
            {
                "summary": {
                    "suite": "manual",
                    "status": "ok",
                    "exit_code": "0",
                    "artifact_finalize_status": "ok",
                    "run_log": "run.log",
                    "artifact_index": "artifact_index.txt",
                },
                "agent_parity_gate_contract": {
                    "checked": True,
                    "content_check_count": 1,
                },
            },
            require_content_checked=True,
        )
        if "contract_report_agent_parity_contract_mismatch" not in set(
            agent_contract_findings
        ):
            print(
                f"SELF_TEST_FAIL agent_contract_mismatch:{agent_contract_findings}",
                file=sys.stderr,
            )
            return 1

        unexpected_agent_contract_run = root / "unexpected-agent-contract"
        write_minimal_self_test_run(
            unexpected_agent_contract_run,
            content_checked=True,
            stored_agent_contract={"checked": True, "content_check_count": 1},
        )
        unexpected_agent_contract_findings = validate_existing_contract_report(
            unexpected_agent_contract_run,
            {
                "summary": {
                    "suite": "manual",
                    "status": "ok",
                    "exit_code": "0",
                    "artifact_finalize_status": "ok",
                    "run_log": "run.log",
                    "artifact_index": "artifact_index.txt",
                },
            },
            require_content_checked=True,
        )
        if "contract_report_unexpected_agent_parity_contract" not in set(
            unexpected_agent_contract_findings
        ):
            print(
                "SELF_TEST_FAIL unexpected_agent_contract:"
                f"{unexpected_agent_contract_findings}",
                file=sys.stderr,
            )
            return 1

        base_field_cases = (
            (
                "not-ok",
                {"ok": False},
                "contract_report_not_ok:False",
            ),
            (
                "bad-run-dir",
                {"run_dir": "absolute-or-host-path"},
                "contract_report_bad_run_dir:absolute-or-host-path",
            ),
            (
                "bad-require-contract-report",
                {"require_contract_report": False},
                "contract_report_bad_require_contract_report:False",
            ),
            (
                "findings-not-empty",
                {"findings": ["contract_report_pending"]},
                "contract_report_findings_not_empty",
            ),
        )
        for label, stored_report_override, expected_finding in base_field_cases:
            case_run = root / label
            write_minimal_self_test_run(
                case_run,
                content_checked=True,
                stored_report_override=stored_report_override,
            )
            case_findings = validate_existing_contract_report(
                case_run,
                {
                    "summary": {
                        "suite": "manual",
                        "status": "ok",
                        "exit_code": "0",
                        "artifact_finalize_status": "ok",
                        "run_log": "run.log",
                        "artifact_index": "artifact_index.txt",
                    },
                },
                require_content_checked=True,
            )
            if expected_finding not in set(case_findings):
                print(
                    f"SELF_TEST_FAIL {label}:{case_findings}",
                    file=sys.stderr,
                )
                return 1

    print("SUITE_ARTIFACT_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("run_dir", type=Path, nargs="?")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--require-contract-report", action="store_true")
    parser.add_argument("--validate-contract-report-content", action="store_true")
    parser.add_argument("--require-contract-report-content-checked", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()
    if args.run_dir is None:
        parser.error("run_dir is required unless --self-test is used")

    validate_contract_report_content = (
        args.validate_contract_report_content or args.require_contract_report_content_checked
    )
    require_contract_report = args.require_contract_report or validate_contract_report_content
    report = validate_run_dir(
        args.run_dir,
        require_contract_report=require_contract_report,
        validate_contract_report_content=validate_contract_report_content,
        require_contract_report_content_checked=args.require_contract_report_content_checked,
    )
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    elif report["ok"]:
        summary = report.get("summary") or {}
        print(
            "SUITE_ARTIFACT_CONTRACT ok "
            f"suite={summary.get('suite')} status={summary.get('status')}"
        )
    else:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True), file=sys.stderr)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
