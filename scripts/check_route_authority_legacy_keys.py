#!/usr/bin/env python3
"""Guard deprecated agent_decides_* keys from returning to runtime config.

The current route authority control is the machine token
`semantic_route_authority`. The older `agent_decides_semantic_route` and
`agent_decides_migration_class` names may appear in docs, tests, historical log
summaries, or comments only.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LEGACY_KEYS = ("agent_decides_semantic_route", "agent_decides_migration_class")
RUST_ROOTS = (ROOT / "crates" / "clawd" / "src", ROOT / "crates" / "claw-core" / "src")
CONFIG_ROOTS = (ROOT / "configs", ROOT / "docker" / "config")


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    if rel_path.endswith(("_tests.rs", "tests.rs")):
        return True
    return any(part == "tests" or part.endswith("_tests") for part in parts)


def rust_files() -> list[Path]:
    files: list[Path] = []
    for root in RUST_ROOTS:
        if root.is_dir():
            files.extend(
                path for path in root.rglob("*.rs") if path.is_file() and not is_test_path(path)
            )
    return sorted(files)


def config_files() -> list[Path]:
    files: list[Path] = []
    for root in CONFIG_ROOTS:
        if root.is_dir():
            files.extend(path for path in root.rglob("*.toml") if path.is_file())
    return sorted(files)


def line_has_legacy_key(line: str) -> bool:
    return any(key in line for key in LEGACY_KEYS)


def scan_rust() -> list[str]:
    findings: list[str] = []
    for path in rust_files():
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            if line_has_legacy_key(line):
                findings.append(f"{rel(path)}:{line_no}: legacy_key_in_production_rust")
    return findings


def scan_config() -> list[str]:
    findings: list[str] = []
    saw_current_authority = False
    for path in config_files():
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            stripped = line.strip()
            if stripped.startswith("#"):
                continue
            if "semantic_route_authority" in stripped:
                saw_current_authority = True
            if line_has_legacy_key(stripped):
                findings.append(f"{rel(path)}:{line_no}: legacy_key_in_config_body")
    if not saw_current_authority:
        findings.append("configs: missing semantic_route_authority config body")
    return findings


def scan_repo() -> list[str]:
    return scan_rust() + scan_config()


def run_self_test() -> int:
    assert line_has_legacy_key("agent_decides_semantic_route = true")
    assert line_has_legacy_key('let key = "agent_decides_migration_class";')
    assert not line_has_legacy_key('semantic_route_authority = "agent_loop_default"')
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_repo()
    print(f"ROUTE_AUTHORITY_LEGACY_KEY_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
