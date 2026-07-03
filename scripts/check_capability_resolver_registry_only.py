#!/usr/bin/env python3
"""Guard capability resolution from regaining static compatibility fallback.

Ordinary capability selection should be planner -> registry-backed resolver ->
verifier/runtime. Unknown or unavailable capabilities must return structured
machine states, not fall through to a hardcoded resolver table.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCAN_FILES = (
    ROOT / "crates" / "clawd" / "src" / "capability_resolver.rs",
    ROOT / "crates" / "clawd" / "src" / "capability_resolver_tests.rs",
    ROOT / "crates" / "clawd" / "src" / "agent_engine" / "dispatch_support.rs",
)

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("static_capability_resolver_fn", re.compile(r"\bresolve_static_capability\b")),
    ("static_capability_resolver_table", re.compile(r"\bstatic_capabilit(?:y|ies)\b")),
    ("static_compat_reason_code", re.compile(r"capability_resolver_static")),
    ("legacy_unresolved_reason_code", re.compile(r"capability_resolver_unresolved")),
    ("static_compat_source", re.compile(r'"static_compat"')),
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def scan_file(path: Path) -> list[str]:
    findings: list[str] = []
    if not path.is_file():
        return findings
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        for code, pattern in FORBIDDEN_PATTERNS:
            if pattern.search(line):
                findings.append(f"{rel(path)}:{line_no}: {code}")
    return findings


def scan_repo() -> list[str]:
    findings: list[str] = []
    for path in SCAN_FILES:
        findings.extend(scan_file(path))
    return findings


def run_self_test() -> int:
    assert FORBIDDEN_PATTERNS[0][1].search("fn resolve_static_capability()")
    assert FORBIDDEN_PATTERNS[1][1].search("let static_capabilities = vec![];")
    assert FORBIDDEN_PATTERNS[2][1].search('"capability_resolver_static_compat_resolved"')
    assert FORBIDDEN_PATTERNS[3][1].search('"capability_resolver_unresolved"')
    assert FORBIDDEN_PATTERNS[4][1].search('"static_compat"')
    assert not FORBIDDEN_PATTERNS[3][1].search('"capability_unavailable"')
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings = scan_repo()
    print(f"CAPABILITY_RESOLVER_REGISTRY_ONLY_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
