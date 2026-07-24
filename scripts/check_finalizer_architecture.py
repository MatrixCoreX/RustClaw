#!/usr/bin/env python3
"""Enforce the domain-neutral finalizer and generic output-contract boundary."""

from __future__ import annotations

import argparse
import dataclasses
import json
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CLAWD_SRC = ROOT / "crates/clawd/src"
CORE_SRC = ROOT / "crates/claw-core/src"
FINALIZE_DIR = CLAWD_SRC / "finalize"
REGISTRY = ROOT / "configs/skills_registry.toml"
ENVELOPE_SOURCE = ROOT / "crates/claw-core/src/capability_result.rs"
SYNTHESIS_SOURCE = ROOT / "crates/clawd/src/agent_engine/capability_result_synthesis.rs"
SYNTHESIS_PROMPT = ROOT / "prompts/layers/overlays/capability_result_synthesis_prompt.md"

# These ceilings are the post-semantic-contract baseline. They prevent
# unrelated growth while domain-specific branches are held at exactly zero.
MAX_FINALIZER_PRODUCTION_MODULES = 55
MAX_FINALIZER_PRODUCTION_LINES = 18_713

FORBIDDEN_RUNTIME_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("legacy_raw_output_type", re.compile(r"\bRawCommandOutput\b")),
    ("legacy_output_type", re.compile(r"\bOutputSemanticKind\b")),
    (
        "legacy_raw_final_answer_shape",
        re.compile(r"\b(?:RawOutputOrShortSummary|raw_output_or_short_summary)\b"),
    ),
    ("legacy_runtime_field", re.compile(r"\bsemantic_kind\b")),
    ("legacy_registry_field", re.compile(r"\boutput_semantic_kind\b")),
    ("legacy_exact_module", re.compile(r"\bloop_reply_raw_command\b")),
    (
        "legacy_raw_verifier_module",
        re.compile(r"\banswer_verifier_delivery_raw\b"),
    ),
    (
        "legacy_raw_exact_helper",
        re.compile(
            r"\b(?:raw_bounded_read|strict_raw_tail_read|"
            r"route_requires_raw_tail_read_passthrough|"
            r"route_expects_synthesis_over_raw_observation)\w*\b"
        ),
    ),
    (
        "answer_verifier_reason_prose_branch",
        re.compile(
            r"\banswer_incomplete_reason\s*\.\s*(?:contains|starts_with|ends_with)\s*\("
        ),
    ),
    (
        "answer_verifier_instruction_prose_branch",
        re.compile(r"\bretry_instruction\s*\.\s*(?:contains|starts_with|ends_with)\s*\("),
    ),
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def relative(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_source(path: Path, source_root: Path) -> bool:
    rel = path.relative_to(source_root)
    return (
        path.stem == "tests"
        or path.stem.endswith("_tests")
        or any(part == "tests" or part.endswith("_tests") for part in rel.parts[:-1])
    )


def production_rust_files(source_root: Path) -> list[Path]:
    return sorted(
        path
        for path in source_root.rglob("*.rs")
        if path.is_file() and not is_test_source(path, source_root)
    )


def runtime_contract_files() -> list[Path]:
    files = production_rust_files(CLAWD_SRC) + production_rust_files(CORE_SRC)
    files.extend(sorted((ROOT / "configs").glob("*.toml")))
    files.extend(sorted((ROOT / "docker/config").glob("*.toml")))
    files.extend(sorted((ROOT / "prompts").rglob("*.md")))
    files.extend(sorted((ROOT / "prompts").rglob("*.json")))
    return files


def registry_skill_names() -> list[str]:
    parsed = tomllib.loads(REGISTRY.read_text(encoding="utf-8"))
    return sorted(
        {
            item["name"]
            for item in parsed.get("skills", [])
            if isinstance(item.get("name"), str) and len(item["name"]) >= 3
        },
        key=len,
        reverse=True,
    )


def scan_forbidden_runtime_tokens(files: list[Path]) -> list[Finding]:
    findings: list[Finding] = []
    for path in files:
        for line_no, line in enumerate(
            path.read_text(encoding="utf-8").splitlines(), start=1
        ):
            for kind, pattern in FORBIDDEN_RUNTIME_PATTERNS:
                if pattern.search(line):
                    findings.append(Finding(relative(path), line_no, kind, line.strip()))
    return findings


def scan_finalizer_registry_tokens(files: list[Path]) -> list[Finding]:
    findings: list[Finding] = []
    names = registry_skill_names()
    for path in files:
        for line_no, line in enumerate(
            path.read_text(encoding="utf-8").splitlines(), start=1
        ):
            for name in names:
                if re.search(rf"(?<![a-z0-9_]){re.escape(name)}(?![a-z0-9_])", line):
                    findings.append(
                        Finding(
                            relative(path),
                            line_no,
                            "finalizer_registry_skill_dependency",
                            f"{name}: {line.strip()}",
                        )
                    )
    return findings


def inventory() -> dict[str, object]:
    finalizer_production = production_rust_files(FINALIZE_DIR)
    runtime_findings = scan_forbidden_runtime_tokens(runtime_contract_files())
    registry_findings = scan_finalizer_registry_tokens(finalizer_production)
    return {
        "zero_domain_runtime_hits": len(runtime_findings),
        "zero_domain_runtime_findings": [dataclasses.asdict(item) for item in runtime_findings],
        "finalizer_registry_dependency_hits": len(registry_findings),
        "finalizer_registry_dependency_findings": [
            dataclasses.asdict(item) for item in registry_findings
        ],
        "finalizer_production_modules": len(finalizer_production),
        "finalizer_production_lines": sum(
            len(path.read_text(encoding="utf-8").splitlines())
            for path in finalizer_production
        ),
    }


def findings_for(metrics: dict[str, object]) -> list[str]:
    findings: list[str] = []
    if int(metrics["zero_domain_runtime_hits"]):
        findings.append(
            f"zero_domain_runtime_hits:{metrics['zero_domain_runtime_hits']}"
        )
    if int(metrics["finalizer_registry_dependency_hits"]):
        findings.append(
            "finalizer_registry_dependency_hits:"
            f"{metrics['finalizer_registry_dependency_hits']}"
        )
    modules = int(metrics["finalizer_production_modules"])
    lines = int(metrics["finalizer_production_lines"])
    if modules > MAX_FINALIZER_PRODUCTION_MODULES:
        findings.append(
            f"finalizer_production_modules_grew:{modules}>{MAX_FINALIZER_PRODUCTION_MODULES}"
        )
    if lines > MAX_FINALIZER_PRODUCTION_LINES:
        findings.append(
            f"finalizer_production_lines_grew:{lines}>{MAX_FINALIZER_PRODUCTION_LINES}"
        )
    for required in (ENVELOPE_SOURCE, SYNTHESIS_SOURCE, SYNTHESIS_PROMPT):
        if not required.exists():
            findings.append(f"generic_capability_result_file_missing:{relative(required)}")
    if ENVELOPE_SOURCE.exists():
        envelope = ENVELOPE_SOURCE.read_text(encoding="utf-8")
        for token in (
            "CapabilityResultEnvelope",
            "EvidenceRef",
            "ArtifactRef",
            "StructuredError",
            "Continuation",
            "CapabilityDeliveryIntent",
        ):
            if token not in envelope:
                findings.append(f"capability_result_contract_missing:{token}")
    return findings


def run_self_test() -> int:
    fixture = FINALIZE_DIR / "fixture.rs"
    sample = (
        'let kind = OutputSemanticKind::RawCommandOutput;\n'
        'let shape = RawOutputOrShortSummary;\n'
        'let helper = strict_raw_tail_read_answer();\n'
        'let branch = answer_incomplete_reason.contains("missing");\n'
        'let retry = retry_instruction.starts_with("collect");\n'
    )
    sample_findings: list[Finding] = []
    for line_no, line in enumerate(sample.splitlines(), start=1):
        for kind, pattern in FORBIDDEN_RUNTIME_PATTERNS:
            if pattern.search(line):
                sample_findings.append(Finding(relative(fixture), line_no, kind, line))
    assert {item.kind for item in sample_findings} == {
        "legacy_raw_output_type",
        "legacy_output_type",
        "legacy_raw_final_answer_shape",
        "legacy_raw_exact_helper",
        "answer_verifier_reason_prose_branch",
        "answer_verifier_instruction_prose_branch",
    }
    assert is_test_source(FINALIZE_DIR / "task_tests/final_status.rs", FINALIZE_DIR)
    assert not is_test_source(
        FINALIZE_DIR / "loop_reply_synthesis_preference.rs", FINALIZE_DIR
    )
    bad = {
        "zero_domain_runtime_hits": 1,
        "finalizer_registry_dependency_hits": 0,
        "finalizer_production_modules": MAX_FINALIZER_PRODUCTION_MODULES,
        "finalizer_production_lines": MAX_FINALIZER_PRODUCTION_LINES,
    }
    assert "zero_domain_runtime_hits:1" in findings_for(bad)
    print("FINALIZER_ARCHITECTURE_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--inventory", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    metrics = inventory()
    findings = findings_for(metrics)
    if args.inventory:
        print(json.dumps(metrics, indent=2, ensure_ascii=False))
    print(
        "FINALIZER_ARCHITECTURE_CHECK "
        f"findings={len(findings)} "
        f"zero_domain_hits={metrics['zero_domain_runtime_hits']} "
        f"registry_dependencies={metrics['finalizer_registry_dependency_hits']} "
        f"modules={metrics['finalizer_production_modules']} "
        f"lines={metrics['finalizer_production_lines']}"
    )
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
