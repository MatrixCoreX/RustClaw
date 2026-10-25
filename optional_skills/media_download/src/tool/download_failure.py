"""Versioned child-process errors; diagnostics never drive retry decisions."""
from __future__ import annotations

import json
import sys
import urllib.error

ERROR_PREFIX = "__MEDIA_DOWNLOAD_ERROR__:"
RETRYABLE_CODES = {
    "login_required", "display_unavailable", "interactive_verification_timeout",
    "interactive_verification_cancelled", "browser_profile_busy", "network_unavailable",
    "request_timeout", "rate_limited", "provider_unavailable",
}
ERROR_CODES = RETRYABLE_CODES | {
    "execution_failed", "dependency_unavailable", "media_not_found", "access_denied", "not_found",
    "browser_authentication_unavailable",
}


def error_metadata(error: BaseException) -> dict:
    current = error
    seen: set[int] = set()
    code = "execution_failed"
    status = None
    retry_after = None
    while current is not None and id(current) not in seen:
        seen.add(id(current))
        explicit = getattr(current, "error_code", None)
        if isinstance(explicit, str) and explicit in ERROR_CODES and explicit != "execution_failed":
            code = explicit
            break
        if isinstance(current, urllib.error.HTTPError):
            status = current.code
            code = (
                "rate_limited" if status == 429 else
                "provider_unavailable" if status >= 500 else
                "access_denied" if status in (401, 403) else "execution_failed"
            )
            raw_delay = current.headers.get("Retry-After", "") if current.headers else ""
            if raw_delay.isdigit():
                retry_after = min(int(raw_delay), 86400)
            break
        if isinstance(current, ModuleNotFoundError):
            code = "dependency_unavailable"
            break
        if isinstance(current, FileNotFoundError):
            code = "not_found"
            break
        if isinstance(current, TimeoutError):
            code = "request_timeout"
            break
        if isinstance(current, urllib.error.URLError):
            code = "network_unavailable"
        current = current.__cause__
    return {
        "schema_version": 1,
        "error_code": code,
        "message_key": f"media_download.error.{code}",
        "retryable": code in RETRYABLE_CODES,
        "status_code": status,
        "retry_after_seconds": retry_after,
    }


def emit_error(error: BaseException) -> None:
    print(f"error: {error}", file=sys.stderr)
    print(ERROR_PREFIX + json.dumps(error_metadata(error), separators=(",", ":")),
          file=sys.stderr, flush=True)


def read_error(stderr: str) -> dict:
    for line in reversed(stderr.splitlines()):
        if not line.startswith(ERROR_PREFIX):
            continue
        try:
            value = json.loads(line[len(ERROR_PREFIX):])
        except (ValueError, TypeError):
            continue
        if not isinstance(value, dict) or value.get("schema_version") != 1:
            continue
        code = value.get("error_code")
        if not isinstance(code, str) or code not in ERROR_CODES or not isinstance(value.get("retryable"), bool):
            continue
        result = {"error_code": code, "message_key": f"media_download.error.{code}",
                  "retryable": value["retryable"]}
        for name in ("status_code", "retry_after_seconds"):
            number = value.get(name)
            if type(number) is int and 0 <= number <= 86400:
                result[name] = number
        return result
    return {"error_code": "execution_failed", "message_key": "media_download.error.execution_failed",
            "retryable": False}
