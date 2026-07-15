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
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"

REMOVED_FILES = (
    "crates/clawd/src/ask_flow_pre_planner_exit.rs",
    "crates/clawd/src/ask_flow_gate_execution.rs",
    "crates/clawd/src/ask_flow_gate_policy.rs",
    "crates/clawd/src/ask_flow_gate_contract.rs",
    "crates/clawd/src/ask_flow_chat_helpers.rs",
)

FORBIDDEN_PRODUCTION_TOKENS = (
    "PRE_PLANNER_EXIT_INVENTORY",
    "with_pre_planner_exit_snapshot",
    "pre_planner_exit_for_reason",
    "direct_answer_gate_planner_promotion_reason_code",
    "direct_answer_gate_boundary_class",
    "direct_answer_gate_ownership_class",
    "direct_answer_gate_boundary_class_is_boundary_owned",
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


def scan_repo() -> list[str]:
    findings: list[str] = []
    for removed in REMOVED_FILES:
        path = ROOT / removed
        if path.exists():
            findings.append(f"{removed}: removed_pre_planner_file_returned")
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
    assert "crates/clawd/src/ask_flow_pre_planner_exit.rs" in REMOVED_FILES
    assert "Can answer before tool loop" in DOC_FORBIDDEN_STALE_TOKENS[
        "docs/legacy_semantic_route_inventory.md"
    ]
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_repo()
    print(f"PRE_PLANNER_EXIT_REMOVAL_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
