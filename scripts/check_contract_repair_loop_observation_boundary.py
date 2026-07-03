#!/usr/bin/env python3
"""Guard worker contract repair stays loop-observation-only.

The Codex/Claude-style migration keeps ordinary semantic repair inside the
agent loop. Worker-side contract repair may expose structured machine
candidates to the loop, but it must not mutate RouteResult, gate kind,
output_contract, or route reason before the planner.
"""
from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "crates/clawd/src/worker/ask_pipeline_contract_repair.rs"

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    (
        "mutable_route_result_param",
        re.compile(r"\broute_result\s*:\s*&mut\s+(?:crate::)?RouteResult\b"),
    ),
    (
        "mutable_route_result_binding",
        re.compile(r"\bmut\s+route_result\b"),
    ),
    (
        "route_result_field_assignment",
        re.compile(r"\broute_result\.[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?\s*="),
    ),
    (
        "route_result_field_mutation_call",
        re.compile(
            r"\broute_result\.[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?"
            r"\.(?:push|push_str|clear|truncate|extend|insert|remove)\s*\("
        ),
    ),
    (
        "route_gate_mutation",
        re.compile(r"\b(?:route_result\.)?set_(?:clarify|chat|execute)_gate\s*\("),
    ),
    (
        "route_reason_mutation_helper",
        re.compile(r"\b(?:append|push|set)_route_reason(?:_marker)?\s*\("),
    ),
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def scan_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in FORBIDDEN_PATTERNS:
            if pattern.search(line):
                findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_repo() -> list[Finding]:
    return scan_text(rel(TARGET), TARGET.read_text(encoding="utf-8"))


def print_report(findings: list[Finding]) -> int:
    print(f"CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings={len(findings)}")
    for item in findings:
        print(f"  - {item.path}:{item.line} [{item.kind}] {item.text}")
    return 1 if findings else 0


def run_self_test() -> int:
    rel_path = "crates/clawd/src/worker/ask_pipeline_contract_repair.rs"
    assert scan_text(rel_path, "fn f(route_result: &mut crate::RouteResult) {}")
    assert scan_text(rel_path, "let mut route_result = route_result.clone();")
    assert scan_text(rel_path, "route_result.output_contract.semantic_kind = OutputSemanticKind::None;")
    assert scan_text(rel_path, "route_result.route_reason.push_str(\";contract_repair\");")
    assert scan_text(rel_path, "route_result.set_clarify_gate();")
    assert scan_text(rel_path, "append_route_reason_marker(route_result, \"x\");")
    assert not scan_text(
        rel_path,
        "json!({ \"source\": \"contract_repair\", \"contract_ref\": contract_ref })",
    )
    assert not scan_repo()
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    return print_report(scan_repo())


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
