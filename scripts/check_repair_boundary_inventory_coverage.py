#!/usr/bin/env python3
"""Ensure production repair modules are covered by REPAIR_BOUNDARY_INVENTORY."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "crates" / "clawd" / "src"
INVENTORY = SRC / "repair_boundary_inventory.rs"

SOURCE_FILE_RE = re.compile(r'"(crates/clawd/src/[^"]+)"')

EXEMPT_FILES = {
    SRC / "repair_boundary_inventory.rs",
}


def is_test_file(path: Path) -> bool:
    rel = path.relative_to(ROOT).as_posix()
    parts = path.relative_to(ROOT).parts
    return (
        path.name.endswith("_tests.rs")
        or rel.endswith("_test.rs")
        or any(part == "tests" or part.endswith("_tests") for part in parts)
    )


def inventory_sources() -> set[str]:
    text = INVENTORY.read_text(encoding="utf-8")
    return set(SOURCE_FILE_RE.findall(text))


def required_repair_sources() -> list[str]:
    required: list[str] = []
    for path in SRC.rglob("*.rs"):
        if path in EXEMPT_FILES or is_test_file(path):
            continue
        if "repair" not in path.name:
            continue
        required.append(path.relative_to(ROOT).as_posix())
    return sorted(required)


def main() -> int:
    covered = inventory_sources()
    required = required_repair_sources()
    missing = [path for path in required if path not in covered]

    for path in missing:
        print(f"{path}: missing from REPAIR_BOUNDARY_INVENTORY source_files")
    print(
        "REPAIR_BOUNDARY_INVENTORY_COVERAGE_CHECK "
        f"required={len(required)} missing={len(missing)}"
    )
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
