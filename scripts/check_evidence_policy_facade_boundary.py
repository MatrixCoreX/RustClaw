#!/usr/bin/env python3
"""Guard production runtime from bypassing the EvidencePolicy facade.

The `contract_matrix` modules still own the bundled evidence-policy backing
tables during migration. Production call sites outside that backing layer must
go through `crates/clawd/src/evidence_policy.rs` so the old TaskContract matrix
cannot quietly regain ordinary semantic routing authority.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates" / "clawd" / "src"

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    (
        "direct_contract_matrix_path",
        re.compile(r"\b(?:crate::|super::)*contract_matrix::"),
    ),
    (
        "direct_contract_matrix_use",
        re.compile(r"\buse\s+(?:crate::|super::)*contract_matrix(?:\s*::|\s*;)"),
    ),
    (
        "direct_task_contract_path",
        re.compile(r"\b(?:crate::|super::)*task_contract::"),
    ),
    (
        "direct_task_contract_use",
        re.compile(r"\buse\s+(?:crate::|super::)*task_contract(?:\s*::|\s*;)"),
    ),
)

BACKING_FILES = {
    "contract_matrix.rs",
    "contract_matrix_runtime.rs",
    "evidence_policy.rs",
    "task_contract.rs",
}

BASELINE_TASK_CONTRACT_FILES = {
    "crates/clawd/src/agent_engine/directory_entry_group_locator.rs",
    "crates/clawd/src/agent_engine/explicit_observed_paths.rs",
    "crates/clawd/src/agent_engine/session_alias_target_coverage.rs",
    "crates/clawd/src/agent_engine/support.rs",
    "crates/clawd/src/finalize/loop_reply_exact_contract.rs",
    "crates/clawd/src/finalize/loop_reply_machine_kv.rs",
    "crates/clawd/src/finalize/loop_reply_observed_contract.rs",
    "crates/clawd/src/finalize/loop_reply_quantity.rs",
}


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    if rel_path.endswith(("_tests.rs", "tests.rs")):
        return True
    return any(part == "tests" or part.endswith("_tests") for part in parts)


def is_allowed_backing_file(path: Path) -> bool:
    return path.name in BACKING_FILES


def rust_production_files() -> list[Path]:
    if not SRC_ROOT.is_dir():
        return []
    return sorted(
        path
        for path in SRC_ROOT.rglob("*.rs")
        if path.is_file()
        and not is_test_path(path)
        and not is_allowed_backing_file(path)
    )


def scan_file(path: Path) -> list[str]:
    findings: list[str] = []
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    for line_no, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        for code, pattern in FORBIDDEN_PATTERNS:
            if pattern.search(line):
                if code.startswith("direct_task_contract") and rel_path in BASELINE_TASK_CONTRACT_FILES:
                    continue
                findings.append(f"{rel_path}:{line_no}: {code}")
    return findings


def scan_repo() -> list[str]:
    findings: list[str] = []
    for path in rust_production_files():
        findings.extend(scan_file(path))
    return findings


def run_self_test() -> int:
    assert FORBIDDEN_PATTERNS[0][1].search("crate::contract_matrix::final_answer_shape_for_route")
    assert FORBIDDEN_PATTERNS[0][1].search("super::contract_matrix::ActionPolicyDecision")
    assert FORBIDDEN_PATTERNS[1][1].search("use crate::contract_matrix;")
    assert FORBIDDEN_PATTERNS[1][1].search("use crate::contract_matrix::FinalAnswerShape;")
    assert FORBIDDEN_PATTERNS[2][1].search("crate::task_contract::target_locators_for_route")
    assert FORBIDDEN_PATTERNS[3][1].search("use crate::task_contract;")
    assert not FORBIDDEN_PATTERNS[0][1].search("crate::evidence_policy::final_answer_shape_for_route")
    assert not FORBIDDEN_PATTERNS[2][1].search("crate::evidence_policy::target_locators_for_route")
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings = scan_repo()
    print(f"EVIDENCE_POLICY_FACADE_BOUNDARY_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
