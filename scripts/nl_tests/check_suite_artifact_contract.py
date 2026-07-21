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
HOST_PATH_MARKERS = ("/home/", "/tmp/", "/root/", "/Users/")

AGENT_PARITY_GATE_REQUIRED_ARTIFACTS = {
    "agent_parity_gate/gate_summary.env",
    "agent_parity_gate/runtime_hard_reply_baseline.txt",
    "agent_parity_gate/policy_boundary_hard_reply.txt",
    "agent_parity_gate/repair_no_user_text_fields.txt",
    "agent_parity_gate/policy_decision_tokens.txt",
    "agent_parity_gate/agent_loop_guard_final_scope.txt",
    "agent_parity_gate/registry_policy_contracts.txt",
    "agent_parity_gate/skill_registry_aliases.txt",
    "agent_parity_gate/long_tail_skill_contracts.txt",
    "agent_parity_gate/task_lifecycle_contracts.txt",
    "agent_parity_gate/task_event_context_team_contracts.txt",
    "agent_parity_gate/clawcli_exec_replay_contracts.txt",
    "agent_parity_gate/clawcli_session_tui_contracts.txt",
    "agent_parity_gate/clawcli_goal_contracts.txt",
    "agent_parity_gate/clawcli_llm_trace_contracts.txt",
    "agent_parity_gate/clawcli_models_catalog_contracts.txt",
    "agent_parity_gate/clawcli_models_readiness_contracts.txt",
    "agent_parity_gate/no_agent_mode_payload.txt",
    "agent_parity_gate/agent_loop_static_contracts.txt",
    "agent_parity_gate/planner_runtime_boundary_contracts.txt",
    "agent_parity_gate/agent_architecture_boundary_contracts.txt",
    "agent_parity_gate/deterministic_boundary_inventory_contracts.txt",
    "agent_parity_gate/maintainability_skill_contracts.txt",
    "agent_parity_gate/agent_parity_gate_inventory_contracts.txt",
    "agent_parity_gate/evidence_extractor_contracts.txt",
    "agent_parity_gate/secret_scan_contract_self_test.txt",
    "agent_parity_gate/secret_scan_contract.json",
    "agent_parity_gate/suite_wrapper_contract.json",
    "agent_parity_gate/runner_path_ref_contract.json",
    "agent_parity_gate/nl_suite_checker_self_tests.txt",
    "agent_parity_gate/suite_artifact_contract_self_test.txt",
    "agent_parity_gate/llm_raw_trace_runner_contract.txt",
    "agent_parity_gate/rollout_metrics_contract.txt",
}

AGENT_PARITY_GATE_REQUIRED_FLAGS = {
    "runtime_hard_reply_baseline": "1",
    "policy_boundary_hard_reply": "1",
    "repair_no_user_text_fields": "1",
    "policy_decision_tokens": "1",
    "agent_loop_guard_final_scope": "1",
    "registry_policy_contracts": "1",
    "skill_registry_aliases": "1",
    "long_tail_skill_contracts": "1",
    "task_lifecycle_contracts": "1",
    "task_event_context_team_contracts": "1",
    "clawcli_exec_replay_contracts": "1",
    "clawcli_session_tui_contracts": "1",
    "clawcli_goal_contracts": "1",
    "clawcli_llm_trace_contracts": "1",
    "clawcli_models_catalog_contracts": "1",
    "clawcli_models_readiness_contracts": "1",
    "no_agent_mode_payload": "1",
    "agent_loop_static_contracts": "1",
    "planner_runtime_boundary_contracts": "1",
    "agent_architecture_boundary_contracts": "1",
    "deterministic_boundary_inventory_contracts": "1",
    "maintainability_skill_contracts": "1",
    "agent_parity_gate_inventory_contracts": "1",
    "evidence_extractor_contracts": "1",
    "secret_scan_contract_self_test": "1",
    "secret_scan_contract": "1",
    "suite_wrapper_contract": "1",
    "runner_path_ref_contract": "1",
    "nl_suite_checker_self_tests": "1",
    "suite_artifact_contract_self_test": "1",
    "llm_raw_trace_runner_contract": "1",
    "rollout_metrics_contract": "1",
}

AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS = {
    "live_metrics": {"0", "1"},
}

AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS = {
    "chinese_provider_live_providers",
    "chinese_provider_env_file_state",
    "chinese_provider_env_file_source",
}

AGENT_PARITY_GATE_ENV_FILE_STATE_VALUES = {"present", "missing", "disabled"}
AGENT_PARITY_GATE_ENV_FILE_SOURCE_VALUES = {"default", "explicit", "disabled"}

