#!/usr/bin/env python3
"""Guard direct output_contract.semantic_kind writes.

OutputSemanticKind is a compatibility/output-shape field, not route authority.
During the migration, direct writes are allowed only in approved boundary
modules that normalize legacy schema, repair structural output contracts, or
project compatibility task evidence. New production code should prefer typed
capability refs, final_answer_shape, or a dedicated output-contract facade.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "crates" / "clawd" / "src"

ASSIGNMENT = re.compile(
    r"\b(?:[A-Za-z_][A-Za-z0-9_]*\.)*output_contract\.semantic_kind\s*=(?!=)"
)

APPROVED_COMPATIBILITY_WRITERS = {
    SRC / "agent_engine" / "service_probe_contract.rs",
    SRC / "finalize" / "task_machine_kv_summary.rs",
    SRC / "intent_router_active_task_repair.rs",
    SRC / "intent_router_contract_hint.rs",
    SRC / "intent_router_current_turn_anchor.rs",
    SRC / "intent_router_current_turn_structural_repair.rs",
    SRC / "intent_router_directory_observation.rs",
    SRC / "intent_router_execution_contract.rs",
    SRC / "intent_router_route_output.rs",
    SRC / "intent_router_state_patch_fields.rs",
    SRC / "intent_router_structural_schedule.rs",
    SRC / "worker" / "ask_pipeline_boundary_preflight.rs",
    SRC / "worker" / "ask_prepare.rs",
    SRC / "worker" / "ask_prepare_field_contract.rs",
    SRC / "worker" / "ask_prepare_file_delivery.rs",
}


def is_test_file(path: Path) -> bool:
    rel = path.relative_to(ROOT).as_posix()
    parts = path.relative_to(ROOT).parts
    return (
        path.name.endswith("_tests.rs")
        or rel.endswith("_test.rs")
        or any(part == "tests" or part.endswith("_tests") for part in parts)
    )


def scan_text(path: Path, text: str) -> tuple[list[tuple[Path, int, str]], int]:
    findings: list[tuple[Path, int, str]] = []
    approved_hits = 0
    for lineno, line in enumerate(text.splitlines(), 1):
        if not ASSIGNMENT.search(line):
            continue
        if path in APPROVED_COMPATIBILITY_WRITERS:
            approved_hits += 1
            continue
        findings.append((path.relative_to(ROOT), lineno, line.strip()))
    return findings, approved_hits


def scan_repo() -> tuple[list[tuple[Path, int, str]], int]:
    findings: list[tuple[Path, int, str]] = []
    approved_hits = 0

    for path in SRC.rglob("*.rs"):
        if is_test_file(path):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        path_findings, path_approved_hits = scan_text(path, text)
        findings.extend(path_findings)
        approved_hits += path_approved_hits
    return findings, approved_hits


def run_self_test() -> int:
    bad = """
fn bad(route: &mut RouteResult) {
    route.output_contract.semantic_kind = OutputSemanticKind::Scalar;
}
"""
    bad_findings, bad_approved_hits = scan_text(SRC / "worker/new_boundary.rs", bad)
    assert bad_findings
    assert bad_approved_hits == 0

    approved = """
fn compatibility(route: &mut RouteResult) {
    route.output_contract.semantic_kind = OutputSemanticKind::Scalar;
}
"""
    approved_findings, approved_hits = scan_text(
        SRC / "intent_router_route_output.rs",
        approved,
    )
    assert not approved_findings
    assert approved_hits == 1

    good = """
fn good(route: &mut RouteResult) {
    route.output_contract.final_answer_shape = FinalAnswerShape::Scalar;
}
"""
    good_findings, good_approved_hits = scan_text(SRC / "worker/new_boundary.rs", good)
    assert not good_findings
    assert good_approved_hits == 0
    print("OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings, approved_hits = scan_repo()

    for rel, lineno, line in findings:
        print(f"{rel}:{lineno}: output_contract.semantic_kind direct write: {line}")
    print(
        "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_CHECK "
        f"findings={len(findings)} approved_hits={approved_hits}"
    )
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
