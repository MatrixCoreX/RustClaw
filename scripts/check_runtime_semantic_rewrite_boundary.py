#!/usr/bin/env python3
"""Guard runtime semantic rewrites stay inside explicit migration inventory.

RustClaw's target is that ordinary semantic decisions live in the planner /
agent loop. Runtime may still expose legacy semantic migration debt as machine
telemetry, but new production control flow must not introduce semantic rewrite
sources outside the tracked inventory.
"""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("semantic_rewrite", re.compile(r"\bsemantic_rewrite\b")),
    ("legacy_migration_debt", re.compile(r"\blegacy_migration_debt\b")),
)

ALLOWED_PRODUCTION_FILES = {
    "crates/clawd/src/ask_flow_pre_planner_exit.rs",
}


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


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
        path
        for path in SRC_ROOT.rglob("*.rs")
        if path.is_file() and not is_test_path(path)
    )


def finding_allowed(rel_path: str) -> bool:
    return rel_path in ALLOWED_PRODUCTION_FILES


def scan_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in FORBIDDEN_PATTERNS:
            if not pattern.search(line):
                continue
            if finding_allowed(rel_path):
                continue
            findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_repo() -> list[Finding]:
    findings: list[Finding] = []
    for path in production_rust_files():
        findings.extend(scan_text(rel(path), path.read_text(encoding="utf-8")))
    return findings


def print_report(findings: list[Finding]) -> int:
    print(f"RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings={len(findings)}")
    for item in findings:
        print(f"  - {item.path}:{item.line} [{item.kind}] {item.text}")
    return 1 if findings else 0


def run_self_test() -> int:
    allowed = scan_text(
        "crates/clawd/src/ask_flow_pre_planner_exit.rs",
        '"decision_source": "semantic_rewrite",\n',
    )
    assert not allowed
    blocked = scan_text(
        "crates/clawd/src/agent_engine/planning.rs",
        '"decision_source": "semantic_rewrite",\n',
    )
    assert blocked and blocked[0].kind == "semantic_rewrite"
    blocked_debt = scan_text(
        "crates/clawd/src/finalize/task.rs",
        '"semantic_control_state": "legacy_migration_debt",\n',
    )
    assert blocked_debt and blocked_debt[0].kind == "legacy_migration_debt"
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