AGENT_PARITY_GATE_TEXT_CONTENT_TOKENS = {
    "agent_parity_gate/runtime_hard_reply_baseline.txt": {
        "SELF_TEST_OK",
        "RUNTIME_HARD_REPLY_ALL_SCAN",
        "new=0",
    },
    "agent_parity_gate/policy_boundary_hard_reply.txt": {
        "POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok",
        "POLICY_BOUNDARY_HARD_REPLY_CHECK ok",
    },
    "agent_parity_gate/repair_no_user_text_fields.txt": {
        "SELF_TEST_OK",
        "REPAIR_USER_TEXT_FIELD_CHECK ok",
    },
    "agent_parity_gate/policy_decision_tokens.txt": {
        "POLICY_DECISION_TOKEN_SELF_TEST ok",
        "POLICY_DECISION_TOKEN_CHECK ok",
    },
    "agent_parity_gate/agent_loop_guard_final_scope.txt": {
        "AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok",
        "AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0",
    },
    "agent_parity_gate/registry_policy_contracts.txt": {
        "REGISTRY_POLICY_CONTRACT_SELF_TEST ok",
        "REGISTRY_POLICY_CONTRACT_CHECK ok",
    },
    "agent_parity_gate/skill_registry_aliases.txt": {
        "SKILL_REGISTRY_ALIAS_SELF_TEST ok",
        "SKILL_REGISTRY_ALIAS_CHECK ok",
    },
    "agent_parity_gate/long_tail_skill_contracts.txt": {
        "LONG_TAIL_SKILL_CONTRACT_SELF_TEST ok",
        "LONG_TAIL_SKILL_CONTRACT_CHECK ok",
    },
    "agent_parity_gate/task_lifecycle_contracts.txt": {
        "TASK_LIFECYCLE_CONTRACT_SELF_TEST ok",
        "TASK_LIFECYCLE_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/task_event_context_team_contracts.txt": {
        "TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok",
        "TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/clawcli_exec_replay_contracts.txt": {
        "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/clawcli_session_tui_contracts.txt": {
        "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok",
        "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/clawcli_goal_contracts.txt": {
        "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok",
        "CLAWCLI_GOAL_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/clawcli_llm_trace_contracts.txt": {
        "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok",
        "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/clawcli_models_catalog_contracts.txt": {
        "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/clawcli_models_readiness_contracts.txt": {
        "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/no_agent_mode_payload.txt": {
        "SELF_TEST_OK",
        "NO_AGENT_MODE_PAYLOAD ok",
    },
    "agent_parity_gate/agent_loop_static_contracts.txt": {
        "AGENT_LOOP_STATIC_SELF_TEST check_route_authority_legacy_keys.py",
        "AGENT_LOOP_STATIC_SELF_TEST check_legacy_route_boundary.py",
        "AGENT_LOOP_STATIC_SELF_TEST check_pre_planner_exit_inventory.py",
        "AGENT_LOOP_STATIC_SELF_TEST check_frontdoor_boundary_dispatch.py",
        "AGENT_LOOP_STATIC_SELF_TEST check_no_nl_hardmatch.py",
        "AGENT_LOOP_STATIC_SELF_TEST check_historical_hardcoded_language.py",
        "ROUTE_AUTHORITY_LEGACY_KEY_CHECK findings=0",
        "LEGACY_ROUTE_BOUNDARY_CHECK findings=0",
        "PRE_PLANNER_EXIT_REMOVAL_CHECK findings=0",
        "FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0",
        "NL_HARDMATCH_SCAN unknown=0 known_legacy=0",
        "HISTORICAL_HARDCODED_LANGUAGE_SCAN total=",
    },
    "agent_parity_gate/planner_runtime_boundary_contracts.txt": {
        "SELF_TEST_OK",
        "PLANNER_RUNTIME_BOUNDARY_CHECK findings=0",
        "CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0",
        "ROUTE_REASON_MARKER_FACADE_SELF_TEST ok",
        "ROUTE_REASON_MARKER_FACADE_CHECK findings=0",
        "FINALIZER_ARCHITECTURE_SELF_TEST ok",
        "FINALIZER_ARCHITECTURE_CHECK findings=0",
        "zero_domain_hits=0",
        "registry_dependencies=0",
    },
    "agent_parity_gate/agent_architecture_boundary_contracts.txt": {
        "SELF_TEST_OK",
        "BOUNDARY_ENVELOPE_SCHEMA_CHECK findings=0",
        "PLANNER_PRE_LLM_DETERMINISTIC_FAST_PATH_CHECK strict_tests=false findings=0",
        "CAPABILITY_RESOLVER_REGISTRY_ONLY_CHECK findings=0",
        "FINALIZER_BOUNDARY_CHECK ok",
        "EVIDENCE_POLICY_FACADE_BOUNDARY_CHECK strict=false findings=0",
    },
    "agent_parity_gate/deterministic_boundary_inventory_contracts.txt": {
        "SELF_TEST_OK",
        "ANSWER_VERIFIER_BOUNDARY_CHECK ok",
        "OBSERVED_OUTPUT_BOUNDARY_CHECK ok",
        "DETERMINISTIC_DECISION_INVENTORY_CHECK ok",
        "REPAIR_BOUNDARY_INVENTORY_CHECK ok",
        "REPAIR_BOUNDARY_INVENTORY_COVERAGE_CHECK required=",
        "missing=0",
    },
    "agent_parity_gate/maintainability_skill_contracts.txt": {
        "LONG_FILE_CHECK ok",
        "OK: all",
        "registry skills have a generated layered prompt body",
        "REGISTRY_PARITY mode=all",
        "differences=0",
        "MCP_RUNTIME_CONTRACT_SELF_TEST ok",
        "MCP_RUNTIME_CONTRACT_CHECK findings=0",
        "AGENT_HOOK_RUNTIME_CONTRACT_SELF_TEST ok",
        "AGENT_HOOK_RUNTIME_CONTRACT_CHECK findings=0",
        "CONTEXT_COMPACTION_RUNTIME_CONTRACT_SELF_TEST ok",
        "CONTEXT_COMPACTION_RUNTIME_CONTRACT_CHECK findings=0",
        "TOOL_OUTPUT_ARTIFACT_CONTRACT_SELF_TEST ok",
        "TOOL_OUTPUT_ARTIFACT_CONTRACT_CHECK findings=0",
        "CODE_INDEX_CONTRACT_SELF_TEST ok",
        "CODE_INDEX_CONTRACT_CHECK findings=0",
        "FINALIZER_ARCHITECTURE_SELF_TEST ok",
        "FINALIZER_ARCHITECTURE_CHECK findings=0",
    },
    "agent_parity_gate/agent_parity_gate_inventory_contracts.txt": {
        "AGENT_PARITY_GATE_INVENTORY_SELF_TEST ok",
        "AGENT_PARITY_GATE_INVENTORY_CHECK ok",
        "NL_TEST_CHECKER_INVENTORY_SELF_TEST ok",
        "NL_TEST_CHECKER_INVENTORY_CHECK ok",
    },
    "agent_parity_gate/evidence_extractor_contracts.txt": {
        "EVIDENCE_EXTRACTOR_CONTRACT_SELF_TEST ok",
        "EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings=0",
    },
    "agent_parity_gate/secret_scan_contract_self_test.txt": {
        "SECRET_SCAN_CONTRACT_SELF_TEST ok",
    },
    "agent_parity_gate/nl_suite_checker_self_tests.txt": {
        "SUITE_WRAPPER_CONTRACT_SELF_TEST ok",
        "RUNNER_PATH_REF_CONTRACT_SELF_TEST ok",
        "COMPACT_COVERAGE_SELF_TEST ok",
    },
    "agent_parity_gate/llm_raw_trace_runner_contract.txt": {
        "SELF_TEST_OK",
        "LLM_RAW_TRACE_RUNNER_CONTRACT_SELF_TEST ok",
        "LLM_RAW_TRACE_RUNNER_CONTRACT ok",
    },
    "agent_parity_gate/suite_artifact_contract_self_test.txt": {
        "SUITE_ARTIFACT_CONTRACT_SELF_TEST ok",
    },
    "agent_parity_gate/rollout_metrics_contract.txt": {
        "ROLLOUT_METRICS_SELF_TEST ok",
    },
}

AGENT_PARITY_GATE_JSON_OK_ARTIFACTS = {
    "agent_parity_gate/secret_scan_contract.json",
    "agent_parity_gate/suite_wrapper_contract.json",
    "agent_parity_gate/runner_path_ref_contract.json",
}

AGENT_PARITY_GATE_OPTIONAL_ARTIFACTS_BY_FLAG = {
    "coverage": {
        "agent_parity_gate/compact_coverage.json",
    },
    "model_catalog": {
        "agent_parity_gate/chinese_model_catalog_self_test.txt",
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
    except UnicodeDecodeError:
        return values, ["summary_decode_failed"]
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
    except UnicodeDecodeError:
        return set(), [f"artifact_index_decode_failed:{artifact_index_rel}"]

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
    except UnicodeDecodeError:
        return ["contract_report_decode_failed"]
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


def validate_text_artifact_no_host_paths(run_dir: Path, rel_path: str) -> list[str]:
    text, findings = read_text_artifact(run_dir / rel_path, rel_path)
    if findings:
        return findings
    if any(marker in text for marker in HOST_PATH_MARKERS):
        findings.append(f"agent_parity_gate_artifact_host_path:{rel_path}")
    return findings


def validate_json_artifact_ok(run_dir: Path, rel_path: str) -> list[str]:
    text, findings = read_text_artifact(run_dir / rel_path, rel_path)
    if findings:
        return findings
    try:
        payload = json.loads(text)
    except json.JSONDecodeError:
        return [f"agent_parity_gate_artifact_bad_json:{rel_path}"]
    if not isinstance(payload, dict):
        return [f"agent_parity_gate_artifact_bad_shape:{rel_path}"]
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
        payload = json.loads(text)
    except json.JSONDecodeError:
        return None, [f"agent_parity_gate_artifact_bad_json:{rel_path}"]
    if not isinstance(payload, dict):
        return None, [f"agent_parity_gate_artifact_bad_shape:{rel_path}"]
    return payload, []


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


def provider_smoke_path_ref_is_safe(value: Any, *, allow_empty: bool = False) -> bool:
    if not isinstance(value, str):
        return False
    if value == "":
        return allow_empty
    if value.startswith("/") or "\\" in value or any(ch.isspace() for ch in value):
        return False
    path = PurePosixPath(value)
    return all(part not in {"", ".", ".."} for part in path.parts)


def validate_gate_summary_no_host_paths(gate_summary: dict[str, str]) -> tuple[list[str], int]:
    findings: list[str] = []
    for key, value in sorted(gate_summary.items()):
        if any(marker in value for marker in HOST_PATH_MARKERS):
            findings.append(f"agent_parity_gate_summary_host_path:{key}")
    if "out_dir" in gate_summary:
        findings.append("agent_parity_gate_summary_legacy_out_dir")
    out_dir_ref = gate_summary.get("out_dir_ref")
    if not provider_smoke_path_ref_is_safe(out_dir_ref):
        findings.append(f"agent_parity_gate_summary_bad_out_dir_ref:{out_dir_ref}")
    return findings, 1


def validate_provider_smoke_path_refs(
    rows: list[dict[str, Any]],
    source: str,
) -> list[str]:
    findings: list[str] = []
    for index, row in enumerate(rows):
        for field in ("case_file", "output_file", "run_dir"):
            if not provider_smoke_path_ref_is_safe(row.get(field), allow_empty=(field == "run_dir")):
                findings.append(
                    f"agent_parity_gate_provider_smoke_bad_path_ref:{source}:{index}:{field}"
                )
    return findings


def parse_live_provider_scope(raw: str | None) -> tuple[set[str], list[str]]:
    if raw is None:
        return set(), ["agent_parity_gate_summary_missing_live_provider_scope"]
    normalized = raw.strip().lower()
    if normalized == "all":
        return set(AGENT_PARITY_CHINESE_MODEL_PROVIDERS), []
    if not normalized:
        return set(), ["agent_parity_gate_summary_bad_live_provider_scope"]
    providers = [item.strip() for item in normalized.split(",")]
    if (
        any(not item for item in providers)
        or any(not MACHINE_TOKEN_RE.fullmatch(item) for item in providers)
        or any(item not in AGENT_PARITY_CHINESE_MODEL_PROVIDERS for item in providers)
    ):
        return set(), ["agent_parity_gate_summary_bad_live_provider_scope"]
    return set(providers), []


def validate_live_provider_scope(gate_summary: dict[str, str]) -> tuple[list[str], int]:
    _, findings = parse_live_provider_scope(gate_summary.get("chinese_provider_live_providers"))
    return findings, 1


def validate_chinese_provider_env_file_summary(
    gate_summary: dict[str, str],
) -> tuple[list[str], int]:
    findings: list[str] = []
    state = gate_summary.get("chinese_provider_env_file_state")
    source = gate_summary.get("chinese_provider_env_file_source")
    if state not in AGENT_PARITY_GATE_ENV_FILE_STATE_VALUES:
        findings.append(f"agent_parity_gate_summary_bad_env_file_state:{state}")
    if source not in AGENT_PARITY_GATE_ENV_FILE_SOURCE_VALUES:
        findings.append(f"agent_parity_gate_summary_bad_env_file_source:{source}")
    return findings, 2


def expected_live_scope_providers(gate_summary: dict[str, str]) -> set[str]:
    providers, findings = parse_live_provider_scope(gate_summary.get("chinese_provider_live_providers"))
    if findings:
        return set()
    return providers


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
    catalog_rows = payload.get("catalog", [])
    if not isinstance(catalog_rows, list):
        findings.append("agent_parity_gate_chinese_model_catalog_bad_catalog_shape")
        catalog_rows = []
    providers = {
        entry.get("provider")
        for entry in catalog_rows
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
    if not provider_smoke_path_ref_is_safe(payload.get("case_file")):
        findings.append("agent_parity_gate_provider_smoke_case_coverage_bad_case_file")
    if payload.get("missing_coverage_tags") != []:
        findings.append("agent_parity_gate_provider_smoke_case_coverage_missing_tags")
    if payload.get("missing_provider_tags") != []:
        findings.append("agent_parity_gate_provider_smoke_case_coverage_missing_provider_tags")
    if payload.get("forbidden_live_tag_hits") != []:
        findings.append("agent_parity_gate_provider_smoke_case_coverage_forbidden_live_tags")
    provider_tags_raw = payload.get("provider_tags") or []
    if not isinstance(provider_tags_raw, list):
        findings.append("agent_parity_gate_provider_smoke_case_coverage_bad_provider_tags")
        provider_tags_raw = []
    provider_tags = set(str(tag) for tag in provider_tags_raw)
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
        provider_rows_raw = payload.get("providers", [])
        if not isinstance(provider_rows_raw, list):
            findings.append("agent_parity_gate_provider_smoke_bad_providers_shape")
            provider_rows_raw = []
        provider_rows = [entry for entry in provider_rows_raw if isinstance(entry, dict)]
        findings.extend(validate_provider_smoke_path_refs(provider_rows, "matrix_summary"))
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
        findings.extend(validate_provider_smoke_path_refs(rows, "provider_summary"))
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
        required_tokens = {
            "CHINESE_PROVIDER_SMOKE_MATRIX_SELF_TEST ok",
            "CHINESE_PROVIDER_SMOKE_SUMMARY_SELF_TEST ok",
            "CHINESE_PROVIDER_SMOKE_SUMMARY_CHECK ok",
        }
        checks += len(required_tokens)
        for token in sorted(required_tokens):
            if token not in text:
                findings.append(f"agent_parity_gate_provider_smoke_missing_runner_token:{token}")
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
    if any(marker in json.dumps(payload, ensure_ascii=False) for marker in HOST_PATH_MARKERS):
        findings.append(f"agent_parity_gate_metrics_host_path:{rel_path}")
    if not provider_smoke_path_ref_is_safe(payload.get("source_run_dir")):
        findings.append(f"agent_parity_gate_metrics_bad_source_run_dir:{rel_path}")
    source_run_dirs = payload.get("source_run_dirs")
    if source_run_dirs is not None:
        if not isinstance(source_run_dirs, list) or any(
            not provider_smoke_path_ref_is_safe(item) for item in source_run_dirs
        ):
            findings.append(f"agent_parity_gate_metrics_bad_source_run_dirs:{rel_path}")
    min_pass_rate = safe_float(gate_summary.get("min_pass_rate"), 1.0)
    max_avg_llm_calls = safe_float(gate_summary.get("max_avg_llm_calls"), 4.0)
    max_prompt_truncations = safe_int(gate_summary.get("max_prompt_truncations"), 0)
    max_provider_final_errors = safe_int(gate_summary.get("max_provider_final_errors"), 0)
    turns_total = safe_int(payload.get("turns_total"))
    checks += 10
    if turns_total <= 0:
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

    capability_outcomes = payload.get("capability_outcomes")
    checks += 1
    if not isinstance(capability_outcomes, dict):
        findings.append(f"agent_parity_gate_metrics_missing_capability_outcomes:{rel_path}")
    else:
        for capability, outcome in capability_outcomes.items():
            checks += 1
            if not isinstance(capability, str) or not capability or not isinstance(outcome, dict):
                findings.append(
                    f"agent_parity_gate_metrics_bad_capability_outcome:{rel_path}:{capability}"
                )
                continue
            required_outcome_fields = {"total", "ok", "error", "other", "success_rate"}
            if not required_outcome_fields.issubset(outcome):
                findings.append(
                    f"agent_parity_gate_metrics_incomplete_capability_outcome:{rel_path}:{capability}"
                )
                continue
            total = safe_int(outcome.get("total"), -1)
            ok = safe_int(outcome.get("ok"), -1)
            error = safe_int(outcome.get("error"), -1)
            other = safe_int(outcome.get("other"), -1)
            if min(total, ok, error, other) < 0 or total != ok + error + other:
                findings.append(
                    f"agent_parity_gate_metrics_inconsistent_capability_outcome:{rel_path}:{capability}"
                )

    action_family_counts = payload.get("action_family_counts")
    checks += 1
    if not isinstance(action_family_counts, dict) or any(
        not isinstance(key, str) or not key or safe_int(value, -1) < 0
        for key, value in (
            action_family_counts.items() if isinstance(action_family_counts, dict) else ()
        )
    ):
        findings.append(f"agent_parity_gate_metrics_bad_action_family_counts:{rel_path}")

    side_effects = payload.get("side_effects")
    checks += 1
    if (
        not isinstance(side_effects, dict)
        or safe_int(side_effects.get("completed_total"), -1) < 0
        or not isinstance(side_effects.get("turn_counts"), dict)
    ):
        findings.append(f"agent_parity_gate_metrics_bad_side_effects:{rel_path}")
    elif sum(safe_int(value, -1) for value in side_effects["turn_counts"].values()) != turns_total:
        findings.append(f"agent_parity_gate_metrics_inconsistent_side_effect_turns:{rel_path}")

    repairs = payload.get("repairs")
    checks += 1
    if (
        not isinstance(repairs, dict)
        or safe_int(repairs.get("attempt_total"), -1) < 0
        or not isinstance(repairs.get("turn_counts"), dict)
        or not isinstance(repairs.get("signal_status_counts"), dict)
    ):
        findings.append(f"agent_parity_gate_metrics_bad_repairs:{rel_path}")
    elif sum(safe_int(value, -1) for value in repairs["turn_counts"].values()) != turns_total:
        findings.append(f"agent_parity_gate_metrics_inconsistent_repair_turns:{rel_path}")

    llm = payload.get("llm")
    required_llm_metrics = {
        "total_calls",
        "total_elapsed_ms",
        "prompt_bytes_before_max",
        "prompt_bytes_after_max",
        "prompt_truncated_bytes_total",
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "avg_calls_per_turn",
    }
    checks += 1
    if not isinstance(llm, dict) or not required_llm_metrics.issubset(llm):
        findings.append(f"agent_parity_gate_metrics_incomplete_llm_metrics:{rel_path}")

    execution = payload.get("execution")
    checks += 1
    if not isinstance(execution, dict) or "tool_call_count" not in execution:
        findings.append(f"agent_parity_gate_metrics_incomplete_execution_metrics:{rel_path}")

    wall_time = payload.get("wall_time")
    checks += 1
    if not isinstance(wall_time, dict):
        findings.append(f"agent_parity_gate_metrics_missing_wall_time:{rel_path}")
    else:
        recording_status = wall_time.get("recording_status")
        recorded_turns = safe_int(wall_time.get("recorded_turns"), -1)
        missing_turns = safe_int(wall_time.get("missing_turns"), -1)
        required_wall_fields = {
            "recording_status",
            "recorded_turns",
            "missing_turns",
            "total_ms",
            "avg_ms",
            "max_ms",
        }
        if (
            not required_wall_fields.issubset(wall_time)
            or recording_status not in {"complete", "partial", "not_recorded"}
            or min(
                recorded_turns,
                missing_turns,
                safe_int(wall_time.get("total_ms"), -1),
                safe_int(wall_time.get("max_ms"), -1),
            )
            < 0
            or recorded_turns + missing_turns != turns_total
            or (recording_status == "complete" and missing_turns != 0)
            or (recording_status == "partial" and (recorded_turns == 0 or missing_turns == 0))
            or (recording_status == "not_recorded" and recorded_turns != 0)
        ):
            findings.append(f"agent_parity_gate_metrics_inconsistent_wall_time:{rel_path}")
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
    text_path_findings = validate_text_artifact_no_host_paths(
        run_dir,
        "agent_parity_gate/coding_loop_repair_metrics.txt",
    )
    findings.extend(text_path_findings)
    checks += 0 if text_path_findings else 1
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
        self_test_findings = validate_text_artifact_tokens(
            run_dir,
            "agent_parity_gate/chinese_model_catalog_self_test.txt",
            {"CHINESE_MODEL_CATALOG_SELF_TEST ok"},
        )
        findings.extend(self_test_findings)
        checks += 0 if self_test_findings else 1
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
    live_metrics_enabled = gate_summary.get("live_metrics") == "1"
    if live_metrics_enabled:
        for rel_path, token in (
            ("agent_parity_gate/run_metrics.txt", "ROLLOUT_METRICS_OK"),
        ):
            artifact_findings = validate_text_artifact_tokens(run_dir, rel_path, {token})
            findings.extend(artifact_findings)
            checks += 0 if artifact_findings else 1
            text_path_findings = validate_text_artifact_no_host_paths(run_dir, rel_path)
            findings.extend(text_path_findings)
            checks += 0 if text_path_findings else 1
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
        return findings, content_checks

    gate_summary, parse_findings = parse_env_file(summary_path)
    findings.extend(f"agent_parity_gate_{finding}" for finding in parse_findings)
    summary_path_findings, summary_path_checks = validate_gate_summary_no_host_paths(
        gate_summary
    )
    findings.extend(summary_path_findings)
    content_checks += summary_path_checks
    run_log_path_findings = validate_text_artifact_no_host_paths(run_dir, "run.log")
    findings.extend(run_log_path_findings)
    content_checks += 0 if run_log_path_findings else 1
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
    scope_findings, scope_checks = validate_live_provider_scope(gate_summary)
    findings.extend(scope_findings)
    content_checks += scope_checks
    env_file_findings, env_file_checks = validate_chinese_provider_env_file_summary(gate_summary)
    findings.extend(env_file_findings)
    content_checks += env_file_checks
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
            "required_machine_field_count": len(AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS)
            + len(AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS),
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

        summary_decode_run = root / "summary-decode-failed"
        write_minimal_self_test_run(summary_decode_run, content_checked=True)
        (summary_decode_run / "suite_summary.env").write_bytes(b"\xff\n")
        summary_decode_report = validate_run_dir(summary_decode_run)
        if "summary_decode_failed" not in set(summary_decode_report.get("findings") or []):
            print(
                f"SELF_TEST_FAIL summary_decode_failed:{summary_decode_report.get('findings')}",
                file=sys.stderr,
            )
            return 1

        artifact_index_decode_run = root / "artifact-index-decode-failed"
        write_minimal_self_test_run(artifact_index_decode_run, content_checked=True)
        (artifact_index_decode_run / "artifact_index.txt").write_bytes(b"\xff\n")
        artifact_index_decode_report = validate_run_dir(artifact_index_decode_run)
        if (
            "artifact_index_decode_failed:artifact_index.txt"
            not in set(artifact_index_decode_report.get("findings") or [])
        ):
            print(
                "SELF_TEST_FAIL artifact_index_decode_failed:"
                f"{artifact_index_decode_report.get('findings')}",
                file=sys.stderr,
            )
            return 1

        contract_report_decode_run = root / "contract-report-decode-failed"
        contract_report_decode_summary = write_minimal_self_test_run(
            contract_report_decode_run,
            content_checked=True,
        )
        (contract_report_decode_run / "suite_artifact_contract.json").write_bytes(b"\xff\n")
        contract_report_decode_findings = validate_existing_contract_report(
            contract_report_decode_run,
            {"summary": contract_report_decode_summary},
            require_content_checked=True,
        )
        if "contract_report_decode_failed" not in set(contract_report_decode_findings):
            print(
                "SELF_TEST_FAIL contract_report_decode_failed:"
                f"{contract_report_decode_findings}",
                file=sys.stderr,
            )
            return 1

        live_scope_cases = (
            ({"chinese_provider_live_providers": "minimax"}, set(), {"minimax"}),
            (
                {"chinese_provider_live_providers": "minimax,qwen"},
                set(),
                {"minimax", "qwen"},
            ),
            (
                {"chinese_provider_live_providers": "all"},
                set(),
                set(AGENT_PARITY_CHINESE_MODEL_PROVIDERS),
            ),
            (
                {"chinese_provider_live_providers": ""},
                {"agent_parity_gate_summary_bad_live_provider_scope"},
                set(),
            ),
            (
                {"chinese_provider_live_providers": "minimax,unknown"},
                {"agent_parity_gate_summary_bad_live_provider_scope"},
                set(),
            ),
            (
                {"chinese_provider_live_providers": "mini max"},
                {"agent_parity_gate_summary_bad_live_provider_scope"},
                set(),
            ),
            (
                {},
                {"agent_parity_gate_summary_missing_live_provider_scope"},
                set(),
            ),
        )
        for gate_summary, expected_findings, expected_providers in live_scope_cases:
            scope_findings, scope_checks = validate_live_provider_scope(gate_summary)
            scope_provider_set = expected_live_scope_providers(gate_summary)
            if set(scope_findings) != expected_findings or scope_provider_set != expected_providers or scope_checks != 1:
                print(
                    "SELF_TEST_FAIL live_provider_scope:"
                    f"summary={gate_summary} findings={scope_findings} providers={scope_provider_set}",
                    file=sys.stderr,
                )
                return 1

        env_file_summary_cases = (
            (
                {
                    "chinese_provider_env_file_state": "present",
                    "chinese_provider_env_file_source": "default",
                },
                set(),
            ),
            (
                {
                    "chinese_provider_env_file_state": "missing",
                    "chinese_provider_env_file_source": "explicit",
                },
                set(),
            ),
            (
                {
                    "chinese_provider_env_file_state": "disabled",
                    "chinese_provider_env_file_source": "disabled",
                },
                set(),
            ),
            (
                {
                    "chinese_provider_env_file_state": "",
                    "chinese_provider_env_file_source": "default",
                },
                {"agent_parity_gate_summary_bad_env_file_state:"},
            ),
            (
                {
                    "chinese_provider_env_file_state": "present",
                    "chinese_provider_env_file_source": "local-path",
                },
                {"agent_parity_gate_summary_bad_env_file_source:local-path"},
            ),
            (
                {},
                {
                    "agent_parity_gate_summary_bad_env_file_state:None",
                    "agent_parity_gate_summary_bad_env_file_source:None",
                },
            ),
        )
        for gate_summary, expected_findings in env_file_summary_cases:
            env_file_findings, env_file_checks = validate_chinese_provider_env_file_summary(
                gate_summary
            )
            if set(env_file_findings) != expected_findings or env_file_checks != 2:
                print(
                    "SELF_TEST_FAIL env_file_summary:"
                    f"summary={gate_summary} findings={env_file_findings}",
                    file=sys.stderr,
                )
                return 1

        gate_summary_path_cases = (
            (
                {"out_dir_ref": "out_dir"},
                set(),
            ),
            (
                {"out_dir": "/home/user/rustclaw/logs/agent_parity_gate/run"},
                {
                    "agent_parity_gate_summary_host_path:out_dir",
                    "agent_parity_gate_summary_legacy_out_dir",
                    "agent_parity_gate_summary_bad_out_dir_ref:None",
                },
            ),
            (
                {"out_dir_ref": "/tmp/run"},
                {
                    "agent_parity_gate_summary_host_path:out_dir_ref",
                    "agent_parity_gate_summary_bad_out_dir_ref:/tmp/run",
                },
            ),
        )
        for gate_summary, expected_findings in gate_summary_path_cases:
            path_findings, path_checks = validate_gate_summary_no_host_paths(gate_summary)
            if set(path_findings) != expected_findings or path_checks != 1:
                print(
                    "SELF_TEST_FAIL gate-summary-host-path:"
                    f"summary={gate_summary} findings={path_findings}",
                    file=sys.stderr,
                )
                return 1

        run_log_host_path_run = root / "agent-parity-run-log-host-path"
        write_minimal_self_test_run(run_log_host_path_run, content_checked=True)
        (run_log_host_path_run / "run.log").write_text(
            "run_dir: /home/user/rustclaw/scripts/nl_suite_logs/agent_parity_gate/run\n",
            encoding="utf-8",
        )
        run_log_host_path_findings = validate_text_artifact_no_host_paths(
            run_log_host_path_run,
            "run.log",
        )
        if "agent_parity_gate_artifact_host_path:run.log" not in set(
            run_log_host_path_findings
        ):
            print(
                "SELF_TEST_FAIL agent_parity_run_log_host_path:"
                f"{run_log_host_path_findings}",
                file=sys.stderr,
            )
            return 1

        agent_summary_missing_run = root / "agent-parity-missing-gate-summary"
        write_minimal_self_test_run(agent_summary_missing_run, content_checked=True)
        (agent_summary_missing_run / "suite_summary.env").write_text(
            "\n".join(
                [
                    "suite=agent_parity_gate",
                    "status=ok",
                    "exit_code=0",
                    "artifact_finalize_status=ok",
                    "run_log=run.log",
                    "artifact_index=artifact_index.txt",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        agent_summary_missing_report = validate_run_dir(agent_summary_missing_run)
        agent_summary_missing_findings = set(
            agent_summary_missing_report.get("findings") or []
        )
        if (
            agent_summary_missing_report.get("ok")
            or "agent_parity_gate_summary_missing" not in agent_summary_missing_findings
        ):
            print(
                "SELF_TEST_FAIL agent_parity_missing_gate_summary:"
                f"{agent_summary_missing_report.get('findings')}",
                file=sys.stderr,
            )
            return 1

        json_bad_shape_run = root / "json-ok-artifact-bad-shape"
        write_minimal_self_test_run(json_bad_shape_run, content_checked=True)
        json_bad_shape_rel = "agent_parity_gate/secret_scan_contract.json"
        json_bad_shape_path = json_bad_shape_run / json_bad_shape_rel
        json_bad_shape_path.parent.mkdir(parents=True, exist_ok=True)
        json_bad_shape_path.write_text("[]\n", encoding="utf-8")
        json_bad_shape_findings = validate_json_artifact_ok(
            json_bad_shape_run,
            json_bad_shape_rel,
        )
        if (
            f"agent_parity_gate_artifact_bad_shape:{json_bad_shape_rel}"
            not in set(json_bad_shape_findings)
        ):
            print(
                f"SELF_TEST_FAIL json_ok_artifact_bad_shape:{json_bad_shape_findings}",
                file=sys.stderr,
            )
            return 1

        load_json_bad_shape_run = root / "load-json-artifact-bad-shape"
        write_minimal_self_test_run(load_json_bad_shape_run, content_checked=True)
        load_json_bad_shape_rel = "agent_parity_gate/compact_coverage.json"
        load_json_bad_shape_path = load_json_bad_shape_run / load_json_bad_shape_rel
        load_json_bad_shape_path.parent.mkdir(parents=True, exist_ok=True)
        load_json_bad_shape_path.write_text("[]\n", encoding="utf-8")
        load_json_bad_shape_findings, _ = validate_compact_coverage_artifact(
            load_json_bad_shape_run
        )
        if (
            f"agent_parity_gate_artifact_bad_shape:{load_json_bad_shape_rel}"
            not in set(load_json_bad_shape_findings)
        ):
            print(
                f"SELF_TEST_FAIL load_json_artifact_bad_shape:{load_json_bad_shape_findings}",
                file=sys.stderr,
            )
            return 1

        text_artifact_decode_run = root / "text-artifact-decode-failed"
        write_minimal_self_test_run(text_artifact_decode_run, content_checked=True)
        text_artifact_decode_rel = "agent_parity_gate/agent_loop_static_contracts.txt"
        text_artifact_decode_path = text_artifact_decode_run / text_artifact_decode_rel
        text_artifact_decode_path.parent.mkdir(parents=True, exist_ok=True)
        text_artifact_decode_path.write_bytes(b"\xff\n")
        text_artifact_decode_findings = validate_text_artifact_tokens(
            text_artifact_decode_run,
            text_artifact_decode_rel,
            {"AGENT_LOOP_STATIC_CONTRACTS ok"},
        )
        if (
            f"agent_parity_gate_artifact_decode_failed:{text_artifact_decode_rel}"
            not in set(text_artifact_decode_findings)
        ):
            print(
                f"SELF_TEST_FAIL text_artifact_decode_failed:{text_artifact_decode_findings}",
                file=sys.stderr,
            )
            return 1

        load_json_decode_run = root / "load-json-artifact-decode-failed"
        write_minimal_self_test_run(load_json_decode_run, content_checked=True)
        load_json_decode_rel = "agent_parity_gate/compact_coverage.json"
        load_json_decode_path = load_json_decode_run / load_json_decode_rel
        load_json_decode_path.parent.mkdir(parents=True, exist_ok=True)
        load_json_decode_path.write_bytes(b"\xff\n")
        load_json_decode_findings, _ = validate_compact_coverage_artifact(
            load_json_decode_run
        )
        if (
            f"agent_parity_gate_artifact_decode_failed:{load_json_decode_rel}"
            not in set(load_json_decode_findings)
        ):
            print(
                f"SELF_TEST_FAIL load_json_artifact_decode_failed:{load_json_decode_findings}",
                file=sys.stderr,
            )
            return 1

        provider_summary_jsonl_run = root / "provider-summary-jsonl-row-errors"
        write_minimal_self_test_run(provider_summary_jsonl_run, content_checked=True)
        provider_summary_jsonl_path = (
            provider_summary_jsonl_run
            / "agent_parity_gate/chinese_provider_smoke/provider_summary.jsonl"
        )
        provider_summary_jsonl_path.parent.mkdir(parents=True, exist_ok=True)
        provider_summary_jsonl_path.write_text(
            "{not-json\n[]\n{\"provider\":\"minimax\"}\n",
            encoding="utf-8",
        )
        provider_summary_rows, provider_summary_findings = parse_provider_summary_jsonl(
            provider_summary_jsonl_run
        )
        provider_summary_finding_set = set(provider_summary_findings)
        if (
            "agent_parity_gate_provider_summary_bad_json_line:1"
            not in provider_summary_finding_set
            or "agent_parity_gate_provider_summary_bad_row:2"
            not in provider_summary_finding_set
            or [row.get("provider") for row in provider_summary_rows] != ["minimax"]
        ):
            print(
                "SELF_TEST_FAIL provider_summary_jsonl_row_errors:"
                f"rows={provider_summary_rows} findings={provider_summary_findings}",
                file=sys.stderr,
            )
            return 1

        provider_path_ref_findings = validate_provider_smoke_path_refs(
            [
                {
                    "case_file": "scripts/nl_tests/cases/cases.txt",
                    "output_file": "/tmp/run.output.txt",
                    "run_dir": "out_dir\\minimax",
                }
            ],
            "self_test",
        )
        provider_path_ref_finding_set = set(provider_path_ref_findings)
        if (
            "agent_parity_gate_provider_smoke_bad_path_ref:self_test:0:output_file"
            not in provider_path_ref_finding_set
            or "agent_parity_gate_provider_smoke_bad_path_ref:self_test:0:run_dir"
            not in provider_path_ref_finding_set
        ):
            print(
                "SELF_TEST_FAIL provider_path_ref_errors:"
                f"{provider_path_ref_findings}",
                file=sys.stderr,
            )
            return 1

        provider_case_coverage_run = root / "provider-case-coverage-bad-provider-tags"
        write_minimal_self_test_run(provider_case_coverage_run, content_checked=True)
        provider_case_coverage_path = (
            provider_case_coverage_run
            / "agent_parity_gate/chinese_provider_smoke/case_coverage.json"
        )
        provider_case_coverage_path.parent.mkdir(parents=True, exist_ok=True)
        provider_case_coverage_path.write_text(
            json.dumps(
                {
                    "ok": True,
                    "missing_coverage_tags": [],
                    "missing_provider_tags": [],
                    "forbidden_live_tag_hits": [],
                    "provider_tags": 1,
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        provider_case_coverage_findings, _ = validate_provider_smoke_case_coverage(
            provider_case_coverage_run
        )
        if (
            "agent_parity_gate_provider_smoke_case_coverage_bad_provider_tags"
            not in set(provider_case_coverage_findings)
        ):
            print(
                "SELF_TEST_FAIL provider_case_coverage_bad_provider_tags:"
                f"{provider_case_coverage_findings}",
                file=sys.stderr,
            )
            return 1

        provider_case_file_run = root / "provider-case-coverage-bad-case-file"
        write_minimal_self_test_run(provider_case_file_run, content_checked=True)
        provider_case_file_path = (
            provider_case_file_run
            / "agent_parity_gate/chinese_provider_smoke/case_coverage.json"
        )
        provider_case_file_path.parent.mkdir(parents=True, exist_ok=True)
        provider_case_file_path.write_text(
            json.dumps(
                {
                    "ok": True,
                    "case_file": "/tmp/cases.txt",
                    "missing_coverage_tags": [],
                    "missing_provider_tags": [],
                    "forbidden_live_tag_hits": [],
                    "provider_tags": sorted(AGENT_PARITY_CHINESE_MODEL_PROVIDERS),
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        provider_case_file_findings, _ = validate_provider_smoke_case_coverage(
            provider_case_file_run
        )
        if (
            "agent_parity_gate_provider_smoke_case_coverage_bad_case_file"
            not in set(provider_case_file_findings)
        ):
            print(
                "SELF_TEST_FAIL provider_case_coverage_bad_case_file:"
                f"{provider_case_file_findings}",
                file=sys.stderr,
            )
            return 1

        model_catalog_shape_run = root / "chinese-model-catalog-bad-catalog-shape"
        write_minimal_self_test_run(model_catalog_shape_run, content_checked=True)
        model_catalog_shape_path = (
            model_catalog_shape_run / "agent_parity_gate/chinese_model_catalog.json"
        )
        model_catalog_shape_path.parent.mkdir(parents=True, exist_ok=True)
        model_catalog_shape_path.write_text(
            json.dumps(
                {
                    "status": "ok",
                    "finding_count": 0,
                    "findings": [],
                    "catalog": 1,
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        model_catalog_shape_findings, _ = validate_chinese_model_catalog_artifact(
            model_catalog_shape_run
        )
        if (
            "agent_parity_gate_chinese_model_catalog_bad_catalog_shape"
            not in set(model_catalog_shape_findings)
        ):
            print(
                "SELF_TEST_FAIL chinese_model_catalog_bad_catalog_shape:"
                f"{model_catalog_shape_findings}",
                file=sys.stderr,
            )
            return 1

        provider_matrix_shape_run = root / "provider-smoke-bad-providers-shape"
        write_minimal_self_test_run(provider_matrix_shape_run, content_checked=True)
        provider_matrix_summary_path = (
            provider_matrix_shape_run
            / "agent_parity_gate/chinese_provider_smoke/matrix_summary.json"
        )
        provider_matrix_summary_path.parent.mkdir(parents=True, exist_ok=True)
        provider_matrix_summary_path.write_text(
            json.dumps(
                {
                    "provider_count": len(AGENT_PARITY_CHINESE_MODEL_PROVIDERS),
                    "providers": 1,
                    "status_counts": {},
                    "reason_code_counts": {},
                    "credential_state_counts": {},
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        provider_matrix_shape_findings, _ = validate_provider_smoke_artifacts(
            provider_matrix_shape_run,
            {"chinese_provider_live_providers": "minimax"},
        )
        if (
            "agent_parity_gate_provider_smoke_bad_providers_shape"
            not in set(provider_matrix_shape_findings)
        ):
            print(
                "SELF_TEST_FAIL provider_smoke_bad_providers_shape:"
                f"{provider_matrix_shape_findings}",
                file=sys.stderr,
            )
            return 1

        metrics_path_run = root / "rollout-metrics-host-path"
        write_minimal_self_test_run(metrics_path_run, content_checked=True)
        metrics_path = metrics_path_run / "agent_parity_gate/coding_loop_repair_metrics.json"
        metrics_path.parent.mkdir(parents=True, exist_ok=True)
        metrics_path.write_text(
            json.dumps(
                {
                    "source_run_dir": "/tmp/client-like-run",
                    "source_run_dirs": ["/tmp/client-like-run"],
                    "turns_total": 1,
                    "pass_rate": 1.0,
                    "parse_errors": 0,
                    "metric_gate": {"passed": True},
                    "llm": {
                        "avg_calls_per_turn": 1.0,
                        "prompt_truncation_count": 0,
                        "provider_final_error_count": 0,
                    },
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        metrics_path_findings, _ = validate_rollout_metrics_artifact(
            metrics_path_run,
            "agent_parity_gate/coding_loop_repair_metrics.json",
            {},
        )
        metrics_path_finding_set = set(metrics_path_findings)
        if (
            "agent_parity_gate_metrics_host_path:agent_parity_gate/coding_loop_repair_metrics.json"
            not in metrics_path_finding_set
            or "agent_parity_gate_metrics_bad_source_run_dir:agent_parity_gate/coding_loop_repair_metrics.json"
            not in metrics_path_finding_set
            or "agent_parity_gate_metrics_bad_source_run_dirs:agent_parity_gate/coding_loop_repair_metrics.json"
            not in metrics_path_finding_set
            or "agent_parity_gate_metrics_missing_capability_outcomes:agent_parity_gate/coding_loop_repair_metrics.json"
            not in metrics_path_finding_set
            or "agent_parity_gate_metrics_missing_wall_time:agent_parity_gate/coding_loop_repair_metrics.json"
            not in metrics_path_finding_set
        ):
            print(
                "SELF_TEST_FAIL rollout_metrics_host_path:"
                f"{metrics_path_findings}",
                file=sys.stderr,
            )
            return 1

        metrics_family_run = root / "rollout-metrics-families"
        write_minimal_self_test_run(metrics_family_run, content_checked=True)
        metrics_family_path = (
            metrics_family_run / "agent_parity_gate/coding_loop_repair_metrics.json"
        )
        metrics_family_path.parent.mkdir(parents=True, exist_ok=True)
        metrics_family_path.write_text(
            json.dumps(
                {
                    "source_run_dir": "scripts/nl_tests/fixtures/client_like_runs/example",
                    "turns_total": 1,
                    "pass_rate": 1.0,
                    "parse_errors": 0,
                    "metric_gate": {"passed": True},
                    "capability_outcomes": {
                        "fs_basic": {
                            "total": 1,
                            "ok": 1,
                            "error": 0,
                            "other": 0,
                            "success_rate": 1.0,
                        }
                    },
                    "action_family_counts": {"fs_basic": 1},
                    "side_effects": {
                        "completed_total": 0,
                        "turn_counts": {"none": 1},
                    },
                    "repairs": {
                        "attempt_total": 0,
                        "turn_counts": {"none": 1},
                        "signal_status_counts": {},
                    },
                    "wall_time": {
                        "recording_status": "not_recorded",
                        "recorded_turns": 0,
                        "missing_turns": 1,
                        "total_ms": 0,
                        "avg_ms": 0.0,
                        "max_ms": 0,
                    },
                    "llm": {
                        "total_calls": 1,
                        "total_elapsed_ms": 1,
                        "avg_calls_per_turn": 1.0,
                        "prompt_truncation_count": 0,
                        "provider_final_error_count": 0,
                        "prompt_bytes_before_max": 1,
                        "prompt_bytes_after_max": 1,
                        "prompt_truncated_bytes_total": 0,
                        "prompt_tokens": 1,
                        "completion_tokens": 1,
                        "total_tokens": 2,
                    },
                    "execution": {"tool_call_count": 1},
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        metrics_family_findings, _ = validate_rollout_metrics_artifact(
            metrics_family_run,
            "agent_parity_gate/coding_loop_repair_metrics.json",
            {},
        )
        if metrics_family_findings:
            print(
                "SELF_TEST_FAIL rollout_metrics_families:"
                f"{metrics_family_findings}",
                file=sys.stderr,
            )
            return 1

        metrics_text_run = root / "rollout-metrics-text-host-path"
        write_minimal_self_test_run(metrics_text_run, content_checked=True)
        metrics_text_path = metrics_text_run / "agent_parity_gate/coding_loop_repair_metrics.txt"
        metrics_text_path.parent.mkdir(parents=True, exist_ok=True)
        metrics_text_path.write_text(
            "ROLLOUT_METRICS_OK output=/tmp/metrics.json turns=1 pass_rate=1.0\n",
            encoding="utf-8",
        )
        metrics_text_findings = validate_text_artifact_no_host_paths(
            metrics_text_run,
            "agent_parity_gate/coding_loop_repair_metrics.txt",
        )
        if (
            "agent_parity_gate_artifact_host_path:agent_parity_gate/coding_loop_repair_metrics.txt"
            not in set(metrics_text_findings)
        ):
            print(
                "SELF_TEST_FAIL rollout_metrics_text_host_path:"
                f"{metrics_text_findings}",
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

        report_shape_cases = (
            (
                "missing-contract-report",
                None,
                "contract_report_missing",
            ),
            (
                "bad-json",
                "{not-json",
                "contract_report_bad_json",
            ),
            (
                "bad-shape",
                "[]",
                "contract_report_bad_shape",
            ),
        )
        for label, stored_report_text, expected_finding in report_shape_cases:
            case_run = root / label
            write_minimal_self_test_run(case_run, content_checked=True)
            report_path = case_run / "suite_artifact_contract.json"
            if stored_report_text is None:
                report_path.unlink()
            else:
                report_path.write_text(stored_report_text + "\n", encoding="utf-8")
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

        read_failed_run = root / "read-failed"
        write_minimal_self_test_run(read_failed_run, content_checked=True)
        read_failed_report_path = read_failed_run / "suite_artifact_contract.json"
        read_failed_report_path.unlink()
        read_failed_report_path.mkdir()
        read_failed_findings = validate_existing_contract_report(
            read_failed_run,
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
        if not any(
            finding.startswith("contract_report_read_failed:")
            for finding in read_failed_findings
        ):
            print(
                f"SELF_TEST_FAIL read_failed:{read_failed_findings}",
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
