#!/usr/bin/env python3
"""Validate the shared secret scanner contract."""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from typing import Any

from secret_scan import secret_scan_findings


@dataclass(frozen=True)
class ContractCase:
    case_id: str
    value: Any
    expected_findings: list[str]


def contract_cases() -> list[ContractCase]:
    api_key_like = "tp-" + ("A" * 20)
    bearer_like = "Bearer " + ("B" * 20)
    jwt_like = "eyJ" + ("C" * 16) + "." + ("D" * 16) + "." + ("E" * 8)
    return [
        ContractCase(
            case_id="forbidden_field_nested_path",
            value={"providers": [{"metadata": {"api-key": "redacted"}}]},
            expected_findings=["forbidden_secret_field:$.providers[0].metadata.api-key"],
        ),
        ContractCase(
            case_id="api_key_like_value",
            value={"catalog": [{"note": api_key_like}]},
            expected_findings=["secret_like_value:$.catalog[0].note:api_key_prefix"],
        ),
        ContractCase(
            case_id="bearer_like_value",
            value={"headers": [{"authorization_hint": bearer_like}]},
            expected_findings=["secret_like_value:$.headers[0].authorization_hint:bearer_token"],
        ),
        ContractCase(
            case_id="jwt_like_value",
            value={"claims": [{"token_hint": jwt_like}]},
            expected_findings=["secret_like_value:$.claims[0].token_hint:jwt_like"],
        ),
        ContractCase(
            case_id="safe_values",
            value={"providers": [{"credential_state": "configured_env", "required_env": ["MINIMAX_API_KEY"]}]},
            expected_findings=[],
        ),
    ]


def build_report() -> dict[str, Any]:
    failures: list[dict[str, Any]] = []
    cases = contract_cases()
    for case in cases:
        actual = secret_scan_findings(case.value)
        if actual != case.expected_findings:
            failures.append(
                {
                    "case_id": case.case_id,
                    "expected_findings": case.expected_findings,
                    "actual_findings": actual,
                }
            )
    return {
        "ok": not failures,
        "case_count": len(cases),
        "failures": failures,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    report = build_report()
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    elif report["ok"]:
        print(f"SECRET_SCAN_CONTRACT ok case_count={report['case_count']}")
    else:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True), file=sys.stderr)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
