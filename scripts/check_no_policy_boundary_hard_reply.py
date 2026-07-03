#!/usr/bin/env python3
"""Guard runtime/finalizer policy boundaries against prose reply rules."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates" / "clawd" / "src"

HARD_REPLY_RE = re.compile(
    r'"[^"\n]*\b('
    r"Do not|Don't|Mention that|Explain that|Explain the|Give one|Ask for|"
    r"Tell the user|before retrying|Do not claim|Do not expose|Do not invent"
    r')\b[^"\n]*"'
)

PROMPT_OR_TEST_ALLOWLIST = {
    SRC_ROOT / "agent_engine" / "observed_output_route_policy.rs",
    SRC_ROOT / "agent_engine" / "support.rs",
    SRC_ROOT / "capability_map.rs",
    SRC_ROOT / "finalize" / "task.rs",
    SRC_ROOT / "finalize" / "task_resume.rs",
    SRC_ROOT / "http" / "ui_routes" / "skill_import_config.rs",
    SRC_ROOT / "intent_router_prompt_render.rs",
    SRC_ROOT / "routing_context.rs",
    SRC_ROOT / "skills" / "builtin.rs",
    SRC_ROOT / "task_context_builder.rs",
    SRC_ROOT / "worker" / "ask_prepare.rs",
}


def is_test_path(path: Path) -> bool:
    rel = path.relative_to(SRC_ROOT)
    return (
        any(part in {"tests", "test"} or part.endswith("_tests") for part in rel.parts)
        or path.name.endswith("_tests.rs")
        or "_tests" in path.name
    )


def rust_files() -> list[Path]:
    return sorted(SRC_ROOT.rglob("*.rs"))


def main() -> int:
    findings: list[str] = []
    scanned = 0
    for path in rust_files():
        if path in PROMPT_OR_TEST_ALLOWLIST or is_test_path(path):
            continue
        scanned += 1
        raw = path.read_text(encoding="utf-8")
        for match in HARD_REPLY_RE.finditer(raw):
            line = raw.count("\n", 0, match.start()) + 1
            snippet = match.group(0)
            if "prompt" in path.name or "_prompt" in path.name:
                continue
            findings.append(f"{path.relative_to(ROOT)}:{line}: hard_reply_rule={snippet}")
    if findings:
        print(f"POLICY_BOUNDARY_HARD_REPLY_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print(f"POLICY_BOUNDARY_HARD_REPLY_CHECK ok scanned_files={scanned}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
