#!/usr/bin/env python3
"""Validate wrapped NL suite artifact contracts."""

from __future__ import annotations

import argparse
import json
import re
import sys
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
    "agent_parity_gate/llm_raw_trace_runner_contract.txt",
}

AGENT_PARITY_GATE_REQUIRED_FLAGS = {
    "no_agent_mode_payload": "1",
    "agent_loop_static_contracts": "1",
    "secret_scan_contract": "1",
    "suite_wrapper_contract": "1",
    "llm_raw_trace_runner_contract": "1",
}


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


def validate_agent_parity_gate_artifacts(run_dir: Path, entries: set[str]) -> list[str]:
    findings: list[str] = []
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
        actual = gate_summary.get(key)
        if actual != expected:
            findings.append(f"agent_parity_gate_summary_bad_flag:{key}:{actual}")
    return findings


def validate_run_dir(run_dir: Path, require_contract_report: bool = False) -> dict[str, Any]:
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
        findings.extend(validate_agent_parity_gate_artifacts(run_dir, artifact_entries))
        agent_parity_gate_contract = {
            "checked": True,
            "required_artifact_count": len(AGENT_PARITY_GATE_REQUIRED_ARTIFACTS),
            "required_flag_count": len(AGENT_PARITY_GATE_REQUIRED_FLAGS),
        }

    report = {
        "ok": not findings,
        "run_dir": str(run_dir),
        "require_contract_report": require_contract_report,
        "summary": summary,
        "findings": findings,
    }
    if agent_parity_gate_contract is not None:
        report["agent_parity_gate_contract"] = agent_parity_gate_contract
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("run_dir", type=Path)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--require-contract-report", action="store_true")
    args = parser.parse_args()

    report = validate_run_dir(args.run_dir, require_contract_report=args.require_contract_report)
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
