#!/usr/bin/env python3
"""Guard legacy first-layer route state stays inside compatibility boundaries.

The Codex/Claude-style target is that ordinary semantic decisions live in the
agent loop. Legacy normalizer route tokens may still exist while release gates
are open, but they must not spread back into agent-loop control code.
"""
from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOT = REPO_ROOT / "crates" / "clawd" / "src"

LEGACY_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("FirstLayerDecision", re.compile(r"\bFirstLayerDecision\b")),
    ("first_layer_decision", re.compile(r"\bfirst_layer_decision\b")),
    ("legacy_route_label", re.compile(r"\blegacy_route_label\b")),
    ("derived_route_label", re.compile(r"\bderived_route_label\b")),
    ("route_label_call", re.compile(r"\.route_label\s*\(")),
    ("intent_normalizer_decision_log", re.compile(r"\bintent_normalizer\b.*\bdecision=")),
    (
        "boundary_envelope_raw_text_copy",
        re.compile(r"raw_user_request\s*:\s*self\.raw_user_request\b"),
    ),
)

ALLOWED_FIRST_LAYER_TYPE_FILES = {
    "crates/clawd/src/main.rs",
    "crates/clawd/src/pipeline_types.rs",
    "crates/clawd/src/runtime/ask_mode.rs",
    "crates/clawd/src/runtime/mod.rs",
    "crates/clawd/src/runtime/types.rs",
    "crates/clawd/src/task_journal.rs",
    "crates/clawd/src/task_journal_decision_envelope.rs",
}

ALLOWED_FIRST_LAYER_TOKEN_FILES = {
    *ALLOWED_FIRST_LAYER_TYPE_FILES,
    "crates/clawd/src/worker/ask_prepare.rs",
}


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def rel(path: Path) -> str:
    return path.resolve().relative_to(REPO_ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    if rel_path.endswith(("_tests.rs", "tests.rs")):
        return True
    return any(part == "tests" or part.endswith("_tests") for part in parts)


def production_rust_files() -> list[Path]:
    return sorted(
        path
        for path in SOURCE_ROOT.rglob("*.rs")
        if path.is_file() and not is_test_path(path)
    )


def is_intent_router_compat_file(rel_path: str) -> bool:
    name = Path(rel_path).name
    return name == "intent_router.rs" or name.startswith("intent_router_")


def is_allowed(rel_path: str, kind: str, line_text: str) -> bool:
    if kind == "legacy_route_label":
        # New production traces must use route_trace_label. Historical artifact
        # readers live outside production Rust and are not scanned here.
        return False
    if kind == "derived_route_label":
        # Production code should use boundary_mode, route_trace_decision, or route_trace_label.
        return False
    if kind == "route_label_call":
        # The old route_label() API was removed; route_trace_label_for_log()
        # is the only permitted production helper.
        return False
    if kind == "intent_normalizer_decision_log":
        # Normalizer may emit route_trace_decision, but not a generic
        # decision= log field that looks like current route authority.
        return False
    if kind == "boundary_envelope_raw_text_copy":
        # BoundaryEnvelope should carry machine request-length metadata, not the
        # raw natural-language request before the planner loop.
        return False
    if kind == "FirstLayerDecision":
        return rel_path in ALLOWED_FIRST_LAYER_TYPE_FILES or is_intent_router_compat_file(rel_path)
    if kind == "first_layer_decision":
        if rel_path in ALLOWED_FIRST_LAYER_TOKEN_FILES or is_intent_router_compat_file(rel_path):
            return True
        # Allow exact JSON/log compatibility fields only where they are emitted
        # through trace helpers.
        return "legacy_first_layer_decision" in line_text
    return False


def scan_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in LEGACY_PATTERNS:
            if not pattern.search(line):
                continue
            if is_allowed(rel_path, kind, line):
                continue
            findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_repo() -> list[Finding]:
    findings: list[Finding] = []
    for path in production_rust_files():
        findings.extend(scan_text(rel(path), path.read_text(encoding="utf-8")))
    return findings


def print_report(findings: list[Finding]) -> int:
    print(f"LEGACY_ROUTE_BOUNDARY_CHECK findings={len(findings)}")
    for item in findings:
        print(f"  - {item.path}:{item.line} [{item.kind}] {item.text}")
    return 1 if findings else 0


def run_self_test() -> int:
    assert scan_text(
        "crates/clawd/src/agent_engine/planning.rs",
        "let x = FirstLayerDecision::PlannerExecute;",
    )
    assert not scan_text(
        "crates/clawd/src/intent_router_route_output.rs",
        "let x = FirstLayerDecision::PlannerExecute;",
    )
    assert scan_text(
        "crates/clawd/src/ask_flow.rs",
        "let label = route.derived_route_label();",
    )
    assert scan_text(
        "crates/clawd/src/task_journal.rs",
        'json!({ "legacy_route_label": "Act" });',
    )
    assert scan_text(
        "crates/clawd/src/ask_flow.rs",
        "let label = route.route_label();",
    )
    assert scan_text(
        "crates/clawd/src/intent_router_normalizer_run.rs",
        '"{} intent_normalizer task_id={} decision={:?}"',
    )
    assert not scan_text(
        "crates/clawd/src/intent_router_normalizer_run.rs",
        '"{} intent_normalizer task_id={} route_trace_decision={:?}"',
    )
    assert scan_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "raw_user_request: self.raw_user_request.clone(),",
    )
    assert not scan_text(
        "crates/clawd/src/intent_router_output_types.rs",
        'raw_user_request: format!("raw_chars:{}", self.raw_user_request.chars().count()),',
    )
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    return print_report(scan_repo())


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
