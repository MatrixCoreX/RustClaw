#!/usr/bin/env python3
"""Reject mutable third-party GitHub Action references in workflow files."""

from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
USES = re.compile(r"^\s*(?:-\s*)?uses:\s*['\"]?([^\s'\"#]+)", re.MULTILINE)
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")


def main() -> int:
    findings: list[str] = []
    for path in sorted((*WORKFLOWS.glob("*.yml"), *WORKFLOWS.glob("*.yaml"))):
        text = path.read_text(encoding="utf-8")
        for match in USES.finditer(text):
            reference = match.group(1)
            if reference.startswith("./"):
                continue
            target, separator, revision = reference.rpartition("@")
            line = text.count("\n", 0, match.start()) + 1
            if not separator or not target or not FULL_SHA.fullmatch(revision):
                findings.append(f"{path.relative_to(ROOT)}:{line}:{reference}")
    if findings:
        print("GITHUB_ACTION_PIN_CHECK findings=" + str(len(findings)))
        for finding in findings:
            print(finding)
        return 1
    print("GITHUB_ACTION_PIN_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
