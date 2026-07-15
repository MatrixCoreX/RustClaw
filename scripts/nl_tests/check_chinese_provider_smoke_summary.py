#!/usr/bin/env python3
"""Validate Chinese-provider smoke matrix summary artifacts."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path, PurePosixPath
from typing import Any

if __package__:
    from .secret_scan import secret_scan_findings
else:
    from secret_scan import secret_scan_findings


REQUIRED_TOP_LEVEL_FIELDS = {
    "credential_state_counts",
    "live_scope_counts",
    "provider_count",
    "providers",
    "reason_code_counts",
    "status_counts",
}

REQUIRED_PROVIDER_FIELDS = {
    "case_file",
    "credential_required_env",
    "credential_state",
    "exit_code",
    "live_scope",
    "live_scope_providers",
    "output_file",
    "provider",
    "reason_code",
    "run_dir",
    "status",
}

ALLOWED_CREDENTIAL_STATES = {
    "configured_env",
    "configured_inline",
    "missing",
    "unknown",
}

ALLOWED_LIVE_SCOPES = {
    "all",
    "excluded",
    "included",
    "unknown",
}

MACHINE_TOKEN_RE = re.compile(r"^[a-z0-9_.-]+$")
ENV_NAME_RE = re.compile(r"^[A-Z][A-Z0-9_]*$")
PATH_REF_FIELDS = {"case_file", "output_file", "run_dir"}


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("summary_not_object")
    return value


def string_counter(rows: list[dict[str, Any]], field: str) -> dict[str, int]:
    return dict(sorted(Counter(str(row.get(field) or "unknown") for row in rows).items()))


def is_safe_path_ref(value: Any, *, allow_empty: bool = False) -> bool:
    if not isinstance(value, str):
        return False
    if value == "":
        return allow_empty
    if value.startswith("/") or "\\" in value or any(ch.isspace() for ch in value):
        return False
    path = PurePosixPath(value)
    return all(part not in {"", ".", ".."} for part in path.parts)


def validate_summary(summary: dict[str, Any]) -> list[str]:
    findings: list[str] = []
    findings.extend(secret_scan_findings(summary))
    missing_top = sorted(REQUIRED_TOP_LEVEL_FIELDS - set(summary))
    if missing_top:
        findings.append(f"missing_top_level_fields:{','.join(missing_top)}")

    providers = summary.get("providers")
    if not isinstance(providers, list):
        findings.append("providers_not_array")
        return findings

    rows: list[dict[str, Any]] = []
    for index, row in enumerate(providers):
        if not isinstance(row, dict):
            findings.append(f"provider_row_not_object:{index}")
            continue
        rows.append(row)
        missing = sorted(REQUIRED_PROVIDER_FIELDS - set(row))
        if missing:
            findings.append(f"provider_row_missing_fields:{index}:{','.join(missing)}")
        provider = str(row.get("provider") or "")
        status = str(row.get("status") or "")
        reason_code = str(row.get("reason_code") or "")
        credential_state = str(row.get("credential_state") or "")
        live_scope = str(row.get("live_scope") or "")
        for field, value in [
            ("provider", provider),
            ("status", status),
            ("reason_code", reason_code),
            ("credential_state", credential_state),
            ("live_scope", live_scope),
        ]:
            if not MACHINE_TOKEN_RE.fullmatch(value):
                findings.append(f"provider_row_bad_machine_token:{index}:{field}:{value}")
        if credential_state not in ALLOWED_CREDENTIAL_STATES:
            findings.append(f"provider_row_bad_credential_state:{index}:{credential_state}")
        if live_scope not in ALLOWED_LIVE_SCOPES:
            findings.append(f"provider_row_bad_live_scope:{index}:{live_scope}")
        for field in sorted(PATH_REF_FIELDS):
            if not is_safe_path_ref(row.get(field), allow_empty=(field == "run_dir")):
                findings.append(f"provider_row_bad_path_ref:{index}:{field}")
        required_env = row.get("credential_required_env")
        if not isinstance(required_env, list):
            findings.append(f"provider_row_credential_required_env_not_array:{index}")
        else:
            for env_name in required_env:
                env_name_text = str(env_name)
                if not ENV_NAME_RE.fullmatch(env_name_text):
                    findings.append(
                        f"provider_row_bad_credential_required_env:{index}:{env_name_text}"
                    )
        for forbidden in ["api_key", "credential_value", "secret_value"]:
            if forbidden in row:
                findings.append(f"provider_row_forbidden_secret_field:{index}:{forbidden}")

    provider_count = summary.get("provider_count")
    if provider_count != len(rows):
        findings.append(f"provider_count_mismatch:{provider_count}:{len(rows)}")

    expected_counters = {
        "credential_state_counts": string_counter(rows, "credential_state"),
        "live_scope_counts": string_counter(rows, "live_scope"),
        "reason_code_counts": string_counter(rows, "reason_code"),
        "status_counts": string_counter(rows, "status"),
    }
    for field, expected in expected_counters.items():
        actual = summary.get(field)
        if actual != expected:
            findings.append(
                f"{field}_mismatch:expected={json.dumps(expected, sort_keys=True)}"
            )
    return findings


def self_test_summary(row_override: dict[str, Any] | None = None) -> dict[str, Any]:
    row: dict[str, Any] = {
        "case_file": "scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt",
        "credential_required_env": ["MINIMAX_API_KEY"],
        "credential_state": "configured_env",
        "exit_code": 0,
        "live_scope": "included",
        "live_scope_providers": ["minimax"],
        "output_file": "out_dir/minimax/run.output.txt",
        "provider": "minimax",
        "reason_code": "dry_run",
        "run_dir": "",
        "status": "planned",
    }
    if row_override:
        row.update(row_override)
    return {
        "credential_state_counts": string_counter([row], "credential_state"),
        "live_scope_counts": string_counter([row], "live_scope"),
        "provider_count": 1,
        "providers": [row],
        "reason_code_counts": string_counter([row], "reason_code"),
        "status_counts": string_counter([row], "status"),
    }


def run_self_test() -> int:
    positive_findings = validate_summary(self_test_summary())
    if positive_findings:
        print(f"SELF_TEST_FAIL positive:{positive_findings}", file=sys.stderr)
        return 1

    cases = (
        ("absolute-output", {"output_file": "/tmp/run.output.txt"}, "provider_row_bad_path_ref:0:output_file"),
        ("parent-case", {"case_file": "../cases.txt"}, "provider_row_bad_path_ref:0:case_file"),
        ("backslash-run-dir", {"run_dir": r"out_dir\minimax"}, "provider_row_bad_path_ref:0:run_dir"),
        ("space-output", {"output_file": "out dir/run.output.txt"}, "provider_row_bad_path_ref:0:output_file"),
    )
    for label, row_override, expected in cases:
        findings = validate_summary(self_test_summary(row_override))
        if expected not in set(findings):
            print(f"SELF_TEST_FAIL {label}:{findings}", file=sys.stderr)
            return 1

    print("CHINESE_PROVIDER_SMOKE_SUMMARY_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("summary_json", type=Path, nargs="?")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()
    if args.summary_json is None:
        parser.error("summary_json is required unless --self-test is used")

    try:
        summary = load_json(args.summary_json)
    except Exception as exc:  # noqa: BLE001 - command-line validator reports parse failures.
        result = {
            "ok": False,
            "summary_json": str(args.summary_json),
            "findings": [f"summary_read_failed:{exc}"],
        }
    else:
        findings = validate_summary(summary)
        result = {
            "ok": not findings,
            "summary_json": str(args.summary_json),
            "provider_count": summary.get("provider_count"),
            "findings": findings,
        }

    if args.json:
        print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    elif result["ok"]:
        print(
            "CHINESE_PROVIDER_SMOKE_SUMMARY_CHECK ok "
            f"provider_count={result.get('provider_count')}"
        )
    else:
        print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True), file=sys.stderr)
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
