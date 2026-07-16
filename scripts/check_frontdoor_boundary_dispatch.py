#!/usr/bin/env python3
"""Guard the ask front door from regaining ordinary semantic routing.

The post-migration front door may materialize attachments, bind machine-owned
context, and construct a safety/budget envelope. Every ordinary ask must then
enter the same planner loop; direct answer, clarification, and capability
selection are not front-door decisions.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ASK_RUNTIME = ROOT / "crates" / "clawd" / "src" / "worker" / "ask_runtime.rs"
ASK_FRONTDOOR = (
    ROOT / "crates" / "clawd" / "src" / "worker" / "ask_planner_frontdoor.rs"
)
DELETED_FRONTDOOR_FILES = (
    ROOT / "crates" / "clawd" / "src" / "worker" / "ask_pipeline.rs",
    ROOT / "crates" / "clawd" / "src" / "runtime" / "ask_mode.rs",
)

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("chat_gate_dispatch", re.compile(r"\bis_chat_gate\s*\(")),
    ("clarify_gate_dispatch", re.compile(r"\bis_clarify_gate\s*\(")),
    ("clarify_only_dispatch", re.compile(r"\bis_clarify_only\s*\(")),
    ("respond_trace_dispatch", re.compile(r"\bis_respond_trace\s*\(")),
    ("first_layer_decision_dispatch", re.compile(r"\bFirstLayerDecision\b")),
    ("direct_answer_mode_dispatch", re.compile(r"\bdirect_answer\s*\(")),
    ("ask_mode_dispatch", re.compile(r"\bAskMode\b")),
    ("route_gate_dispatch", re.compile(r"\broute_gate_kind\b")),
    ("route_trace_dispatch", re.compile(r"\broute_trace_decision\b")),
    ("legacy_pipeline_call", re.compile(r"\bprepare_ask_pipeline\s*\(")),
)

REQUIRED_DISPATCH_TOKENS = (
    "build_agent_run_context_from_prepared_flow",
    "run_agent_with_tools",
    "agent_loop_default_entry",
)
REQUIRED_PREPARE_TOKENS = (
    "prepare_planner_owned_ask_routing",
    "prepare_ask_execution_context",
    "load_active_session_snapshot",
)
REQUIRED_FRONTDOOR_TOKENS = (
    "TurnBoundaryEnvelope::from_claimed_task",
    "explicit_machine_syntax_command_segment",
    "agent_loop_semantic_authority",
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


def scan_body(
    label: str,
    start_line: int,
    body: str,
    required_tokens: tuple[str, ...],
) -> list[str]:
    findings: list[str] = []
    for token in required_tokens:
        if token not in body:
            findings.append(f"{label}:{start_line}: missing_required_token:{token}")
    for offset, line in enumerate(body.splitlines()):
        if line.strip().startswith("//"):
            continue
        for code, pattern in FORBIDDEN_PATTERNS:
            if pattern.search(line):
                findings.append(f"{label}:{start_line + offset}: {code}")
    return findings


def scan_function(path: Path, name: str, required_tokens: tuple[str, ...]) -> list[str]:
    if not path.is_file():
        return [f"{rel(path)}:0: required_file_missing"]
    try:
        start_line, body = function_body(path, name)
    except RuntimeError as err:
        return [str(err)]
    return scan_body(rel(path), start_line, body, required_tokens)


def scan_deleted_files() -> list[str]:
    return [f"{rel(path)}:0: deleted_frontdoor_file_returned" for path in DELETED_FRONTDOOR_FILES if path.exists()]


def scan_repo() -> list[str]:
    return (
        scan_deleted_files()
        + scan_function(
            ASK_RUNTIME,
            "execute_ask_dispatch",
            REQUIRED_DISPATCH_TOKENS,
        )
        + scan_function(ASK_RUNTIME, "prepare_ask_flow", REQUIRED_PREPARE_TOKENS)
        + scan_function(
            ASK_FRONTDOOR,
            "prepare_planner_owned_ask_routing",
            REQUIRED_FRONTDOOR_TOKENS,
        )
    )


def run_self_test() -> int:
    valid = " ".join(REQUIRED_DISPATCH_TOKENS)
    assert scan_body("fixture.rs", 1, valid, REQUIRED_DISPATCH_TOKENS) == []
    missing = scan_body("fixture.rs", 1, "run_agent_with_tools", REQUIRED_DISPATCH_TOKENS)
    assert any("missing_required_token" in finding for finding in missing)
    forbidden = scan_body(
        "fixture.rs",
        1,
        f"{valid}\nask_mode.is_chat_gate();",
        REQUIRED_DISPATCH_TOKENS,
    )
    assert any("chat_gate_dispatch" in finding for finding in forbidden)
    assert any(path.name == "ask_pipeline.rs" for path in DELETED_FRONTDOOR_FILES)
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
