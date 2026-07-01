#!/usr/bin/env python3
"""Guard deterministic decision inventory coverage after agent-loop migration.

The inventory is intentionally module-scoped. It is not a semantic router and
does not inspect user language. Its job is to ensure every production module in
the convergence plan's target file families has an explicit owner-category
classification. Deleted pre-planner semantic routing files are intentionally not
part of this inventory.
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


@dataclasses.dataclass(frozen=True)
class BranchInventoryEntry:
    name: str
    path: str
    category: str
    input_fields: tuple[str, ...]
    output_fields: tuple[str, ...]
    tokens: tuple[str, ...]
    reads_user_prompt_text: bool = False
    reads_model_answer_text: bool = False
    reads_skill_text_fields: bool = False
    reads_skill_error_text_fields: bool = False


INVENTORY: tuple[InventoryEntry, ...] = (
    InventoryEntry(
        name="ask_flow_boundary_and_media_preparation",
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
        name="planner_deterministic_helpers",
        patterns=(
            "crates/clawd/src/agent_engine/planning*.rs",
            "crates/clawd/src/agent_engine/planning/**/*.rs",
        ),
        categories=(
            "contract_boundary",
            "evidence_projection",
            "recovery_boundary",
        ),
        reads_user_prompt_text=True,
        reads_model_answer_text=True,
        reads_skill_text_fields=False,
        reads_skill_error_text_fields=False,
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


BRANCH_INVENTORY: tuple[BranchInventoryEntry, ...] = (
    BranchInventoryEntry(
        name="planner_structural_route_markers",
        path="crates/clawd/src/agent_engine/planning_route_markers.rs",
        category="contract_boundary",
        input_fields=("route_reason", "needs_clarify"),
        output_fields=("structured_marker_match",),
        tokens=("route_reason_has_structural_marker", "route_has_unresolved_clarify_or_locator_marker"),
    ),
    BranchInventoryEntry(
        name="recent_artifact_selector_machine_tokens",
        path="crates/clawd/src/agent_engine/planning_recent_artifacts.rs",
        category="contract_boundary",
        input_fields=("resolved_intent", "route_reason", "list_selector"),
        output_fields=("selector_target_kind", "selector_limit", "selector_sort_by"),
        tokens=("selector_value_machine_token", "selector_bool_machine_token"),
    ),
    BranchInventoryEntry(
        name="observed_output_generic_projection",
        path="crates/clawd/src/agent_engine/observed_output.rs",
        category="evidence_projection",
        input_fields=("structured_observation", "output_contract"),
        output_fields=("scalar", "path", "list", "status", "artifact_refs"),
        tokens=("extract_direct_answer_from_generic_output", "extract_direct_scalar_from_generic_output"),
    ),
    BranchInventoryEntry(
        name="observed_output_git_machine_summary",
        path="crates/clawd/src/agent_engine/observed_output_git.rs",
        category="evidence_projection",
        input_fields=("git_basic.structured_json_v1", "status_output"),
        output_fields=("git.branch", "git.worktree"),
        tokens=("git_repository_state_answer", "answer_is_git_repository_state_machine_summary"),
    ),
    BranchInventoryEntry(
        name="answer_verifier_missing_evidence_gap",
        path="crates/clawd/src/answer_verifier_runtime.rs",
        category="recovery_boundary",
        input_fields=("required_evidence_fields", "observed_evidence"),
        output_fields=("missing_evidence_fields", "retry_instruction"),
        tokens=("answer_verifier_observed_gap", "missing_evidence_fields"),
        reads_model_answer_text=True,
    ),
    BranchInventoryEntry(
        name="finalizer_requested_machine_kv_summary",
        path="crates/clawd/src/finalize/loop_reply.rs",
        category="evidence_projection",
        input_fields=("state_patch", "observed_values", "output_contract"),
        output_fields=("requested_machine_kv_summary",),
        tokens=("requested_machine_kv_summary", "log_deterministic_delivery_record"),
        reads_model_answer_text=True,
    ),
    BranchInventoryEntry(
        name="finalizer_verifier_failure_message_key",
        path="crates/clawd/src/finalize/task_answer_verifier_failure.rs",
        category="contract_boundary",
        input_fields=("AnswerVerifierError", "missing_evidence_fields"),
        output_fields=("message_key", "reason_code", "status_code"),
        tokens=("answer_verifier_required_evidence_block", "missing_evidence_fields"),
    ),
    BranchInventoryEntry(
        name="deterministic_delivery_trace_record",
        path="crates/clawd/src/finalize/loop_reply_delivery_record.rs",
        category="compat_trace",
        input_fields=("reason_code", "semantic_kind", "response_shape"),
        output_fields=("deterministic_delivery_record",),
        tokens=("log_deterministic_delivery_record", "deterministic_delivery_record"),
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
            findings.append(f"{entry.name}: semantic_rewrite_not_allowed_in_current_inventory")
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


def validate_branch_inventory() -> list[str]:
    findings: list[str] = []
    seen_names: set[str] = set()
    for entry in BRANCH_INVENTORY:
        if entry.name in seen_names:
            findings.append(f"duplicate_branch_inventory_name={entry.name}")
        seen_names.add(entry.name)
        if entry.category not in OWNER_CATEGORIES:
            findings.append(f"{entry.name}: unknown_branch_category={entry.category}")
        if entry.category in {"unknown", "semantic_rewrite"}:
            findings.append(f"{entry.name}: branch_category_not_allowed={entry.category}")
        if not entry.input_fields:
            findings.append(f"{entry.name}: missing_input_fields")
        if not entry.output_fields:
            findings.append(f"{entry.name}: missing_output_fields")
        if entry.reads_skill_text_fields or entry.reads_skill_error_text_fields:
            findings.append(f"{entry.name}: reads_visible_skill_text_protocol")
        path = ROOT / entry.path
        if not path.is_file():
            findings.append(f"{entry.name}: missing_path={entry.path}")
            continue
        body = path.read_text(encoding="utf-8")
        for token in entry.tokens:
            if token not in body:
                findings.append(f"{entry.name}: missing_token={token}")
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
    print(f"branch_inventory_entries={len(BRANCH_INVENTORY)}")
    for entry in BRANCH_INVENTORY:
        print(
            "{} file={} category={} inputs={} outputs={} tokens={} reads=user_prompt:{} "
            "model_answer:{} skill_text:{} skill_error_text:{}".format(
                entry.name,
                entry.path,
                entry.category,
                ",".join(entry.input_fields),
                ",".join(entry.output_fields),
                ",".join(entry.tokens),
                str(entry.reads_user_prompt_text).lower(),
                str(entry.reads_model_answer_text).lower(),
                str(entry.reads_skill_text_fields).lower(),
                str(entry.reads_skill_error_text_fields).lower(),
            )
        )


def run_self_test() -> int:
    assert matches_any(
        "crates/clawd/src/ask_flow.rs", TARGET_PATTERNS
    )
    assert is_test_path(ROOT / "crates/clawd/src/answer_verifier_tests.rs")
    planning_entries = covering_entries("crates/clawd/src/agent_engine/planning.rs")
    assert not any("semantic_rewrite" in entry.categories for entry in planning_entries)
    assert any(
        entry.name == "planner_structural_route_markers"
        for entry in BRANCH_INVENTORY
    )
    assert any(
        entry.name == "answer_verifier_missing_evidence_gap"
        and entry.category == "recovery_boundary"
        for entry in BRANCH_INVENTORY
    )
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
    findings = (
        validate_inventory_shape()
        + validate_branch_inventory()
        + validate_target_coverage()
    )
    if args.summary:
        print_summary()
    if findings:
        print(f"DETERMINISTIC_DECISION_INVENTORY_CHECK findings={len(findings)}")
        for finding in findings:
            print(f"  - {finding}")
        return 1
    print(
        "DETERMINISTIC_DECISION_INVENTORY_CHECK ok "
        f"target_files={len(target_files())} entries={len(INVENTORY)} "
        f"branch_entries={len(BRANCH_INVENTORY)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
