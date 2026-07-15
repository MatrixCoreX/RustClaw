#!/usr/bin/env python3
"""Core legacy/runtime guards for runtime semantic rewrite boundary checks."""

from __future__ import annotations

import dataclasses
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("semantic_rewrite", re.compile(r"\bsemantic_rewrite\b")),
    ("legacy_migration_debt", re.compile(r"\blegacy_migration_debt\b")),
    ("legacy_semantic_reroute", re.compile(r"\blegacy_semantic_reroute\b")),
    ("agent_loop_semantic_defer", re.compile(r"\bagent_loop_semantic_defer\b")),
    (
        "post_route_semantic_clarify_deferred",
        re.compile(r"\bpost_route_semantic_clarify_deferred_to_agent_loop\b"),
    ),
)

ROUTE_RESULT_RAW_SEMANTIC_ACCESS = re.compile(
    r"\b(?:route|route_result|execution_route_result)\.output_contract\.semantic_kind\b"
)
ROUTE_RESULT_RAW_SEMANTIC_CLEAR = re.compile(
    r"\b(?:route|route_result|execution_route_result)\.output_contract\.semantic_kind"
    r"\s*=\s*(?:crate::)?OutputSemanticKind::None\b"
)
LEGACY_JSON_SEMANTIC_FIELD_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r'"semantic_kind"\s*:'),
    re.compile(r'\\"semantic_kind\\"\s*:'),
    re.compile(r'\.get\("semantic_kind"\)'),
    re.compile(r'contains_key\("semantic_kind"\)'),
    re.compile(r'\.pointer\("/semantic_kind"\)'),
    re.compile(r'"semantic_kind"\.to_string\(\)'),
)
LEGACY_RUNTIME_SEMANTIC_OUTPUT_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("legacy_semantic_kv_output", re.compile(r'"(?:contract_)?semantic_kind[=:]')),
    ("legacy_semantic_trace_label", re.compile(r'"[^"]*\bsemantic[=:]')),
    ("legacy_semantic_colon_output", re.compile(r'"semantic_kind:\s')),
    ("legacy_semantic_prompt_instruction", re.compile(r"\bSet\s+semantic_kind\b")),
    ("legacy_expected_semantic_fact", re.compile(r"expected_semantic_kind:")),
)

ALLOWED_PRODUCTION_FILES: set[str] = set()


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    if rel_path.endswith(("_tests.rs", "tests.rs")):
        return True
    return any(part == "tests" or part.endswith("_tests") for part in parts)


def production_rust_files() -> list[Path]:
    return sorted(
        path
        for path in SRC_ROOT.rglob("*.rs")
        if path.is_file() and not is_test_path(path)
    )


def finding_allowed(rel_path: str) -> bool:
    return rel_path in ALLOWED_PRODUCTION_FILES


def scan_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in FORBIDDEN_PATTERNS:
            if not pattern.search(line):
                continue
            if finding_allowed(rel_path):
                continue
            findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_repo_text(rel_path: str, text: str) -> list[Finding]:
    return scan_text(rel_path, text)


def scan_route_result_raw_semantic_access(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if not ROUTE_RESULT_RAW_SEMANTIC_ACCESS.search(line):
            continue
        if ROUTE_RESULT_RAW_SEMANTIC_CLEAR.search(line):
            continue
        findings.append(
            Finding(
                rel_path,
                line_no,
                "route_result_raw_semantic_access",
                line.strip(),
            )
        )
    return findings


def scan_legacy_json_semantic_fields(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for pattern in LEGACY_JSON_SEMANTIC_FIELD_PATTERNS:
            if not pattern.search(line):
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "legacy_json_semantic_kind_field",
                    line.strip(),
                )
            )
    return findings


def scan_legacy_runtime_semantic_outputs(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in LEGACY_RUNTIME_SEMANTIC_OUTPUT_PATTERNS:
            if not pattern.search(line):
                continue
            findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_normalizer_route_result_boundary() -> list[Finding]:
    path = SRC_ROOT / "intent_router_route_output.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    findings: list[Finding] = []
    required_tokens = [
        "fn demote_output_contract_semantic_to_route_marker",
        'format!("contract:{}"',
        "output_contract.apply_output_contract_ref(OutputContractRef::new(OutputSemanticKind::None));",
        "demote_output_contract_semantic_to_route_marker(&mut output_contract, &mut route_reason);",
    ]
    for token in required_tokens:
        if token in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "normalizer_route_result_semantic_demote_missing",
                f"missing required boundary token: {token}",
            )
        )
    return findings


def scan_journal_output_contract_ref_boundary() -> list[Finding]:
    path = SRC_ROOT / "task_journal_decision_envelope.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    if "let contract = route.effective_output_contract();" in text:
        return []
    return [
        Finding(
            rel_path,
            1,
            "journal_output_contract_ref_not_effective",
            "output_contract_ref_for_route must use route.effective_output_contract()",
        )
    ]


def scan_static_capability_compat_boundary() -> list[Finding]:
    paths = (
        SRC_ROOT / "capability_resolver.rs",
        SRC_ROOT / "capability_resolver_tests.rs",
        SRC_ROOT / "agent_engine" / "dispatch_support.rs",
    )
    forbidden_tokens = [
        "resolve_static_capability",
        "resolve_static_capability_action_for_state",
        "static_capability_compat_enabled",
        "static_capability",
        "static_capabilities",
        "registry_capability_surface_available",
        "capability_resolver_static_compat_resolved",
        "capability_resolver_unresolved",
        '"static_compat"',
    ]
    findings: list[Finding] = []
    for path in paths:
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        for line_no, line in enumerate(text.splitlines(), start=1):
            for token in forbidden_tokens:
                if token not in line:
                    continue
                findings.append(
                    Finding(
                        rel_path,
                        line_no,
                        "static_capability_compat_forbidden",
                        line.strip(),
                    )
                )
    return findings


def scan_contract_repair_judge_boundary() -> list[Finding]:
    path = SRC_ROOT / "intent_router_normalizer_answer_repair.rs"
    if not path.exists():
        return []
    return scan_contract_repair_judge_boundary_text(rel(path), path.read_text(encoding="utf-8"))


def scan_contract_repair_judge_boundary_text(rel_path: str, text: str) -> list[Finding]:
    required_tokens = [
        "#[cfg(test)]\nasync fn apply_contract_judge_repair(",
        "#[cfg(not(test))]\nasync fn apply_contract_judge_repair(",
        "contract_repair_report.needs_llm_contract_integrity_repair()",
    ]
    findings: list[Finding] = []
    for token in required_tokens:
        if token in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_repair_judge_boundary_missing",
                f"missing required boundary token: {token}",
            )
        )
    if "contract_repair_judge_runtime_enabled" in text or "cfg!(test)" in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_repair_judge_runtime_switch",
                "pre-agent LLM repair must be compile-time test-only, not a runtime switch",
            )
        )
    findings.extend(scan_semantic_suspect_report_boundary(rel_path, text))
    return findings


def scan_semantic_suspect_report_boundary(rel_path: str, text: str) -> list[Finding]:
    semantic_report_pos = text.find('contract_repair_report.add("semantic_suspect"')
    if semantic_report_pos < 0:
        return []
    test_only_repair_pos = text.find(
        "#[cfg(test)]\nasync fn apply_contract_judge_repair("
    )
    if 0 <= test_only_repair_pos < semantic_report_pos:
        return []
    return [
        Finding(
            rel_path,
            1,
            "semantic_suspect_report_not_test_gated",
            "semantic_suspect report collection must stay behind contract_repair_judge_runtime_enabled()",
        )
    ]
