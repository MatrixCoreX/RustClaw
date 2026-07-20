#!/usr/bin/env python3
"""Guard repair recovery from consuming user-visible text/error_text fields."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

CORE_RECOVERY_FILES = [
    "crates/clawd/src/agent_engine/loop_control.rs",
    "crates/clawd/src/agent_engine/loop_control_answer_recovery.rs",
    "crates/clawd/src/answer_verifier_runtime.rs",
    "crates/clawd/src/verifier.rs",
    "crates/clawd/src/repair_signal.rs",
    "crates/clawd/src/execution_adapters.rs",
    "crates/clawd/src/finalize/task_resume.rs",
    "crates/clawd/src/finalize/loop_reply_execution_summary.rs",
]

# These files may check that lifecycle handoff payloads do not contain
# user-visible fallback fields. They must not read those fields as strings.
PRESENCE_CHECK_ONLY_FILES = [
    "crates/clawd/src/repo/task_resume_execution.rs",
    "crates/clawd/src/worker/async_poll_executor.rs",
    "crates/clawd/src/worker/resume_replay_executor.rs",
    "crates/clawd/src/worker/runtime_support/stale_recovery.rs",
]

MONITORED_FILES = CORE_RECOVERY_FILES + PRESENCE_CHECK_ONLY_FILES

NORMALIZE_FOR_USER_FORBIDDEN_FILES = [
    "crates/clawd/src/agent_engine.rs",
    "crates/clawd/src/agent_engine/observed_output_entries.rs",
    "crates/clawd/src/agent_engine/skill_execution.rs",
    "crates/clawd/src/agent_engine/skill_execution_preflight.rs",
    "crates/clawd/src/finalize/loop_reply_content_evidence_failure.rs",
    "crates/clawd/src/finalize/task_resume.rs",
    "crates/clawd/src/task_journal_evidence_registry.rs",
]
NORMALIZE_FOR_USER_FORBIDDEN_SNIPPET = "normalize_skill_error_for_user("

FINALIZER_RESUME_FILE = "crates/clawd/src/finalize/task_resume.rs"
EXECUTION_SUMMARY_FILE = "crates/clawd/src/finalize/loop_reply_execution_summary.rs"
FINALIZER_RESUME_FORBIDDEN_SNIPPETS = [
    "fn resume_context_failed_step_texts",
    "fn text_is_directory_lookup_failure",
    "structured.error_text",
    "error.error_text",
]
EXECUTION_SUMMARY_FORBIDDEN_SNIPPETS = [
    "normalize_skill_error_for_user",
    "file not found",
]

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


def scan_source(relative: str, text: str) -> list[str]:
    findings: list[str] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if relative == FINALIZER_RESUME_FILE:
            for snippet in FINALIZER_RESUME_FORBIDDEN_SNIPPETS:
                if snippet in line:
                    findings.append(f"{relative}:{line_no}: finalizer_resume_text_control: {line.strip()}")
                    break
            else:
                pass
        if relative == EXECUTION_SUMMARY_FILE:
            for snippet in EXECUTION_SUMMARY_FORBIDDEN_SNIPPETS:
                if snippet in line:
                    findings.append(f"{relative}:{line_no}: execution_summary_text_control: {line.strip()}")
                    break
            else:
                pass
        if any(pattern.search(line) for pattern in STRING_READ_PATTERNS):
            findings.append(f"{relative}:{line_no}: string_read: {line.strip()}")
            continue
        has_forbidden_reference = any(
            pattern.search(line) for pattern in FORBIDDEN_PATTERNS
        )
        if has_forbidden_reference and not is_allowed_presence_check(relative, line):
            findings.append(f"{relative}:{line_no}: {line.strip()}")
    return findings


def scan_normalize_for_user_text(relative: str, text: str) -> list[str]:
    findings: list[str] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if NORMALIZE_FOR_USER_FORBIDDEN_SNIPPET in line:
            findings.append(
                f"{relative}:{line_no}: normalize_for_user_boundary: {line.strip()}"
            )
    return findings


def scan_normalize_for_user_call_sites() -> list[str]:
    findings: list[str] = []
    for relative in NORMALIZE_FOR_USER_FORBIDDEN_FILES:
        path = ROOT / relative
        text = path.read_text(encoding="utf-8")
        findings.extend(scan_normalize_for_user_text(relative, text))
    return findings


def scan_repo() -> list[str]:
    findings: list[str] = []
    for relative in MONITORED_FILES:
        path = ROOT / relative
        text = path.read_text(encoding="utf-8")
        findings.extend(scan_source(relative, text))
    findings.extend(scan_normalize_for_user_call_sites())
    return findings


def run_self_test() -> int:
    core_bad = 'fn bad(value: &Value) { let text = value.get("text").and_then(Value::as_str); }'
    core_presence_bad = 'fn bad(value: &Value) -> bool { value.get("text").is_some() }'
    finalizer_error_text_bad = (
        'fn bad(value: &Value) { let text = value.get("error_text").and_then(Value::as_str); }'
    )
    finalizer_text_classifier_bad = (
        "fn resume_context_failed_step_texts(value: &Value) -> Vec<&str> { Vec::new() }"
    )
    execution_summary_text_bad = (
        'fn bad(value: &Value) { let text = value.get("text").and_then(Value::as_str); }'
    )
    execution_summary_normalizer_bad = (
        "fn bad() { crate::skills::normalize_skill_error_for_user(\"run_cmd\", \"err\"); }"
    )
    normalize_for_user_call_site_bad = (
        "let value = crate::skills::normalize_skill_error_for_user(\"run_cmd\", err);"
    )
    lifecycle_presence_ok = 'fn ok(value: &Value) -> bool { value.get("text").is_none() }'
    lifecycle_read_bad = (
        'fn bad(value: &Value) { let text = value.get("error_text").and_then(Value::as_str); }'
    )
    assert scan_source(CORE_RECOVERY_FILES[0], core_bad)
    assert scan_source(CORE_RECOVERY_FILES[0], core_presence_bad)
    assert scan_source(FINALIZER_RESUME_FILE, finalizer_error_text_bad)
    assert scan_source(FINALIZER_RESUME_FILE, finalizer_text_classifier_bad)
    assert scan_source(EXECUTION_SUMMARY_FILE, execution_summary_text_bad)
    assert scan_source(EXECUTION_SUMMARY_FILE, execution_summary_normalizer_bad)
    assert scan_normalize_for_user_text(
        NORMALIZE_FOR_USER_FORBIDDEN_FILES[0],
        normalize_for_user_call_site_bad,
    )
    assert not scan_source(PRESENCE_CHECK_ONLY_FILES[0], lifecycle_presence_ok)
    assert scan_source(PRESENCE_CHECK_ONLY_FILES[0], lifecycle_read_bad)
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings = scan_repo()

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
    sys.exit(main(sys.argv[1:]))
