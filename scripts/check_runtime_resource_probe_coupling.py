#!/usr/bin/env python3
"""Keep OS resource probing behind claw-core's shared host snapshot."""

from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
RUNTIME_ROOT = Path("crates/clawd/src")
OWNER = Path("crates/claw-core/src/host_resources.rs")
PROBE_PATTERN = re.compile(
    r"/proc/(?:meminfo|pressure/memory)"
    r"|\bMem(?:Available|Total):"
    r"|\bmemory\.(?:current|max|high|events|pressure)\b"
    r"|\bhw\.memsize\b"
    r"|\bvm_stat\b"
)


def repository_files() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    return [Path(raw.decode()) for raw in result.stdout.split(b"\0") if raw]


def findings_for_text(relative: Path, text: str) -> list[str]:
    if relative == OWNER or not relative.is_relative_to(RUNTIME_ROOT):
        return []
    return [
        f"{relative.as_posix()}:{line_number}:{line.strip()}"
        for line_number, line in enumerate(text.splitlines(), 1)
        if PROBE_PATTERN.search(line)
    ]


def run_inventory() -> int:
    findings: list[str] = []
    for relative in repository_files():
        if relative.suffix != ".rs":
            continue
        path = ROOT / relative
        if not path.is_file():
            continue
        try:
            findings.extend(findings_for_text(relative, path.read_text(encoding="utf-8")))
        except (OSError, UnicodeDecodeError):
            continue

    print(
        "RUNTIME_RESOURCE_PROBE_INVENTORY "
        f"runtime_direct_probe_occurrences={len(findings)}"
    )
    if findings:
        print("RUNTIME_RESOURCE_PROBE_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print("RUNTIME_RESOURCE_PROBE_CHECK ok")
    return 0


def self_test() -> int:
    assert findings_for_text(
        Path("crates/clawd/src/example.rs"),
        'let source = "/proc/meminfo";',
    )
    assert findings_for_text(
        Path("crates/clawd/src/example.rs"),
        'Command::new("vm_stat");',
    )
    assert not findings_for_text(
        OWNER,
        'let source = "/proc/meminfo";',
    )
    assert not findings_for_text(
        Path("scripts/build.rs"),
        'let source = "/proc/meminfo";',
    )
    print("RUNTIME_RESOURCE_PROBE_CHECK self-test ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    return self_test() if args.self_test else run_inventory()


if __name__ == "__main__":
    raise SystemExit(main())
