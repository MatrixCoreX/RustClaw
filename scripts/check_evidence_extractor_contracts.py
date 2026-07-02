#!/usr/bin/env python3
"""Validate structured evidence extractor metadata.

This guards the finalizer/verifier migration from drifting back to
language-text evidence. Explicit structured extractors must declare stable
machine evidence fields, and text-legacy extractors must stay fallback-only.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "crates" / "clawd" / "src" / "task_journal_evidence_registry.rs"

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
    "archive_basic.text_legacy_v1",
    "git_basic.text_legacy_v1",
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


def main() -> int:
    text = REGISTRY.read_text(encoding="utf-8")
    findings = check_step_json_extractors(text)
    findings.extend(check_step_text_extractors(text))
    findings.extend(check_explicit_text_legacy_strict_shape(text))
    print(f"EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
