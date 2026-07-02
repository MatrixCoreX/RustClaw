#!/usr/bin/env python3
"""Guard the planner from regaining pre-LLM deterministic feature routers.

The Codex/Claude-style target is that ordinary respond/clarify/act/capability
choice happens inside the agent loop. Deterministic plan-result helpers may
remain as test fixtures or isolated safety/boundary utilities, but
`plan_round_actions()` must not short-circuit to one before the planner LLM has
seen the task.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
AGENT_ENGINE_ROOT = ROOT / "crates" / "clawd" / "src" / "agent_engine"
PLANNING_RS = AGENT_ENGINE_ROOT / "planning.rs"

DETERMINISTIC_PLAN_FN = re.compile(r"\bfn\s+\w+_deterministic_plan_result\s*\(")
DETERMINISTIC_PLAN_CALL = re.compile(r"\b\w+_deterministic_plan_result\s*\(")
PRE_LLM_FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("deterministic_plan_result_call", DETERMINISTIC_PLAN_CALL),
    ("direct_plan_result_builder", re.compile(r"\bbuild_plan_result\s*\(")),
    ("deterministic_plan_marker", re.compile(r'"deterministic:')),
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def planning_prefix_before_llm() -> tuple[int, str]:
    text = read(PLANNING_RS)
    start = text.find("pub(super) async fn plan_round_actions")
    if start < 0:
        raise RuntimeError("plan_round_actions not found")
    llm = text.find("llm_gateway::run_with_fallback_with_prompt_source", start)
    if llm < 0:
        raise RuntimeError("planner LLM gateway call not found")
    line_no = text[:start].count("\n") + 1
    return line_no, text[start:llm]


def test_only_module_paths() -> set[Path]:
    lines = read(PLANNING_RS).splitlines()
    result: set[Path] = set()
    pending_cfg_test = False
    pending_path: str | None = None
    path_re = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')
    mod_re = re.compile(r"\bmod\s+\w+\s*;")
    for line in lines:
        stripped = line.strip()
        if stripped == "#[cfg(test)]":
            pending_cfg_test = True
            continue
        path_match = path_re.search(stripped)
        if path_match:
            pending_path = path_match.group(1)
            continue
        if mod_re.search(stripped):
            if pending_cfg_test and pending_path:
                result.add((AGENT_ENGINE_ROOT / pending_path).resolve())
            pending_cfg_test = False
            pending_path = None
            continue
        if stripped and not stripped.startswith("#["):
            pending_cfg_test = False
            pending_path = None
    return result


def has_cfg_test_near(lines: list[str], index: int) -> bool:
    window = lines[max(0, index - 4) : index]
    return any(line.strip() == "#[cfg(test)]" for line in window)


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    if rel_path.endswith(("_tests.rs", "tests.rs")):
        return True
    return any(part == "tests" or part.endswith("_tests") for part in parts)


def scan_pre_llm_prefix() -> list[str]:
    start_line, prefix = planning_prefix_before_llm()
    findings: list[str] = []
    for offset, line in enumerate(prefix.splitlines(), start=0):
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        for code, pattern in PRE_LLM_FORBIDDEN_PATTERNS:
            if pattern.search(line):
                findings.append(f"{rel(PLANNING_RS)}:{start_line + offset}: {code}")
    return findings


def scan_deterministic_helper_visibility() -> list[str]:
    test_only_paths = test_only_module_paths()
    findings: list[str] = []
    for path in sorted(AGENT_ENGINE_ROOT.rglob("*.rs")):
        if not path.is_file() or is_test_path(path):
            continue
        lines = read(path).splitlines()
        module_is_test_only = path.resolve() in test_only_paths
        for index, line in enumerate(lines):
            if not DETERMINISTIC_PLAN_FN.search(line):
                continue
            if module_is_test_only or has_cfg_test_near(lines, index):
                continue
            findings.append(
                f"{rel(path)}:{index + 1}: deterministic_plan_helper_not_test_only"
            )
    return findings


def scan_repo() -> list[str]:
    return scan_pre_llm_prefix() + scan_deterministic_helper_visibility()


def run_self_test() -> int:
    assert DETERMINISTIC_PLAN_CALL.search("service_status_deterministic_plan_result(")
    assert PRE_LLM_FORBIDDEN_PATTERNS[1][1].search("build_plan_result(")
    assert PRE_LLM_FORBIDDEN_PATTERNS[2][1].search('"deterministic:service_status"')
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings = scan_repo()
    print(f"PLANNER_PRE_LLM_DETERMINISTIC_FAST_PATH_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
