#!/usr/bin/env python3
"""Guard removed pre-planner/direct-answer gate inventory from returning.

The old script used to require an inventory for pre-planner semantic exits.
After the agent-loop migration, those exits are deleted. This check keeps the
historical script entry point but now enforces the new invariant: no production
Rust module may reintroduce the old inventory files or direct-answer gate
promotion tokens.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
DECISION_INVENTORY_PATH = ROOT / "scripts/inventories/pre_planner_decisions.toml"

OWNER_CATEGORIES = {
    "attachment_boundary",
    "context_boundary",
    "explicit_machine_command_boundary",
    "explicit_schedule_boundary",
    "lifecycle_boundary",
    "safety_boundary",
    "semantic_authority",
    "session_boundary",
    "target_semantic_owner",
}
RETAINED_BOUNDARY_OWNER_CATEGORIES = {
    "attachment_boundary",
    "context_boundary",
    "explicit_machine_command_boundary",
    "explicit_schedule_boundary",
    "lifecycle_boundary",
    "safety_boundary",
    "session_boundary",
    "target_semantic_owner",
}
DISPOSITIONS = {
    "delete_after_migration",
    "move_to_agent_loop",
    "retain_boundary",
}
REQUIRED_DECISION_IDS = {
    "attachment_audio_materialization",
    "ui_attachment_projection",
    "explicit_machine_command_projection",
    "turn_boundary_envelope",
    "execution_context_materialization",
    "explicit_schedule_direct_text_boundary",
    "neutral_agent_loop_frontdoor",
    "agent_loop_semantic_entry",
}

REMOVED_FILES = (
    "crates/clawd/src/ask_flow_pre_planner_exit.rs",
    "crates/clawd/src/ask_flow_gate_execution.rs",
    "crates/clawd/src/ask_flow_gate_policy.rs",
    "crates/clawd/src/ask_flow_gate_contract.rs",
    "crates/clawd/src/ask_flow_chat_helpers.rs",
    "crates/clawd/src/worker/ask_prepare.rs",
    "crates/clawd/src/worker/ask_pipeline.rs",
    "crates/clawd/src/intent_router.rs",
)

REMOVED_FILE_PREFIXES = ("intent_router_", "ask_prepare_", "ask_pipeline_")

FORBIDDEN_PRODUCTION_TOKENS = (
    "PRE_PLANNER_EXIT_INVENTORY",
    "with_pre_planner_exit_snapshot",
    "pre_planner_exit_for_reason",
    "direct_answer_gate_planner_promotion_reason_code",
    "direct_answer_gate_boundary_class",
    "direct_answer_gate_ownership_class",
    "direct_answer_gate_boundary_class_is_boundary_owned",
    "run_intent_normalizer(",
    "maybe_handle_ask_self_extension(",
)

DOC_FORBIDDEN_STALE_TOKENS = {
    "docs/legacy_semantic_route_inventory.md": (
        "Can answer before tool loop",
        "`keep_boundary` for fallback safety; `delete_after_canary`",
    ),
    "docs/compat_cleanup_inventory.md": (
        "PRE_PLANNER_EXIT_INVENTORY_CHECK ok calls=",
        "Non-deleting direct-answer gate exits",
        "Ordinary semantic exits carry",
        "direct-answer gate promotion/chat fallback",
    ),
    "docs/planner_loop_pre_agent_gate_audit.md": (
        "If a new direct-answer gate reason is introduced",
        "when a new gate is added",
    ),
}


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
        path for path in SRC_ROOT.rglob("*.rs") if path.is_file() and not is_test_path(path)
    )


def load_decision_inventory() -> dict:
    return tomllib.loads(DECISION_INVENTORY_PATH.read_text(encoding="utf-8"))


def validate_decision_inventory(data: dict) -> list[str]:
    findings: list[str] = []
    if data.get("schema_version") != 1:
        findings.append("decision_inventory: schema_version_must_be_1")
    if data.get("target_semantic_owner") != "agent_loop":
        findings.append("decision_inventory: target_semantic_owner_must_be_agent_loop")
    decisions = data.get("decisions")
    if not isinstance(decisions, list):
        return findings + ["decision_inventory: decisions_must_be_array"]

    seen: set[str] = set()
    for index, decision in enumerate(decisions):
        prefix = f"decision_inventory[{index}]"
        if not isinstance(decision, dict):
            findings.append(f"{prefix}: entry_must_be_table")
            continue
        decision_id = decision.get("id")
        if not isinstance(decision_id, str) or not decision_id:
            findings.append(f"{prefix}: missing_id")
            continue
        prefix = f"decision_inventory:{decision_id}"
        if decision_id in seen:
            findings.append(f"{prefix}: duplicate_id")
        seen.add(decision_id)

        path_value = decision.get("path")
        if not isinstance(path_value, str) or not path_value.startswith("crates/clawd/src/"):
            findings.append(f"{prefix}: invalid_production_path")
            continue
        path = ROOT / path_value
        if not path.is_file() or is_test_path(path):
            findings.append(f"{prefix}: production_path_missing_or_test")
            continue
        body = path.read_text(encoding="utf-8")
        symbols = decision.get("symbols")
        if not isinstance(symbols, list) or not symbols:
            findings.append(f"{prefix}: missing_symbols")
        else:
            for symbol in symbols:
                if not isinstance(symbol, str) or not symbol:
                    findings.append(f"{prefix}: invalid_symbol")
                elif symbol not in body:
                    findings.append(f"{prefix}: missing_symbol:{symbol}")

        owner = decision.get("owner_category")
        if owner not in OWNER_CATEGORIES:
            findings.append(f"{prefix}: invalid_owner_category:{owner}")
        disposition = decision.get("disposition")
        if disposition not in DISPOSITIONS:
            findings.append(f"{prefix}: invalid_disposition:{disposition}")
        if (
            disposition == "retain_boundary"
            and owner not in RETAINED_BOUNDARY_OWNER_CATEGORIES
        ):
            findings.append(f"{prefix}: retained_owner_not_allowlisted:{owner}")
        for field in ("input_fields", "output_fields"):
            values = decision.get(field)
            if not isinstance(values, list) or not values or not all(
                isinstance(value, str) and value for value in values
            ):
                findings.append(f"{prefix}: invalid_{field}")
        semantic = decision.get("ordinary_semantic_authority")
        terminal = decision.get("terminal_before_planner")
        if not isinstance(semantic, bool) or not isinstance(terminal, bool):
            findings.append(f"{prefix}: authority_and_terminal_must_be_bool")
        if semantic and owner != "target_semantic_owner" and disposition == "retain_boundary":
            findings.append(f"{prefix}: ordinary_semantics_cannot_remain_boundary_owned")
        if terminal and semantic and disposition == "retain_boundary":
            findings.append(f"{prefix}: semantic_terminal_exit_cannot_be_retained")

    for missing in sorted(REQUIRED_DECISION_IDS - seen):
        findings.append(f"decision_inventory: missing_required_id:{missing}")
    return findings


def scan_repo() -> list[str]:
    findings: list[str] = []
    if not DECISION_INVENTORY_PATH.is_file():
        findings.append("scripts/inventories/pre_planner_decisions.toml: missing_inventory")
    else:
        try:
            findings.extend(validate_decision_inventory(load_decision_inventory()))
        except (OSError, tomllib.TOMLDecodeError) as exc:
            findings.append(f"decision_inventory: load_failed:{exc.__class__.__name__}")
    for removed in REMOVED_FILES:
        path = ROOT / removed
        if path.exists():
            findings.append(f"{removed}: removed_pre_planner_file_returned")
    for path in production_rust_files():
        if Path(rel(path)).name.startswith(REMOVED_FILE_PREFIXES):
            findings.append(f"{rel(path)}: removed_pre_planner_file_prefix_returned")
    for path in production_rust_files():
        raw = path.read_text(encoding="utf-8")
        for token in FORBIDDEN_PRODUCTION_TOKENS:
            if token in raw:
                findings.append(f"{rel(path)}: forbidden_pre_planner_token:{token}")
    for rel_path, tokens in DOC_FORBIDDEN_STALE_TOKENS.items():
        path = ROOT / rel_path
        if not path.exists():
            continue
        try:
            raw = path.read_text(encoding="utf-8")
        except OSError as exc:
            findings.append(f"{rel_path}: docs_read_failed:{exc.__class__.__name__}")
            continue
        for token in tokens:
            if token in raw:
                findings.append(f"{rel_path}: stale_pre_planner_docs_token:{token}")
    return findings


def run_self_test() -> int:
    assert "direct_answer_gate_boundary_class" in FORBIDDEN_PRODUCTION_TOKENS
    assert "crates/clawd/src/worker/ask_pipeline.rs" in REMOVED_FILES
    assert "run_intent_normalizer(" in FORBIDDEN_PRODUCTION_TOKENS
    assert "semantic_authority" not in RETAINED_BOUNDARY_OWNER_CATEGORIES
    assert "Can answer before tool loop" in DOC_FORBIDDEN_STALE_TOKENS[
        "docs/legacy_semantic_route_inventory.md"
    ]
    data = load_decision_inventory()
    assert not validate_decision_inventory(data)
    invalid = {
        "schema_version": 1,
        "target_semantic_owner": "agent_loop",
        "decisions": [
            {
                "id": "invalid_semantic_owner",
                "path": "crates/clawd/src/worker/ask_runtime.rs",
                "symbols": ["run_agent_with_tools"],
                "owner_category": "semantic_authority",
                "input_fields": ["prompt"],
                "output_fields": ["route"],
                "disposition": "retain_boundary",
                "ordinary_semantic_authority": True,
                "terminal_before_planner": True,
            }
        ],
    }
    invalid_findings = validate_decision_inventory(invalid)
    assert any("ordinary_semantics_cannot_remain_boundary_owned" in item for item in invalid_findings)
    assert any("semantic_terminal_exit_cannot_be_retained" in item for item in invalid_findings)
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_repo()
    if args.json:
        inventory = load_decision_inventory() if DECISION_INVENTORY_PATH.is_file() else {}
        print(json.dumps({"findings": findings, "inventory": inventory}, ensure_ascii=True, sort_keys=True))
        return 1 if findings else 0
    print(f"PRE_PLANNER_EXIT_REMOVAL_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
