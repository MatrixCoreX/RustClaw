#!/usr/bin/env python3
"""Guard repair recovery from consuming user-visible text/error_text fields."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

MONITORED_FILES = [
    "crates/clawd/src/agent_engine/loop_control_answer_recovery.rs",
    "crates/clawd/src/agent_engine/loop_control_local_health_recovery.rs",
    "crates/clawd/src/answer_verifier_runtime.rs",
]

FORBIDDEN_PATTERNS = [
    re.compile(r'\.get\("text"\)'),
    re.compile(r'\.get\("error_text"\)'),
    re.compile(r'\[\s*"text"\s*\]'),
    re.compile(r'\[\s*"error_text"\s*\]'),
]


def main() -> int:
    findings: list[str] = []
    for relative in MONITORED_FILES:
        path = ROOT / relative
        text = path.read_text(encoding="utf-8")
        for line_no, line in enumerate(text.splitlines(), start=1):
            if any(pattern.search(line) for pattern in FORBIDDEN_PATTERNS):
                findings.append(f"{relative}:{line_no}: {line.strip()}")

    if findings:
        print("REPAIR_USER_TEXT_FIELD_CHECK findings={}".format(len(findings)))
        for finding in findings:
            print(finding)
        return 1

    print(
        "REPAIR_USER_TEXT_FIELD_CHECK ok files={}".format(
            len(MONITORED_FILES)
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
