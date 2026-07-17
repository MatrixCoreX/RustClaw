#!/usr/bin/env python3
"""Validate task event, context budget, and subagent team contracts.

This guard keeps Codex/Claude-style execution observability release-gated:
task journal events, context budget/compaction records, provider-call metrics,
coding evidence, and read-only subagent team aggregation must remain
machine-readable protocol rather than prose/log-only behavior.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/task_journal_event_stream.rs": (
        "fn task_event_json",
        '"event_type"',
        '"owner_layer": "task_journal"',
        "task_event_stream_json",
        '"task_lifecycle"',
        '"task_goal"',
        "append_context_budget_events",
        '"context_budget"',
        '"context_compaction"',
        '"context_prompt_attribution"',
        '"checkpoint_created"',
        '"task_transition"',
        '"agent_round"',
        "append_provider_call_events",
        '"provider_call"',
        '"tool_started"',
        '"tool_step"',
        '"tool_finished"',
        '"agent_hook"',
        '"subagent"',
        "append_subagent_team_lifecycle_events",
        "subagent_team_event_type",
        '"agent_team_started"',
        '"subagent_started"',
        '"subagent_finished"',
        '"subagent_failed"',
        '"agent_team_aggregated"',
        '"agent_team_conflict_detected"',
        '"coding_checkpoint"',
        '"coding_task_contract"',
        '"coding_evidence"',
        '"task_final"',
        '"step_ref"',
        '"evidence_ref"',
        '"evidence_refs"',
        '"artifact_ref_count"',
        '"verification_command_count"',
        '"verification_status"',
        '"verification_failure_kinds"',
        '"unverified_risk"',
        '"prompt_truncation_count"',
        '"prompt_bytes_before_max"',
        '"prompt_bytes_budget_min"',
        '"prompt_bytes_after_max"',
        '"prompt_truncated_bytes_total"',
    ),
    "crates/clawd/src/task_journal_context_budget.rs": (
        "context_budget_report_json",
        "context_budget_report=",
        ".filter(Value::is_object)",
    ),
    "crates/clawd/src/task_journal_context_compaction.rs": (
        "transcript_compaction_records_json",
        '"context_compaction_record"',
        'observation.get("record")',
    ),
    "crates/clawd/src/task_journal_goal.rs": (
        "task_goal_summary_json",
        '"schema_version"',
        '"render_owner"',
        '"finalizer_or_ui_i18n"',
        '"goal_status"',
        '"goal_status_source"',
        '"current_progress"',
        '"remaining_work"',
        '"missing_evidence"',
        "canonical_goal_status_token",
        '"waiting_user"',
        '"background"',
        '"completed"',
    ),
    "crates/clawd/src/agent_engine/subagent_runtime.rs": (
        "SubagentRuntimeConfig",
        "allowed_roles",
        "max_parallel_readonly",
        "write_enabled",
        "external_publish_enabled",
        "role_allowed",
        "record_subagent_action_with_config",
        '"owner_layer": "subagent_runtime"',
        '"execution_mode": "inline_readonly_child_run"',
        '"subagent_role_not_allowed"',
        '"subagent_role_disabled_by_config"',
        '"runtime_config"',
        '"timeout_policy"',
        '"cancellation_policy"',
        '"merge_contract"',
        '"failure_isolated"',
    ),
    "crates/clawd/src/agent_engine/subagent_runtime_batch.rs": (
        "record_subagent_batch_action_with_config",
        "team_lifecycle_event",
        '"agent_team_started"',
        '"subagent_started"',
        '"subagent_finished"',
        '"subagent_failed"',
        '"agent_team_aggregated"',
        '"agent_team_conflict_detected"',
        '"bounded_parallel_readonly_child_runs"',
        '"write_permission": "read_only"',
        '"write_enabled": false',
        '"external_publish_enabled": false',
        '"merge_child_machine_findings"',
        '"finding_refs"',
        '"evidence_refs"',
        '"conflict_summary"',
        '"recommended_next_action"',
        "SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED",
    ),
    "crates/clawd/src/task_journal_tests/event_stream_hooks.rs": (
        "trace_json_includes_pollable_machine_event_stream",
        "trace_json_projects_goal_and_context_budget_events",
        "trace_json_expands_subagent_team_lifecycle_events",
        "trace_json_projects_checkpoint_as_machine_event",
        "trace_json_projects_subagent_observations_as_subagent_events",
        '"task_goal"',
        '"context_budget"',
        '"context_compaction"',
        '"context_prompt_attribution"',
        '"subagent"',
        '"agent_team_started"',
        '"subagent_finished"',
        '"agent_team_aggregated"',
        '"provider_call"',
        "prompt_truncation_count",
    ),
    "README.md": (
        "task_goal",
        "context_budget",
        "context_compaction",
        "agent_team_started",
        "subagent_finished",
        "agent_team_aggregated",
        "prompt_truncation_count",
        "prompt_bytes_before_max",
        "clawcli replay export/run/diff",
        "Teaching mode",
    ),
    "README.zh-CN.md": (
        "task_goal",
        "context_budget",
        "context_compaction",
        "agent_team_started",
        "subagent_finished",
        "agent_team_aggregated",
        "prompt_truncation_count",
        "prompt_bytes_before_max",
        "clawcli replay export/run/diff",
        "教学模式",
    ),
}


def read_repo_texts() -> dict[str, str | None]:
    texts: dict[str, str | None] = {}
    for rel_path in REQUIRED_TOKENS_BY_PATH:
        try:
            texts[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            texts[rel_path] = None
    return texts


def scan_texts(texts: dict[str, str | None]) -> list[str]:
    findings: list[str] = []
    for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token not in text:
                findings.append(f"missing_token:{rel_path}:{token}")

    event_stream = texts.get("crates/clawd/src/task_journal_event_stream.rs") or ""
    required_event_types = (
        '"task_goal"',
        '"context_budget"',
        '"context_compaction"',
        '"context_prompt_attribution"',
        '"provider_call"',
        '"coding_checkpoint"',
        '"coding_task_contract"',
        '"coding_evidence"',
        '"agent_team_conflict_detected"',
    )
    for token in required_event_types:
        if event_stream.count(token) < 1:
            findings.append(f"event_stream_missing_event_type:{token}")
    if event_stream.count("task_event_json(") < 10:
        findings.append("event_stream_machine_event_projection_too_weak")

    subagent_text = "\n".join(
        texts.get(path) or ""
        for path in (
            "crates/clawd/src/agent_engine/subagent_runtime.rs",
            "crates/clawd/src/agent_engine/subagent_runtime_batch.rs",
        )
    )
    for token in (
        '"write_enabled": false',
        '"external_publish_enabled": false',
        '"runtime_config"',
        '"team_lifecycle_events"',
        '"child_results"',
        '"finding_refs"',
        '"evidence_refs"',
    ):
        if token not in subagent_text:
            findings.append(f"subagent_machine_boundary_missing:{token}")
    if "write_permission" in subagent_text and '"read_only"' not in subagent_text:
        findings.append("subagent_write_permission_not_read_only")

    tests = texts.get("crates/clawd/src/task_journal_tests/event_stream_hooks.rs") or ""
    for test_name in (
        "trace_json_projects_goal_and_context_budget_events",
        "trace_json_expands_subagent_team_lifecycle_events",
        "trace_json_projects_subagent_observations_as_subagent_events",
    ):
        if test_name not in tests:
            findings.append(f"missing_rust_event_stream_test:{test_name}")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    texts["crates/clawd/src/task_journal_event_stream.rs"] += "\n" + "\n".join(
        ["task_event_json(" for _ in range(12)]
    )
    texts["crates/clawd/src/agent_engine/subagent_runtime_batch.rs"] += "\n" + "\n".join(
        [
            '"team_lifecycle_events"',
            '"child_results"',
            '"write_permission"',
            '"read_only"',
        ]
    )
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    good_findings = scan_texts(good)
    assert not good_findings, good_findings

    missing_context = dict(good)
    missing_context["crates/clawd/src/task_journal_event_stream.rs"] = '"task_goal"'
    findings = scan_texts(missing_context)
    assert any("context_budget" in item for item in findings), findings

    missing_subagent_boundary = dict(good)
    missing_subagent_boundary["crates/clawd/src/agent_engine/subagent_runtime.rs"] = (
        "SubagentRuntimeConfig"
    )
    missing_subagent_boundary["crates/clawd/src/agent_engine/subagent_runtime_batch.rs"] = (
        '"team_lifecycle_events"'
    )
    findings = scan_texts(missing_subagent_boundary)
    assert any("subagent_machine_boundary_missing" in item for item in findings), findings

    missing_tests = dict(good)
    missing_tests["crates/clawd/src/task_journal_tests/event_stream_hooks.rs"] = '"task_goal"'
    findings = scan_texts(missing_tests)
    assert any("missing_rust_event_stream_test" in item for item in findings), findings

    print("TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
