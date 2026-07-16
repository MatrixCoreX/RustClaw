#!/usr/bin/env python3
"""Guard the planner-owned semantic frontdoor from legacy route reintroduction."""
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
    ("derived_route_decision", re.compile(r"\bderived_route_decision\b")),
    ("route_label_call", re.compile(r"\.route_label\s*\(")),
    ("intent_normalizer_decision_log", re.compile(r"\bintent_normalizer\b.*\bdecision=")),
    (
        "boundary_envelope_raw_text_copy",
        re.compile(r"raw_user_request\s*:\s*self\.raw_user_request\b"),
    ),
    (
        "boundary_envelope_raw_chars_string_token",
        re.compile(r"raw_user_request\s*:\s*format!\(\s*\"raw_chars:"),
    ),
)

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


def is_allowed(rel_path: str, kind: str, line_text: str) -> bool:
    if kind == "legacy_route_label":
        # New production traces must use route_trace_label. Historical artifact
        # readers live outside production Rust and are not scanned here.
        return False
    if kind == "derived_route_label":
        # Production code should use boundary_mode, route_trace_decision, or route_trace_label.
        return False
    if kind == "derived_route_decision":
        # Generic decision naming makes normalizer trace compatibility look
        # authoritative. Production code should spell this as route_trace_*.
        return False
    if kind == "route_label_call":
        # The old route_label() API was removed; route_trace_label_for_log()
        # is the only permitted production helper.
        return False
    if kind == "intent_normalizer_decision_log":
        # Normalizer may emit route_trace_decision, but not a generic
        # decision= log field that looks like current route authority.
        return False
    if kind in {
        "boundary_envelope_raw_text_copy",
        "boundary_envelope_raw_chars_string_token",
    }:
        # BoundaryEnvelope should carry machine request-length metadata, not the
        # raw natural-language request or a string-encoded raw_chars token before
        # the planner loop.
        return False
    if kind == "FirstLayerDecision":
        return False
    if kind == "first_layer_decision":
        return False
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


def scan_verifier_contract_boundary_text(rel_path: str, text: str) -> list[Finding]:
    if not Path(rel_path).name.startswith("verifier"):
        return []
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if re.search(r"\bRouteResult\b|\broute_result\b", line):
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "verifier_route_result_dependency",
                    line.strip(),
                )
            )
    return findings


def scan_planner_contract_boundary_text(rel_path: str, text: str) -> list[Finding]:
    name = Path(rel_path).name
    if not (name == "planning.rs" or name.startswith(("planning_", "planner_abort_"))):
        return []
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if re.search(r"\bRouteResult\b|\broute_result\b", line):
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "planner_route_result_dependency",
                    line.strip(),
                )
            )
    return findings


def scan_agent_loop_machine_guard_boundary_text(rel_path: str, text: str) -> list[Finding]:
    name = Path(rel_path).name
    if name not in {"agent_engine.rs", "support.rs", "execution_loop.rs"}:
        return []
    findings: list[Finding] = []
    guarded_symbols = (
        "answer_verifier_enforce_required",
        "answer_verifier_required_evidence_enabled",
        "registry_idempotency_guard_enabled",
        "action_fingerprint_for_policy",
        "registry_idempotency_guard_attribution",
        "check_repeat_action_guard",
    )
    for symbol in guarded_symbols:
        for match in re.finditer(
            rf"fn\s+{re.escape(symbol)}\b(?P<body>.*?)(?:\n\}}|\n\n)",
            text,
            flags=re.DOTALL,
        ):
            body = match.group("body")
            route_match = re.search(r"\bRouteResult\b|\broute_result\b", body)
            if route_match is None:
                continue
            offset = match.start("body") + route_match.start()
            findings.append(
                Finding(
                    rel_path,
                    line_number_for_offset(text, offset),
                    "agent_loop_machine_guard_route_dependency",
                    symbol,
                )
            )
    return findings


def scan_task_journal_direct_contract_boundary_text(rel_path: str, text: str) -> list[Finding]:
    journal_files = {
        "crates/clawd/src/task_journal.rs",
        "crates/clawd/src/task_journal/summary_trace.rs",
        "crates/clawd/src/task_journal_decision_envelope.rs",
        "crates/clawd/src/task_journal_evidence_coverage.rs",
        "crates/clawd/src/task_journal_goal.rs",
    }
    if rel_path not in journal_files:
        return []
    findings: list[Finding] = []
    pattern = re.compile(r"\bRouteResult\b|\broute_result\b|\broute_reason\b|for_route\b")
    for line_no, line in enumerate(text.splitlines(), start=1):
        if pattern.search(line):
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "task_journal_route_dependency",
                    line.strip(),
                )
            )
    return findings


def line_number_for_offset(text: str, offset: int) -> int:
    return text.count("\n", 0, max(offset, 0)) + 1


