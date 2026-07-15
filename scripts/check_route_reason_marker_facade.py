#!/usr/bin/env python3
"""Guard route_reason marker parsing behind the typed facade.

Production code should not grow ad hoc route_reason parsing. The only direct
parser allowed during this migration is the facade in pipeline_types.rs. The
finalizer helper is a temporary Track D compatibility exception because it
classifies rendered route evidence rather than choosing route authority.
"""

from __future__ import annotations

import argparse
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


def scan_text(path: Path, text: str) -> list[tuple[Path, int, str, str]]:
    findings: list[tuple[Path, int, str, str]] = []
    for lineno, line in enumerate(text.splitlines(), 1):
        for kind, pattern in PATTERNS:
            if pattern.search(line):
                findings.append((path.relative_to(ROOT), lineno, kind, line.strip()))
    return findings


def scan_repo() -> list[tuple[Path, int, str, str]]:
    findings: list[tuple[Path, int, str, str]] = []
    for path in SRC.rglob("*.rs"):
        if is_test_file(path) or path in ALLOWED_DIRECT_PARSERS:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        findings.extend(scan_text(path, text))
    return findings


def run_self_test() -> int:
    bad = """
fn bad(route_reason: &str, route: &RouteResult) {
    let _ = route_reason.contains("execution");
    let _ = route.route_reason.split(';');
}
"""
    bad_findings = scan_text(SRC / "worker/example.rs", bad)
    assert any(kind == "route_reason_contains" for _, _, kind, _ in bad_findings)
    assert any(kind == "field_route_reason_split" for _, _, kind, _ in bad_findings)
    good = """
fn good(marker: RouteReasonMarker) {
    let _ = marker.is_execution();
}
"""
    assert not scan_text(SRC / "worker/example.rs", good)
    print("ROUTE_REASON_MARKER_FACADE_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings = scan_repo()

    for rel, lineno, kind, line in findings:
        print(f"{rel}:{lineno}: {kind}: {line}")
    print(f"ROUTE_REASON_MARKER_FACADE_CHECK findings={len(findings)}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
