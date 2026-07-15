#!/usr/bin/env python3
"""Guard against reintroducing the retired `agent_mode` payload switch.

RustClaw's ordinary ask path now defaults to the agent loop. Channel/UI clients
should submit the user's text and machine context, not a legacy boolean that
implies ordinary semantic routing can be disabled before the planner.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCAN_ROOTS = (
    "README.md",
    "configs",
    "crates",
    "UI/src",
)
SKIP_PATH_PARTS = {
    "target",
    "node_modules",
    "dist",
    "__pycache__",
}
SKIP_FILES = {
    Path("scripts/check_no_agent_mode_payload.py"),
}
SCAN_SUFFIXES = {
    ".rs",
    ".ts",
    ".tsx",
    ".toml",
    ".md",
}
FORBIDDEN_PATTERN = re.compile(r"(?<![A-Za-z0-9_])agent_mode(?![A-Za-z0-9_])")


def iter_files() -> list[Path]:
    files: list[Path] = []
    for root_name in SCAN_ROOTS:
        root = ROOT / root_name
        if root.is_file():
            files.append(root)
            continue
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if not path.is_file() or path.suffix not in SCAN_SUFFIXES:
                continue
            rel = path.relative_to(ROOT)
            if rel in SKIP_FILES:
                continue
            if any(part in SKIP_PATH_PARTS for part in rel.parts):
                continue
            files.append(path)
    return sorted(files)


def scan_file(path: Path) -> list[str]:
    rel = path.relative_to(ROOT)
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return []
    findings: list[str] = []
    for lineno, line in enumerate(text.splitlines(), start=1):
        if FORBIDDEN_PATTERN.search(line):
            findings.append(f"{rel}:{lineno}: {line.strip()}")
    return findings


def run_self_test() -> int:
    assert FORBIDDEN_PATTERN.search('"agent_mode": true')
    assert not FORBIDDEN_PATTERN.search('"subagent_model_child"')
    assert Path("scripts/check_no_agent_mode_payload.py") in SKIP_FILES
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        return run_self_test()

    findings: list[str] = []
    for path in iter_files():
        findings.extend(scan_file(path))
    if findings:
        for finding in findings:
            print(f"[agent-mode-payload] {finding}", file=sys.stderr)
        return 1
    print("NO_AGENT_MODE_PAYLOAD ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
