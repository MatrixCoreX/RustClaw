#!/usr/bin/env python3
"""Guard policy decision JSON fields against hardcoded token strings."""

from __future__ import annotations

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


def main() -> int:
    findings: list[str] = []
    scanned = 0
    for path in rust_files():
        if path == POLICY_DECISION_FILE or is_test_path(path):
            continue
        scanned += 1
        raw = path.read_text(encoding="utf-8")
        for match in DECISION_LITERAL_RE.finditer(raw):
            line = raw.count("\n", 0, match.start()) + 1
            token = match.group(1)
            findings.append(f"{path.relative_to(ROOT)}:{line}: hardcoded_decision={token}")
    if findings:
        print(f"POLICY_DECISION_TOKEN_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print(f"POLICY_DECISION_TOKEN_CHECK ok scanned_files={scanned}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
