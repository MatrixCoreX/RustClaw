#!/usr/bin/env python3
"""Keep the legacy domain finalizer surface monotonic while it is deleted."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CLAWD_SRC = ROOT / "crates/clawd/src"
FINALIZE_DIR = CLAWD_SRC / "finalize"
PIPELINE_TYPES = CLAWD_SRC / "pipeline_types.rs"
REGISTRY = ROOT / "configs/skills_registry.toml"
ENVELOPE_SOURCE = ROOT / "crates/claw-core/src/capability_result.rs"
SYNTHESIS_SOURCE = (
    ROOT / "crates/clawd/src/agent_engine/capability_result_synthesis.rs"
)
SYNTHESIS_PROMPT = (
    ROOT / "prompts/layers/overlays/capability_result_synthesis_prompt.md"
)

# Baseline after introducing the generic envelope. These are ceilings, not
# targets. Track C must drive them toward 0/0/<=15_000 as legacy cohorts leave.
MAX_SEMANTIC_VARIANTS = 32
MAX_SEMANTIC_PRODUCTION_FILES = 78
MAX_FINALIZER_PRODUCTION_MODULES = 75
MAX_FINALIZER_PRODUCTION_LINES = 33_582
MAX_FINALIZER_REGISTRY_TOKEN_OCCURRENCES = 161


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
        if not is_test_source(path, source_root)
    )


def semantic_variants(raw: str) -> list[str]:
    marker = "enum OutputSemanticKind {"
    if marker not in raw:
        return []
    body = raw.split(marker, 1)[1].split("\n}", 1)[0]
    return [
        line.strip().rstrip(",")
        for line in body.splitlines()
        if re.fullmatch(r"[A-Za-z][A-Za-z0-9]*,?", line.strip())
    ]


def classify_semantic_owner(path: Path) -> str:
    rel = relative(path)
    if "/finalize/" in rel:
        if any(
            token in rel
            for token in (
                "machine",
                "exact",
                "raw_command",
                "scalar",
                "markdown",
            )
        ):
            return "exact_machine_serialization"
        if any(token in rel for token in ("file", "artifact", "delivery")):
            return "artifact_channel_delivery"
        if any(token in rel for token in ("task_", "resume", "lifecycle")):
            return "lifecycle_control"
        return "business_language_rendering"
    if any(
        token in rel
        for token in ("verifier", "contract", "resolver", "clarify", "policy")
    ):
        return "safety_policy_contract"
    if any(
        token in rel
        for token in ("observed_output", "evidence", "task_journal", "observed_facts")
    ):
        return "evidence_extraction"
    if any(token in rel for token in ("delivery", "output_paths")):
        return "artifact_channel_delivery"
    return "unclassified_legacy"


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


def registry_token_occurrences(files: list[Path]) -> int:
    names = registry_skill_names()
    total = 0
    for path in files:
        raw = path.read_text(encoding="utf-8")
        for name in names:
            total += len(
                re.findall(
                    rf"(?<![a-z0-9_]){re.escape(name)}(?![a-z0-9_])",
                    raw,
                )
            )
    return total


def inventory() -> dict[str, object]:
    production = production_rust_files(CLAWD_SRC)
    finalizer_production = production_rust_files(FINALIZE_DIR)
    semantic_files = [
        path
        for path in production
        if "OutputSemanticKind" in path.read_text(encoding="utf-8")
    ]
    owners = Counter(classify_semantic_owner(path) for path in semantic_files)
    return {
        "semantic_variants": len(
            semantic_variants(PIPELINE_TYPES.read_text(encoding="utf-8"))
        ),
        "semantic_production_files": len(semantic_files),
        "semantic_owner_categories": dict(sorted(owners.items())),
        "semantic_owner_files": [
            {
                "path": relative(path),
                "category": classify_semantic_owner(path),
            }
            for path in semantic_files
        ],
        "finalizer_production_modules": len(finalizer_production),
        "finalizer_production_lines": sum(
            len(path.read_text(encoding="utf-8").splitlines())
            for path in finalizer_production
        ),
        "finalizer_registry_token_occurrences": registry_token_occurrences(
            finalizer_production
        ),
    }


def findings_for(metrics: dict[str, object]) -> list[str]:
    findings: list[str] = []
    ceilings = {
        "semantic_variants": MAX_SEMANTIC_VARIANTS,
        "semantic_production_files": MAX_SEMANTIC_PRODUCTION_FILES,
        "finalizer_production_modules": MAX_FINALIZER_PRODUCTION_MODULES,
        "finalizer_production_lines": MAX_FINALIZER_PRODUCTION_LINES,
        "finalizer_registry_token_occurrences": (
            MAX_FINALIZER_REGISTRY_TOKEN_OCCURRENCES
        ),
    }
    for key, ceiling in ceilings.items():
        value = int(metrics[key])
        if value > ceiling:
            findings.append(f"{key}_grew:{value}>{ceiling}")
    for required in (ENVELOPE_SOURCE, SYNTHESIS_SOURCE, SYNTHESIS_PROMPT):
        if not required.exists():
            findings.append(f"generic_capability_result_file_missing:{relative(required)}")
            continue
        raw = required.read_text(encoding="utf-8")
        if "OutputSemanticKind" in raw:
            findings.append(
                f"generic_capability_result_depends_on_semantic_kind:{relative(required)}"
            )
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
    sample = """
#[derive(Default)]
enum OutputSemanticKind {
    None,
    ScalarCount,
}
"""
    assert semantic_variants(sample) == ["None", "ScalarCount"]
    assert classify_semantic_owner(
        FINALIZE_DIR / "loop_reply_git_state.rs"
    ) == "business_language_rendering"
    assert classify_semantic_owner(
        FINALIZE_DIR / "loop_reply_machine_kv.rs"
    ) == "exact_machine_serialization"
    assert is_test_source(
        FINALIZE_DIR / "task_tests/final_status.rs",
        FINALIZE_DIR,
    )
    assert not is_test_source(
        FINALIZE_DIR / "loop_reply_synthesis_preference.rs",
        FINALIZE_DIR,
    )
    bad = {
        "semantic_variants": MAX_SEMANTIC_VARIANTS + 1,
        "semantic_production_files": MAX_SEMANTIC_PRODUCTION_FILES,
        "finalizer_production_modules": MAX_FINALIZER_PRODUCTION_MODULES,
        "finalizer_production_lines": MAX_FINALIZER_PRODUCTION_LINES,
        "finalizer_registry_token_occurrences": (
            MAX_FINALIZER_REGISTRY_TOKEN_OCCURRENCES
        ),
    }
    assert any("semantic_variants_grew" in item for item in findings_for(bad))
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
        f"variants={metrics['semantic_variants']} "
        f"semantic_files={metrics['semantic_production_files']} "
        f"modules={metrics['finalizer_production_modules']} "
        f"lines={metrics['finalizer_production_lines']} "
        f"registry_tokens={metrics['finalizer_registry_token_occurrences']}"
    )
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
