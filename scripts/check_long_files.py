#!/usr/bin/env python3
"""Guard RustClaw against growing oversized Rust source files.

This check treats oversized Rust files as an explicit violation. Historical
long-file debt has been split down; future exemptions must be current,
intentional, and documented in BASELINE_LONG_FILES with a clear reason in the
change that adds them.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


PRODUCTION_THRESHOLD = 2_000
TEST_THRESHOLD = 2_000

# Current explicit exemptions. Keep this empty unless a short-lived exception is
# justified by a concrete follow-up split plan.
BASELINE_LONG_FILES: dict[str, int] = {}

SKIP_DIRS = {".git", "target", "node_modules", "UI/dist"}


def rel_path(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def is_skipped(path: Path, root: Path) -> bool:
    rel = path.relative_to(root)
    parts = set(rel.parts)
    if parts & {".git", "target", "node_modules"}:
        return True
    return rel.as_posix().startswith("UI/dist/")


def is_test_file(path: Path) -> bool:
    name = path.name
    return name.endswith("_tests.rs") or name == "tests.rs" or "tests" in path.parts


def count_lines(path: Path) -> int:
    with path.open("rb") as handle:
        return sum(1 for _ in handle)


def scan(root: Path) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    violations: list[dict[str, object]] = []
    debt: list[dict[str, object]] = []
    rust_roots = (root / "crates", root / "optional_skills")
    for path in sorted(path for rust_root in rust_roots for path in rust_root.rglob("*.rs")):
        if is_skipped(path, root):
            continue
        rel = rel_path(path, root)
        lines = count_lines(path)
        threshold = TEST_THRESHOLD if is_test_file(path) else PRODUCTION_THRESHOLD
        if lines <= threshold:
            continue
        baseline = BASELINE_LONG_FILES.get(rel)
        record = {
            "path": rel,
            "lines": lines,
            "threshold": threshold,
            "baseline": baseline,
        }
        if baseline is None:
            record["reason"] = "new_over_threshold_file"
            violations.append(record)
        elif lines > baseline:
            record["reason"] = "baseline_file_grew"
            violations.append(record)
        else:
            debt.append(record)
    return violations, debt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--json", action="store_true", help="print machine-readable JSON")
    args = parser.parse_args()

    root = Path(args.root).resolve()
    violations, debt = scan(root)
    if args.json:
        print(json.dumps({"violations": violations, "baseline_debt": debt}, indent=2))
    else:
        if violations:
            print("LONG_FILE_CHECK failed")
            for item in violations:
                print(
                    f"- {item['path']}: {item['lines']} lines "
                    f"(threshold {item['threshold']}, baseline {item['baseline']}) "
                    f"reason={item['reason']}"
                )
        else:
            print(f"LONG_FILE_CHECK ok baseline_debt_files={len(debt)}")
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
