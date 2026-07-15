#!/usr/bin/env python3
"""Classify live NL provider failures from client-like runner output."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


def read_text(path: str) -> str:
    if path == "-":
        return sys.stdin.read()
    return Path(path).read_text(encoding="utf-8", errors="replace")


def provider_http_statuses(text: str) -> list[int]:
    statuses: list[int] = []
    for raw in re.findall(r"error=http\s+(\d+)", text):
        try:
            status = int(raw)
        except ValueError:
            continue
        if status not in statuses:
            statuses.append(status)
    return statuses


def raw_provider_error_objects(text: str) -> list[dict[str, Any]]:
    objects: list[dict[str, Any]] = []
    for raw in re.findall(r"raw_response=(\{[^\n]*\})", text):
        try:
            value = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and isinstance(value.get("error"), dict):
            objects.append(value["error"])
    return objects


def lower_tokens(values: list[Any]) -> set[str]:
    return {str(value).strip().lower() for value in values if str(value).strip()}


def classify_provider_failure(text: str) -> dict[str, Any]:
    statuses = provider_http_statuses(text)
    errors = raw_provider_error_objects(text)
    codes = lower_tokens([error.get("code") for error in errors])
    types = lower_tokens([error.get("type") for error in errors])
    provider_seen = bool(re.search(r"provider=vendor-[^\s]+", text))

    reason_code = "runner_failed"
    status = "failed"
    if "clawd is not healthy" in text:
        reason_code = "clawd_unhealthy"
    elif 429 in statuses or "429" in codes or "limitation" in types:
        status = "blocked"
        reason_code = "provider_quota_exceeded"
    elif 402 in statuses or "arrearage" in codes or "arrearage" in types:
        status = "blocked"
        reason_code = "provider_account_blocked"
    elif "model_not_found" in codes:
        status = "blocked"
        reason_code = "provider_model_unavailable"
    elif any(status_code in statuses for status_code in (401, 403)):
        status = "blocked"
        reason_code = "provider_auth_failed"
    elif provider_seen and "timed out" in text.lower():
        status = "blocked"
        reason_code = "provider_timeout"
    elif provider_seen:
        reason_code = "provider_runtime_error"

    return {
        "status": status,
        "reason_code": reason_code,
        "provider_http_statuses": statuses,
        "provider_error_codes": sorted(codes),
        "provider_error_types": sorted(types),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output_file")
    parser.add_argument("--reason-only", action="store_true")
    parser.add_argument("--status-only", action="store_true")
    args = parser.parse_args()

    result = classify_provider_failure(read_text(args.output_file))
    if args.reason_only:
        print(result["reason_code"])
    elif args.status_only:
        print(result["status"])
    else:
        print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