def scan_boundary_envelope_type_contract_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    match = re.search(
        r"struct\s+TurnBoundaryEnvelope\s*\{(?P<body>.*?)\n\}",
        text,
        flags=re.DOTALL,
    )
    if not match:
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_envelope_struct_missing",
                "TurnBoundaryEnvelope struct not found",
            )
        )
        return findings

    body = match.group("body")
    body_start = match.start("body")
    raw_offset = body.find("raw_user_request")
    if raw_offset >= 0:
        findings.append(
            Finding(
                rel_path,
                line_number_for_offset(text, body_start + raw_offset),
                "boundary_envelope_raw_user_request_field",
                "TurnBoundaryEnvelope must not carry raw_user_request",
            )
        )
    if not re.search(r"\braw_chars\s*:\s*usize\b", body):
        findings.append(
            Finding(
                rel_path,
                line_number_for_offset(text, match.start()),
                "boundary_envelope_raw_chars_missing",
                "TurnBoundaryEnvelope must expose raw_chars: usize",
            )
        )
    return findings


def scan_repo() -> list[Finding]:
    findings: list[Finding] = []
    for path in production_rust_files():
        rel_path = rel(path)
        if path.name == "intent_router.rs" or path.name.startswith("intent_router_"):
            findings.append(
                Finding(rel_path, 1, "legacy_intent_router_file", "legacy intent router file returned")
            )
        findings.extend(scan_text(rel_path, path.read_text(encoding="utf-8")))
        findings.extend(
            scan_verifier_contract_boundary_text(
                rel_path,
                path.read_text(encoding="utf-8"),
            )
        )
        findings.extend(
            scan_planner_contract_boundary_text(
                rel_path,
                path.read_text(encoding="utf-8"),
            )
        )
        findings.extend(
            scan_agent_loop_machine_guard_boundary_text(
                rel_path,
                path.read_text(encoding="utf-8"),
            )
        )
        findings.extend(
            scan_task_journal_direct_contract_boundary_text(
                rel_path,
                path.read_text(encoding="utf-8"),
            )
        )
    output_types = SOURCE_ROOT / "turn_boundary_envelope.rs"
    findings.extend(
        scan_boundary_envelope_type_contract_text(
            rel(output_types),
            output_types.read_text(encoding="utf-8"),
        )
    )
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
    assert scan_text(
        "crates/clawd/src/runtime/types.rs",
        "let x = FirstLayerDecision::PlannerExecute;",
    )
    assert scan_text(
        "crates/clawd/src/ask_flow.rs",
        "let label = route.derived_route_label();",
    )
    assert scan_text(
        "crates/clawd/src/intent_router_normalizer_run.rs",
        "let derived_route_decision = route_trace_decision_from_state(...);",
    )
    assert not scan_text(
        "crates/clawd/src/task_journal.rs",
        "let derived_route_trace_decision = route_trace_decision_from_state(...);",
    )
    assert scan_text(
        "crates/clawd/src/task_journal.rs",
        'json!({ "legacy_route_label": "Act" });',
    )
    assert scan_text(
        "crates/clawd/src/ask_flow.rs",
        "let label = route.route_label();",
    )
    assert scan_task_journal_direct_contract_boundary_text(
        "crates/clawd/src/task_journal.rs",
        "fn record_route_result(route: &RouteResult) {}",
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
        "crates/clawd/src/turn_boundary_envelope.rs",
        "raw_user_request: self.raw_user_request.clone(),",
    )
    assert scan_text(
        "crates/clawd/src/turn_boundary_envelope.rs",
        'raw_user_request: format!("raw_chars:{}", self.raw_user_request.chars().count()),',
    )
    assert not scan_text(
        "crates/clawd/src/turn_boundary_envelope.rs",
        "raw_chars: self.raw_user_request.chars().count(),",
    )
    assert scan_verifier_contract_boundary_text(
        "crates/clawd/src/verifier.rs",
        "fn verify(route_result: Option<&RouteResult>) {}",
    )
    assert not scan_verifier_contract_boundary_text(
        "crates/clawd/src/verifier.rs",
        "fn verify(output_contract: Option<&IntentOutputContract>) {}",
    )
    assert scan_planner_contract_boundary_text(
        "crates/clawd/src/agent_engine/planning.rs",
        "fn plan(route_result: Option<&RouteResult>) {}",
    )
    assert not scan_planner_contract_boundary_text(
        "crates/clawd/src/agent_engine/planning.rs",
        "fn plan(output_contract: Option<&IntentOutputContract>) {}",
    )
    assert scan_agent_loop_machine_guard_boundary_text(
        "crates/clawd/src/agent_engine/support.rs",
        "fn action_fingerprint_for_policy(route_result: Option<&RouteResult>) -> String {\n    String::new()\n}\n",
    )
    assert not scan_agent_loop_machine_guard_boundary_text(
        "crates/clawd/src/agent_engine/support.rs",
        "fn action_fingerprint_for_policy(action: &AgentAction) -> String {\n    action_fingerprint(action)\n}\n",
    )
    assert scan_boundary_envelope_type_contract_text(
        "crates/clawd/src/turn_boundary_envelope.rs",
        "pub(crate) struct TurnBoundaryEnvelope {\n    pub(crate) raw_user_request: String,\n}",
    )
    assert scan_boundary_envelope_type_contract_text(
        "crates/clawd/src/turn_boundary_envelope.rs",
        "pub(crate) struct TurnBoundaryEnvelope {\n    pub(crate) session_binding: Option<String>,\n}",
    )
    assert not scan_boundary_envelope_type_contract_text(
        "crates/clawd/src/turn_boundary_envelope.rs",
        "pub(crate) struct TurnBoundaryEnvelope {\n    pub(crate) raw_chars: usize,\n}",
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
