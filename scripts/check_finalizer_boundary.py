#!/usr/bin/env python3
"""Guard finalizer modules from becoming planner/dispatch code.

The finalizer may render user-visible replies from observed evidence, apply
language policy, and produce structured fallback contracts. It must not choose
or dispatch new capabilities/tools/skills after the agent loop has planned and
executed actions.
"""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FINALIZE_ROOT = ROOT / "crates/clawd/src/finalize"


@dataclasses.dataclass(frozen=True)
class BoundaryRule:
    name: str
    pattern: re.Pattern[str]


RULES: tuple[BoundaryRule, ...] = (
    BoundaryRule(
        name="planner_action_emission",
        pattern=re.compile(
            r"\b("
            r"call_capability|call_skill|call_tool|"
            r"CallCapability|CallSkill|CallTool|"
            r"AgentAction::CallCapability|AgentAction::CallSkill|AgentAction::CallTool"
            r")\b"
        ),
    ),
    BoundaryRule(
        name="runtime_dispatch",
        pattern=re.compile(
            r"\b("
            r"run_skill_with_runner|execution_adapters::run_skill|"
            r"CapabilityResolver|PlanVerifier|run_agent_with_tools"
            r")\b"
        ),
    ),
    BoundaryRule(
        name="action_policy_router",
        pattern=re.compile(r"\bcontract_matrix::capability_ref_action_policy_for_route\b"),
    ),
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    return rel_path.endswith(("_tests.rs", "tests.rs")) or any(
        part == "tests" or part.endswith("_tests") for part in parts
    )


def finalizer_files() -> list[Path]:
    if not FINALIZE_ROOT.is_dir():
        return []
    return sorted(
        path for path in FINALIZE_ROOT.rglob("*.rs") if path.is_file() and not is_test_path(path)
    )


def scan_repo() -> list[str]:
    findings: list[str] = []
    for path in finalizer_files():
        rel_path = rel(path)
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            for rule in RULES:
                if rule.pattern.search(line):
                    findings.append(
                        f"{rel_path}:{line_no}: {rule.name}_outside_finalizer_boundary: {line.strip()}"
                    )
    return findings


def run_self_test() -> int:
    action_rule = next(rule for rule in RULES if rule.name == "planner_action_emission")
    dispatch_rule = next(rule for rule in RULES if rule.name == "runtime_dispatch")
    policy_rule = next(rule for rule in RULES if rule.name == "action_policy_router")
    assert action_rule.pattern.search("AgentAction::CallSkill")
    assert action_rule.pattern.search('"call_capability"')
    assert dispatch_rule.pattern.search("CapabilityResolver")
    assert policy_rule.pattern.search("contract_matrix::capability_ref_action_policy_for_route")
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_repo()
    if findings:
        print(f"FINALIZER_BOUNDARY_CHECK findings={len(findings)}")
        for finding in findings:
            print(f"  - {finding}")
        return 1
    print(
        "FINALIZER_BOUNDARY_CHECK ok "
        f"files={len(finalizer_files())} rules={len(RULES)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
