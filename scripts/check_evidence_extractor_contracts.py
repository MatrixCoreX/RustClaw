#!/usr/bin/env python3
"""Validate structured evidence extractor metadata.

This guards the finalizer/verifier migration from drifting back to
language-text evidence. Explicit structured extractors must declare stable
machine evidence fields, and text-legacy extractors must stay fallback-only.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "crates" / "clawd" / "src" / "task_journal_evidence_registry.rs"
AGENTS = ROOT / "AGENTS.md"

STEP_JSON_RE = re.compile(
    r"step_json_extractor\(\s*"
    r'"(?P<action>[^"]+)"\s*,\s*'
    r'"(?P<extractor>[^"]+)"\s*,\s*'
    r"&\[(?P<fields>.*?)\]\s*,?\s*\)",
    re.DOTALL,
)
STEP_TEXT_RE = re.compile(
    r"step_text_extractor\(\s*"
    r'"(?P<action>[^"]+)"\s*,\s*'
    r'"(?P<extractor>[^"]+)"\s*,\s*'
    r"&\[(?P<fields>.*?)\]\s*,?\s*\)",
    re.DOTALL,
)
FIELD_RE = re.compile(r'"([^"]+)"')

BASELINE_TEXT_LEGACY_STRICT_REFS = {
    "http_basic.text_legacy_v1",
    "list_dir.text_legacy_v1",
    "run_cmd.text_legacy_v1",
    "task_control.get.text_legacy_v1",
    "task_control.list.text_legacy_v1",
    "write_file.text_legacy_v1",
    "x.text_legacy_v1",
}


def stable_field_tokens(raw_fields: str) -> list[str]:
    return [field.strip() for field in FIELD_RE.findall(raw_fields) if field.strip()]


def check_step_json_extractors(text: str) -> list[str]:
    findings: list[str] = []
    seen = 0
    for match in STEP_JSON_RE.finditer(text):
        seen += 1
        action = match.group("action")
        extractor = match.group("extractor")
        fields = stable_field_tokens(match.group("fields"))
        if not fields:
            findings.append(f"{extractor}: action={action} missing_provided_evidence")
        if not extractor.endswith(".structured_json_v1") and ".structured_json_" not in extractor:
            findings.append(f"{extractor}: action={action} extractor_ref_not_structured_json")
        for field in fields:
            if not re.fullmatch(r"[a-z0-9_.-]+", field):
                findings.append(f"{extractor}: invalid_evidence_field={field!r}")
    if seen == 0:
        findings.append("no_step_json_extractors_found")
    return findings


def check_step_text_extractors(text: str) -> list[str]:
    findings: list[str] = []
    for match in STEP_TEXT_RE.finditer(text):
        action = match.group("action")
        extractor = match.group("extractor")
        fields = stable_field_tokens(match.group("fields"))
        if not fields:
            findings.append(f"{extractor}: action={action} missing_provided_evidence")
        for field in fields:
            if not re.fullmatch(r"[a-z0-9_.-]+", field):
                findings.append(f"{extractor}: invalid_evidence_field={field!r}")
        if extractor not in BASELINE_TEXT_LEGACY_STRICT_REFS:
            findings.append(f"{extractor}: new_text_legacy_strict_extractor_not_allowed")
    return findings


def check_explicit_text_legacy_strict_shape(text: str) -> list[str]:
    findings: list[str] = []
    for match in re.finditer(
        r"EvidenceExtractorSpec\s*\{(?P<body>.*?)\}", text, re.DOTALL
    ):
        body = match.group("body")
        if "EvidenceExtractorKind::TextLegacy" not in body:
            continue
        if re.search(r"strict_shape_eligible\s*:\s*true", body):
            extractor = re.search(r'extractor_ref\s*:\s*"([^"]+)"', body)
            if not extractor:
                continue
            ref = extractor.group(1)
            if ref not in BASELINE_TEXT_LEGACY_STRICT_REFS:
                findings.append(f"{ref}: new_text_legacy_strict_extractor_not_allowed")
    return findings


def check_agents_rule_text(text: str) -> list[str]:
    findings: list[str] = []
    required_tokens = {
        "script": "python3 scripts/check_evidence_extractor_contracts.py",
        "stable_machine_fields": "stable machine evidence fields",
        "text_legacy_limit": "text_legacy",
        "text_error_text_boundary": "text/error_text",
        "machine_protocol_boundary": "machine-readable evidence protocol",
    }
    for label, token in required_tokens.items():
        if token not in text:
            findings.append(f"agents_rule_missing:{label}")
    return findings


def check_agents_rule() -> list[str]:
    try:
        text = AGENTS.read_text(encoding="utf-8")
    except OSError as exc:
        return [f"agents_rule_read_failed:{exc.__class__.__name__}"]
    return check_agents_rule_text(text)


def run_self_test() -> int:
    positive_structured = (
        'step_json_extractor("demo", "demo.structured_json_v1", &["field_value"])'
    )
    positive_findings = check_step_json_extractors(positive_structured)
    if positive_findings:
        print(f"SELF_TEST_FAIL positive_structured:{positive_findings}", file=sys.stderr)
        return 1

    missing_field_findings = check_step_json_extractors(
        'step_json_extractor("demo", "demo.structured_json_v1", &[])'
    )
    if not any("missing_provided_evidence" in item for item in missing_field_findings):
        print(
            f"SELF_TEST_FAIL missing_structured_field:{missing_field_findings}",
            file=sys.stderr,
        )
        return 1

    new_text_legacy_findings = check_step_text_extractors(
        'step_text_extractor("demo", "new_skill.text_legacy_v1", &["legacy_text_excerpt"])'
    )
    if not any(
        "new_text_legacy_strict_extractor_not_allowed" in item
        for item in new_text_legacy_findings
    ):
        print(
            f"SELF_TEST_FAIL new_text_legacy:{new_text_legacy_findings}",
            file=sys.stderr,
        )
        return 1

    good_agents = (
        "python3 scripts/check_evidence_extractor_contracts.py "
        "stable machine evidence fields text_legacy text/error_text "
        "machine-readable evidence protocol"
    )
    if check_agents_rule_text(good_agents):
        print("SELF_TEST_FAIL positive_agents_rule", file=sys.stderr)
        return 1

    bad_agents_findings = check_agents_rule_text(
        "python3 scripts/check_evidence_extractor_contracts.py text_legacy"
    )
    expected_agent_findings = {
        "agents_rule_missing:stable_machine_fields",
        "agents_rule_missing:text_error_text_boundary",
    }
    if not expected_agent_findings.issubset(set(bad_agents_findings)):
        print(f"SELF_TEST_FAIL missing_agents_tokens:{bad_agents_findings}", file=sys.stderr)
        return 1

    print("EVIDENCE_EXTRACTOR_CONTRACT_SELF_TEST ok")
    return 0


def run_check() -> int:
    text = REGISTRY.read_text(encoding="utf-8")
    findings = check_step_json_extractors(text)
    findings.extend(check_step_text_extractors(text))
    findings.extend(check_explicit_text_legacy_strict_shape(text))
    findings.extend(check_agents_rule())
    print(f"EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    return run_check()


if __name__ == "__main__":
    sys.exit(main())
