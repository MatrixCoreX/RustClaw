#!/usr/bin/env python3
"""Guard policy decision JSON fields against hardcoded token strings."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates" / "clawd" / "src"
POLICY_DECISION_FILE = SRC_ROOT / "policy_decision.rs"

DECISION_LITERAL_RE = re.compile(
    r'"decision"\s*:\s*"(allow|deny|require_confirmation|background_wait)"'
)


def is_test_path(path: Path) -> bool:
    parts = path.relative_to(SRC_ROOT).parts
    return (
        any(part in {"tests", "test"} or part.endswith("_tests") for part in parts)
        or path.name.endswith("_tests.rs")
        or "_tests" in path.name
    )


def rust_files() -> list[Path]:
    return sorted(SRC_ROOT.rglob("*.rs"))


def scan_text(relative: str, raw: str) -> list[str]:
    findings: list[str] = []
    for match in DECISION_LITERAL_RE.finditer(raw):
        line = raw.count("\n", 0, match.start()) + 1
        token = match.group(1)
        findings.append(f"{relative}:{line}: hardcoded_decision={token}")
    return findings


def run_self_test() -> int:
    hardcoded = 'let payload = json!({"decision": "allow"});'
    enum_token = (
        'let payload = json!({"decision": '
        'crate::policy_decision::PolicyDecision::Allow.as_token()});'
    )
    if not scan_text("crates/clawd/src/verifier.rs", hardcoded):
        print("SELF_TEST_FAIL missing_hardcoded_decision_match", file=sys.stderr)
        return 1
    if scan_text("crates/clawd/src/verifier.rs", enum_token):
        print("SELF_TEST_FAIL policy_decision_enum_false_positive", file=sys.stderr)
        return 1
    print("POLICY_DECISION_TOKEN_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings: list[str] = []
    scanned = 0
    for path in rust_files():
        if path == POLICY_DECISION_FILE or is_test_path(path):
            continue
        scanned += 1
        raw = path.read_text(encoding="utf-8")
        findings.extend(scan_text(path.relative_to(ROOT).as_posix(), raw))
    if findings:
        print(f"POLICY_DECISION_TOKEN_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print(f"POLICY_DECISION_TOKEN_CHECK ok scanned_files={scanned}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
