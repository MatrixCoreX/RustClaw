#!/usr/bin/env python3
"""Guard deterministic decision inventory coverage for runtime convergence.

The inventory is intentionally module-scoped. It is not a semantic router and
does not inspect user language. Its job is to ensure every production module in
the convergence plan's target file families has an explicit owner-category
classification and, when semantic migration debt is acknowledged, a non-runtime
migration target.
"""

from __future__ import annotations

import argparse
import dataclasses
import fnmatch
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"

OWNER_CATEGORIES = {
    "contract_boundary",
    "safety_policy",
    "permission_policy",
    "evidence_projection",
    "lifecycle_projection",
    "recovery_boundary",
    "compat_trace",
    "semantic_rewrite",
    "unknown",
}

SEMANTIC_MIGRATION_TARGETS = {
    "move_to_planner",
    "move_to_registry",
    "move_to_interface",
    "move_to_prompt_patch",
    "replace_with_schema",
    "delete_after_gate",
}

TARGET_PATTERNS = (
    "crates/clawd/src/ask_flow*.rs",
    "crates/clawd/src/agent_engine/planning*.rs",
    "crates/clawd/src/agent_engine/planning/**/*.rs",
    "crates/clawd/src/agent_engine/observed_output*.rs",
    "crates/clawd/src/agent_engine/observed_output/**/*.rs",
    "crates/clawd/src/answer_verifier*.rs",
    "crates/clawd/src/finalize/*.rs",
)


@dataclasses.dataclass(frozen=True)
class InventoryEntry:
    name: str
    patterns: tuple[str, ...]
    categories: tuple[str, ...]
    reads_user_prompt_text: bool
    reads_model_answer_text: bool
    reads_skill_text_fields: bool
    reads_skill_error_text_fields: bool
    migration_targets: tuple[str, ...] = ()


