#!/usr/bin/env python3
"""Implementation for agent-loop trace release/deletion gates."""
from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from types import ModuleType
from typing import Any


ALLOWED_RUNTIME_DECISION_SOURCES = {
    "not_recorded",
    "contract_boundary",
    "safety_policy",
    "permission_policy",
    "evidence_projection",
    "lifecycle_projection",
    "recovery_boundary",
    "compat_trace",
}

ALLOWED_SEMANTIC_CONTROL_STATES = {
    "not_recorded",
    "none",
}

def load_summarizer() -> ModuleType:
    path = Path(__file__).with_name("agent_loop_trace_replay_summary_impl.py")
    spec = importlib.util.spec_from_file_location("agent_loop_trace_replay_summary_impl", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load agent-loop trace replay summarizer: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def counter_total(counter: dict[str, Any], *keys: str) -> int:
    return sum(int_value(counter.get(key)) for key in keys)


def counter_value_total(counter: dict[str, Any]) -> int:
    return sum(int_value(value) for value in counter.values())


def unexpected_keys(counter: dict[str, Any], allowed: set[str]) -> list[str]:
    return sorted(
        key
        for key, value in counter.items()
        if int_value(value) > 0 and str(key) not in allowed
    )


def add_counter_findings(
    findings: list[str],
    summary: dict[str, Any],
    field: str,
    allowed: set[str],
) -> None:
    counter = dict_value(summary.get(field))
    bad_keys = unexpected_keys(counter, allowed)
    if bad_keys:
        findings.append(f"{field}: unexpected_keys={','.join(bad_keys)}")


def evaluate_summary(summary: dict[str, Any], allow_semantic_debt: bool) -> list[str]:
    findings: list[str] = []
    if int_value(summary.get("parse_errors")):
        findings.append(f"parse_errors={summary.get('parse_errors')}")
    route_delta_items = int_value(summary.get("route_delta_items"))
    round_envelope_items = counter_value_total(
        dict_value(summary.get("round_decision_envelope_source_counts"))
    )
    if route_delta_items <= 0 and round_envelope_items <= 0:
        findings.append("route_observation_items=0")
    if int_value(summary.get("unexplained_mismatch_count")):
        findings.append(
            f"unexplained_mismatch_count={summary.get('unexplained_mismatch_count')}"
        )

    runtime_sources = dict_value(summary.get("runtime_decision_source_counts"))
    semantic_rewrite_count = counter_total(runtime_sources, "semantic_rewrite")
    if semantic_rewrite_count and not allow_semantic_debt:
        findings.append(f"runtime_decision_source_counts.semantic_rewrite={semantic_rewrite_count}")
    add_counter_findings(
        findings,
        summary,
        "runtime_decision_source_counts",
        ALLOWED_RUNTIME_DECISION_SOURCES
        | ({"semantic_rewrite"} if allow_semantic_debt else set()),
    )

    control_states = dict_value(summary.get("runtime_semantic_control_state_counts"))
    legacy_debt_count = counter_total(control_states, "legacy_migration_debt")
    if legacy_debt_count and not allow_semantic_debt:
        findings.append(
            f"runtime_semantic_control_state_counts.legacy_migration_debt={legacy_debt_count}"
        )
    add_counter_findings(
        findings,
        summary,
        "runtime_semantic_control_state_counts",
        ALLOWED_SEMANTIC_CONTROL_STATES
        | ({"legacy_migration_debt"} if allow_semantic_debt else set()),
    )

    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("run_dirs", nargs="+", type=Path)
    parser.add_argument("--max-examples", type=int, default=3)
    parser.add_argument(
        "--allow-semantic-debt",
        action="store_true",
        help="Permit known migration debt counters while still checking parse errors and unexplained mismatches.",
    )
    parser.add_argument(
        "--print-summary",
        action="store_true",
        help="Print the computed agent-loop trace summary before gate findings.",
    )
    args = parser.parse_args()

    for run_dir in args.run_dirs:
        if not run_dir.is_dir():
            raise SystemExit(f"run dir not found: {run_dir}")

    summarizer = load_summarizer()
    summary = summarizer.summarize_run_dirs(
        args.run_dirs,
        max(args.max_examples, 0),
        dedupe_latest_case=False,
    )
    if args.print_summary:
        print(json.dumps(summary, ensure_ascii=False, sort_keys=True, indent=2))

    findings = evaluate_summary(summary, args.allow_semantic_debt)
    if findings:
        print("AGENT_LOOP_TRACE_RELEASE_GATE failed")
        for finding in findings:
            print(f"- {finding}")
        return 1

    print(
        "AGENT_LOOP_TRACE_RELEASE_GATE ok "
        f"route_delta_items={int_value(summary.get('route_delta_items'))} "
        "round_decision_envelope_items="
        f"{counter_value_total(dict_value(summary.get('round_decision_envelope_source_counts')))} "
        f"unexplained_mismatch_count={int_value(summary.get('unexplained_mismatch_count'))}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
