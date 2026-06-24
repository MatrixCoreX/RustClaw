#!/usr/bin/env python3
"""Guard repair recovery from consuming user-visible text/error_text fields."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

CORE_RECOVERY_FILES = [
    "crates/clawd/src/agent_engine/loop_control_answer_recovery.rs",
    "crates/clawd/src/agent_engine/loop_control_local_health_recovery.rs",
    "crates/clawd/src/answer_verifier_runtime.rs",
    "crates/clawd/src/verifier.rs",
    "crates/clawd/src/repair_signal.rs",
    "crates/clawd/src/execution_adapters.rs",
]

# These files may check that lifecycle handoff payloads do not contain
# user-visible fallback fields. They must not read those fields as strings.
PRESENCE_CHECK_ONLY_FILES = [
    "crates/clawd/src/repo/task_resume_execution.rs",
    "crates/clawd/src/worker/async_poll_executor.rs",
    "crates/clawd/src/worker/resume_replay_executor.rs",
]

MONITORED_FILES = CORE_RECOVERY_FILES + PRESENCE_CHECK_ONLY_FILES

STRING_READ_PATTERNS = [
    re.compile(r'\.get\("text"\)\.and_then\(\s*Value::as_str\s*\)'),
    re.compile(r'\.get\("error_text"\)\.and_then\(\s*Value::as_str\s*\)'),
    re.compile(r'\.get\("text"\)\.and_then\(\s*\|\w+\|\s*\w+\.as_str\(\)\s*\)'),
    re.compile(r'\.get\("error_text"\)\.and_then\(\s*\|\w+\|\s*\w+\.as_str\(\)\s*\)'),
    re.compile(r'\[\s*"text"\s*\]\.as_str\(\)'),
    re.compile(r'\[\s*"error_text"\s*\]\.as_str\(\)'),
]

FORBIDDEN_PATTERNS = [
    re.compile(r'\.get\("text"\)'),
    re.compile(r'\.get\("error_text"\)'),
    re.compile(r'\[\s*"text"\s*\]'),
    re.compile(r'\[\s*"error_text"\s*\]'),
]


def is_allowed_presence_check(relative: str, line: str) -> bool:
    if relative not in PRESENCE_CHECK_ONLY_FILES:
        return False
    stripped = line.strip()
    if ".and_then" in stripped or ".as_str()" in stripped:
        return False
    return any(
        token in stripped
        for token in (
            '.get("text").is_none()',
            '.get("text").is_some()',
            '.get("error_text").is_none()',
            '.get("error_text").is_some()',
        )
    )


def main() -> int:
    findings: list[str] = []
    for relative in MONITORED_FILES:
        path = ROOT / relative
        text = path.read_text(encoding="utf-8")
        for line_no, line in enumerate(text.splitlines(), start=1):
            if any(pattern.search(line) for pattern in STRING_READ_PATTERNS):
                findings.append(f"{relative}:{line_no}: string_read: {line.strip()}")
                continue
            has_forbidden_reference = any(
                pattern.search(line) for pattern in FORBIDDEN_PATTERNS
            )
            if has_forbidden_reference and not is_allowed_presence_check(relative, line):
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
