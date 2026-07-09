#!/usr/bin/env python3
"""Guard route_reason marker parsing behind the typed facade.

Production code should not grow ad hoc route_reason parsing. The only direct
parser allowed during this migration is the facade in pipeline_types.rs. The
finalizer helper is a temporary Track D compatibility exception because it
classifies rendered route evidence rather than choosing route authority.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "crates" / "clawd" / "src"

ALLOWED_DIRECT_PARSERS = {
    SRC / "pipeline_types.rs",
}

PATTERNS = (
    ("route_reason_contains", re.compile(r"\broute_reason\.contains\s*\(")),
    ("route_reason_split", re.compile(r"\broute_reason\.split\s*\(")),
    ("field_route_reason_contains", re.compile(r"\.route_reason\.contains\s*\(")),
    ("field_route_reason_split", re.compile(r"\.route_reason\.split\s*\(")),
)


def is_test_file(path: Path) -> bool:
    rel = path.relative_to(ROOT).as_posix()
    parts = path.relative_to(ROOT).parts
    return (
        path.name.endswith("_tests.rs")
        or rel.endswith("_test.rs")
        or any(part == "tests" or part.endswith("_tests") for part in parts)
    )


def main() -> int:
    findings: list[tuple[Path, int, str, str]] = []
    for path in SRC.rglob("*.rs"):
        if is_test_file(path) or path in ALLOWED_DIRECT_PARSERS:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), 1):
            for kind, pattern in PATTERNS:
                if pattern.search(line):
                    findings.append((path.relative_to(ROOT), lineno, kind, line.strip()))

    for rel, lineno, kind, line in findings:
        print(f"{rel}:{lineno}: {kind}: {line}")
    print(f"ROUTE_REASON_MARKER_FACADE_CHECK findings={len(findings)}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
