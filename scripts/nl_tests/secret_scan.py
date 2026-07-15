#!/usr/bin/env python3
"""Shared secret scanning helpers for NL gate artifacts."""

from __future__ import annotations

import re
from typing import Any


FORBIDDEN_SECRET_FIELD_NAMES = {
    "api_key",
    "apikey",
    "authorization",
    "access_token",
    "refresh_token",
    "secret",
    "secret_value",
    "credential_value",
}

SECRET_VALUE_PATTERNS = [
    ("api_key_prefix", re.compile(r"\b(?:sk|tp|ak)-[A-Za-z0-9][A-Za-z0-9_-]{15,}\b")),
    ("bearer_token", re.compile(r"\b[Bb]earer\s+[A-Za-z0-9._-]{12,}\b")),
    (
        "jwt_like",
        re.compile(
            r"\beyJ[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{8,}\b"
        ),
    ),
]


def secret_scan_findings(value: Any, path: str = "$") -> list[str]:
    findings: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            key_text = str(key)
            key_token = key_text.lower().replace("-", "_")
            child_path = f"{path}.{key_text}"
            if key_token in FORBIDDEN_SECRET_FIELD_NAMES:
                findings.append(f"forbidden_secret_field:{child_path}")
            findings.extend(secret_scan_findings(child, child_path))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            findings.extend(secret_scan_findings(child, f"{path}[{index}]"))
    elif isinstance(value, str):
        for kind, pattern in SECRET_VALUE_PATTERNS:
            if pattern.search(value):
                findings.append(f"secret_like_value:{path}:{kind}")
                break
    return findings
