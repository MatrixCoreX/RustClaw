#!/usr/bin/env python3
"""Guard the ask front door from regaining ordinary semantic routing.

The front-door worker may submit pure schedule requests, resume an existing
discussion, or enter the agent loop. It must not decide ordinary direct answer
vs clarification vs execution before the planner.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ASK_PIPELINE = ROOT / "crates" / "clawd" / "src" / "worker" / "ask_pipeline.rs"
ASK_MODE = ROOT / "crates" / "clawd" / "src" / "runtime" / "ask_mode.rs"

FORBIDDEN_DISPATCH_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("chat_gate_dispatch", re.compile(r"\bis_chat_gate\s*\(")),
    ("clarify_gate_dispatch", re.compile(r"\bis_clarify_gate\s*\(")),
    ("clarify_only_dispatch", re.compile(r"\bis_clarify_only\s*\(")),
    ("respond_trace_dispatch", re.compile(r"\bis_respond_trace\s*\(")),
    ("first_layer_decision_dispatch", re.compile(r"\bFirstLayerDecision\b")),
    ("direct_answer_mode_dispatch", re.compile(r"\bdirect_answer\s*\(")),
    ("clarify_mode_dispatch", re.compile(r"\bAskMode::clarify\s*\(")),
    ("respond_trace_variant", re.compile(r"\bRespondTrace\b")),
    ("clarify_trace_variant", re.compile(r"\bClarifyTrace\b")),
)

REQUIRED_DISPATCH_TOKENS: tuple[str, ...] = (
    "should_route_schedule_direct",
    "agent_loop_default_context",
    "run_agent_with_tools",
)
REQUIRED_PREPARE_TOKENS: tuple[str, ...] = (
    "should_route_schedule_direct",
    "is_resume_discussion()",
    "resume_execution()",
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def function_body(path: Path, name: str) -> tuple[int, str]:
    text = read(path)
    start = text.find(f"fn {name}")
    if start < 0:
        start = text.find(f"async fn {name}")
    if start < 0:
        raise RuntimeError(f"{name} not found in {rel(path)}")
    open_brace = text.find("{", start)
    if open_brace < 0:
        raise RuntimeError(f"{name} body not found in {rel(path)}")
    depth = 0
    for index in range(open_brace, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                line_no = text[:start].count("\n") + 1
                return line_no, text[start : index + 1]
    raise RuntimeError(f"{name} body not closed in {rel(path)}")


def cfg_test_line_indices(lines: list[str]) -> set[int]:
    return {idx for idx, line in enumerate(lines) if line.strip() == "#[cfg(test)]"}


def has_cfg_test_near(lines: list[str], index: int) -> bool:
    cfg_lines = cfg_test_line_indices(lines)
    return any(candidate in cfg_lines for candidate in range(max(0, index - 8), index))


def scan_dispatch_body() -> list[str]:
    start_line, body = function_body(ASK_PIPELINE, "execute_ask_dispatch")
    findings: list[str] = []
    for token in REQUIRED_DISPATCH_TOKENS:
        if token not in body:
            findings.append(f"{rel(ASK_PIPELINE)}:{start_line}: missing_dispatch_token:{token}")
    for offset, line in enumerate(body.splitlines(), start=0):
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        for code, pattern in FORBIDDEN_DISPATCH_PATTERNS:
            if pattern.search(line):
                findings.append(f"{rel(ASK_PIPELINE)}:{start_line + offset}: {code}")
    return findings


def scan_prepare_body() -> list[str]:
    start_line, body = function_body(ASK_PIPELINE, "prepare_ask_flow")
    findings: list[str] = []
    for token in REQUIRED_PREPARE_TOKENS:
        if token not in body:
            findings.append(f"{rel(ASK_PIPELINE)}:{start_line}: missing_prepare_token:{token}")
    return findings


def scan_ask_mode_trace_variants() -> list[str]:
    lines = read(ASK_MODE).splitlines()
    findings: list[str] = []
    for index, line in enumerate(lines):
        if "RespondTrace" not in line and "ClarifyTrace" not in line:
            continue
        if has_cfg_test_near(lines, index):
            continue
        stripped = line.strip()
        if stripped.startswith("//!") or stripped.startswith("///"):
            continue
        findings.append(f"{rel(ASK_MODE)}:{index + 1}: trace_variant_not_cfg_test")
    return findings


def scan_repo() -> list[str]:
    return scan_dispatch_body() + scan_prepare_body() + scan_ask_mode_trace_variants()


def run_self_test() -> int:
    assert FORBIDDEN_DISPATCH_PATTERNS[0][1].search("ask_mode.is_chat_gate()")
    assert FORBIDDEN_DISPATCH_PATTERNS[3][1].search("ask_mode.is_respond_trace()")
    assert "run_agent_with_tools" in REQUIRED_DISPATCH_TOKENS
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings = scan_repo()
    print(f"FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
