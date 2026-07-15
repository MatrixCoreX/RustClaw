#!/usr/bin/env python3
"""Validate Chinese-provider smoke matrix summary artifacts."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any


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


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("summary_not_object")
    return value


def string_counter(rows: list[dict[str, Any]], field: str) -> dict[str, int]:
    return dict(sorted(Counter(str(row.get(field) or "unknown") for row in rows).items()))


def validate_summary(summary: dict[str, Any]) -> list[str]:
    findings: list[str] = []
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("summary_json", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

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
            "CHINESE_PROVIDER_SMOKE_SUMMARY ok "
            f"provider_count={result.get('provider_count')}"
        )
    else:
        print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True), file=sys.stderr)
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