INVENTORY: tuple[InventoryEntry, ...] = (
    InventoryEntry(
        name="ask_flow_boundary_and_pre_planner",
        patterns=("crates/clawd/src/ask_flow*.rs",),
        categories=(
            "contract_boundary",
            "safety_policy",
            "evidence_projection",
            "lifecycle_projection",
            "compat_trace",
        ),
        reads_user_prompt_text=True,
        reads_model_answer_text=True,
        reads_skill_text_fields=False,
        reads_skill_error_text_fields=False,
    ),
    InventoryEntry(
        name="ask_flow_residual_semantic_debt",
        patterns=(
            "crates/clawd/src/ask_flow.rs",
            "crates/clawd/src/ask_flow_gate_execution.rs",
            "crates/clawd/src/ask_flow_pre_planner_exit.rs",
        ),
        categories=("semantic_rewrite",),
        reads_user_prompt_text=True,
        reads_model_answer_text=True,
        reads_skill_text_fields=False,
        reads_skill_error_text_fields=False,
        migration_targets=("move_to_planner", "delete_after_gate"),
    ),
    InventoryEntry(
        name="planner_deterministic_helpers",
        patterns=(
            "crates/clawd/src/agent_engine/planning*.rs",
            "crates/clawd/src/agent_engine/planning/**/*.rs",
        ),
        categories=(
            "contract_boundary",
            "evidence_projection",
            "recovery_boundary",
            "semantic_rewrite",
        ),
        reads_user_prompt_text=True,
        reads_model_answer_text=True,
        reads_skill_text_fields=False,
        reads_skill_error_text_fields=False,
        migration_targets=(
            "move_to_registry",
            "move_to_interface",
            "move_to_prompt_patch",
            "replace_with_schema",
        ),
    ),
    InventoryEntry(
        name="observed_output_projection",
        patterns=(
            "crates/clawd/src/agent_engine/observed_output*.rs",
            "crates/clawd/src/agent_engine/observed_output/**/*.rs",
        ),
        categories=("contract_boundary", "evidence_projection"),
        reads_user_prompt_text=False,
        reads_model_answer_text=False,
        reads_skill_text_fields=False,
        reads_skill_error_text_fields=False,
    ),
    InventoryEntry(
        name="answer_verifier_contract_boundary",
        patterns=("crates/clawd/src/answer_verifier*.rs",),
        categories=("contract_boundary", "recovery_boundary"),
        reads_user_prompt_text=False,
        reads_model_answer_text=True,
        reads_skill_text_fields=False,
        reads_skill_error_text_fields=False,
    ),
    InventoryEntry(
        name="finalizer_projection_and_lifecycle",
        patterns=("crates/clawd/src/finalize/*.rs",),
        categories=(
            "contract_boundary",
            "evidence_projection",
            "lifecycle_projection",
            "recovery_boundary",
        ),
        reads_user_prompt_text=False,
        reads_model_answer_text=True,
        reads_skill_text_fields=False,
        reads_skill_error_text_fields=False,
    ),
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    if rel_path.endswith(("_tests.rs", "tests.rs")):
        return True
    return any(part == "tests" or part.endswith("_tests") for part in parts)


def matches_any(path: str, patterns: tuple[str, ...]) -> bool:
    return any(fnmatch.fnmatch(path, pattern) for pattern in patterns)


def target_files() -> list[Path]:
    files: list[Path] = []
    for path in SRC_ROOT.rglob("*.rs"):
        if not path.is_file() or is_test_path(path):
            continue
        if matches_any(rel(path), TARGET_PATTERNS):
            files.append(path)
    return sorted(files)


def covering_entries(path: str) -> list[InventoryEntry]:
    return [entry for entry in INVENTORY if matches_any(path, entry.patterns)]


def validate_inventory_shape() -> list[str]:
    findings: list[str] = []
    seen_names: set[str] = set()
    for entry in INVENTORY:
        if entry.name in seen_names:
            findings.append(f"duplicate_inventory_name={entry.name}")
        seen_names.add(entry.name)
        unknown_categories = sorted(set(entry.categories) - OWNER_CATEGORIES)
        if unknown_categories:
            findings.append(
                f"{entry.name}: unknown_categories={','.join(unknown_categories)}"
            )
        if "unknown" in entry.categories:
            findings.append(f"{entry.name}: unknown_category_not_allowed_in_final_inventory")
        if "semantic_rewrite" in entry.categories:
            missing_targets = sorted(
                set(entry.migration_targets) - SEMANTIC_MIGRATION_TARGETS
            )
            if missing_targets:
                findings.append(
                    f"{entry.name}: unknown_migration_targets={','.join(missing_targets)}"
                )
            if not entry.migration_targets:
                findings.append(f"{entry.name}: semantic_rewrite_missing_migration_target")
        elif entry.migration_targets:
            findings.append(f"{entry.name}: migration_targets_without_semantic_rewrite")
    return findings


def validate_target_coverage() -> list[str]:
    findings: list[str] = []
    for path in target_files():
        rel_path = rel(path)
        entries = covering_entries(rel_path)
        if not entries:
            findings.append(f"{rel_path}: missing_inventory_entry")
            continue
        categories = {category for entry in entries for category in entry.categories}
        if "semantic_rewrite" in categories:
            targets = {
                target for entry in entries for target in entry.migration_targets
            }
            if not targets:
                findings.append(f"{rel_path}: semantic_rewrite_without_target")
    return findings


def print_summary() -> None:
    print(f"target_files={len(target_files())}")
    for entry in INVENTORY:
        matched = sorted(
            rel(path) for path in target_files() if matches_any(rel(path), entry.patterns)
        )
        print(
            "{} files={} categories={} migrations={} reads=user_prompt:{} model_answer:{} "
            "skill_text:{} skill_error_text:{}".format(
                entry.name,
                len(matched),
                ",".join(entry.categories),
                ",".join(entry.migration_targets) or "none",
                str(entry.reads_user_prompt_text).lower(),
                str(entry.reads_model_answer_text).lower(),
                str(entry.reads_skill_text_fields).lower(),
                str(entry.reads_skill_error_text_fields).lower(),
            )
        )


def run_self_test() -> int:
    assert matches_any(
        "crates/clawd/src/ask_flow_gate_execution.rs", TARGET_PATTERNS
    )
    assert is_test_path(ROOT / "crates/clawd/src/answer_verifier_tests.rs")
    planning_entries = covering_entries("crates/clawd/src/agent_engine/planning.rs")
    assert any("semantic_rewrite" in entry.categories for entry in planning_entries)
    observed_entries = covering_entries(
        "crates/clawd/src/agent_engine/observed_output.rs"
    )
    assert not any("semantic_rewrite" in entry.categories for entry in observed_entries)
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--summary", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = validate_inventory_shape() + validate_target_coverage()
    if args.summary:
        print_summary()
    if findings:
        print(f"DETERMINISTIC_DECISION_INVENTORY_CHECK findings={len(findings)}")
        for finding in findings:
            print(f"  - {finding}")
        return 1
    print(
        "DETERMINISTIC_DECISION_INVENTORY_CHECK ok "
        f"target_files={len(target_files())} entries={len(INVENTORY)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
