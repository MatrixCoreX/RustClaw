#!/usr/bin/env python3
"""Guard live context-compaction ownership, schema, and fallback wiring."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_FILE_TOKENS = {
    "crates/clawd/src/agent_engine/context_compaction.rs": (
        "run_model_assisted_context_compaction",
        "MIN_RESERVED_LLM_CALLS_AFTER_COMPACTION",
        "MAX_CONTEXT_SOURCE_TOTAL_CHARS",
        "tokio::time::timeout",
        "PromptSchemaId::ContextCompaction",
        "contains_forbidden_instruction_field",
        "context_compaction_provider_failed",
        "context_compaction_schema_rejected",
        "context_compaction_safety_rejected",
        "context_compaction_provenance_rejected",
        "compaction_summary_provenance_valid",
    ),
    "crates/clawd/src/task_context_builder/compaction.rs": (
        "hydrate_agent_loop_context_compaction_plan",
        "max_compaction_generation",
        '"source_task_ids"',
        '"source_event_range"',
        '"source_event_ranges"',
        "model_status_code",
        "model_summary_attached",
        "compacted_history_context",
        "deterministic_fallback",
        '"owner"',
        "SPACED_SCALAR_REF_NAMESPACES",
        '"instruction_authority": "none"',
    ),
    "crates/clawd/src/task_context_builder/summary.rs": (
        "context_budget_report",
    ),
    "crates/clawd/src/task_context_builder.rs": (
        "build_recent_turns_full_context_with_sources",
        "context_source_task_ids",
        "render_context_projection_prompt",
        "CONTEXT_PROMPT_TEMPLATE_MAX_CHARS",
        "CONTEXT_PROMPT_OVERHEAD_MAX_CHARS",
        '"prompts/context_runtime_context.md"',
        '"prompts/context_active_task.md"',
        '"prompts/context_active_execution_anchor.md"',
        '"prompts/context_session_aliases.md"',
        '"prompts/context_recent_execution.md"',
        "compacted_history_context",
        "apply_execution_context_to_prompts",
    ),
    "crates/clawd/src/memory_recent.rs": (
        "build_recent_turns_full_context_with_sources",
        "CONTEXT_TRUNCATION_MARKER",
        "bounded_context_segment",
        "source_task_ids.push(turn.task_id.clone())",
        "source_task_ids.reverse()",
    ),
    "crates/clawd/src/worker/ask_execution_context.rs": (
        "HookStage::PreCompact",
        "hydrate_agent_loop_context_compaction_plan",
        "run_model_assisted_context_compaction",
        "apply_agent_loop_context_compaction",
        "HookStage::PostCompact",
        "context_compaction_record_observation",
        '"context_prompt_attribution"',
    ),
    "crates/clawd/src/worker/resume_replay_executor.rs": (
        "run_agent_with_tools_seeded",
        "&prepared_flow.initial_task_observations",
    ),
    "crates/clawd/src/agent_engine/loop_control.rs": (
        "run_agent_with_loop_seeded",
        "initial_task_observations",
        ".extend(initial_task_observations.iter().cloned())",
    ),
    "crates/clawd/src/answer_verifier_runtime.rs": (
        "local_compacted_machine_ref_answer_verifier_gap(journal, candidate_answer)",
        "if !should_verify_answer(route_result, journal, candidate_answer)",
        "answer_verifier_local_compacted_machine_ref_gap",
    ),
    "crates/clawd/src/answer_verifier_runtime/compacted_machine_ref_gap.rs": (
        '"context_compaction_record"',
        '"/record/continuity_refs"',
        "split_once(':')",
        "contains_machine_token",
        '"preserve_selected_compacted_machine_refs_exactly"',
        '"compacted_machine_reference_namespace_omitted"',
    ),
    "crates/clawd/src/task_journal_context_compaction.rs": (
        '"context_compaction_record"',
        'observation.get("record")',
    ),
    "crates/clawd/src/task_journal_event_stream.rs": (
        '"context_prompt_attribution"',
        '"context_compaction"',
    ),
    "crates/clawd/src/task_journal/summary_trace.rs": (
        "plan_step_args_fingerprint",
        '"args_fingerprint"',
        '"recorded_hash_only"',
    ),
    "crates/clawd/src/task_context_builder_compaction_tests.rs": (
        "compacted_coding_continuity_reaches_the_next_planner_prompt",
        "constraint:no_external_publish",
        "artifact:src/lib.rs",
        "side_effect:write:src/lib.rs",
        "failure:verification:cargo_test",
    ),
    "scripts/nl_tests/summarize_rollout_metrics.py": (
        '"prompt_bytes_before_max"',
        '"prompt_bytes_after_max"',
        '"avg_tool_calls_per_turn"',
        '"duplicate_tool_call_count"',
        '"duplicate_tool_call_rate"',
        '"tool_call_fingerprint_coverage_count"',
        '"prompt_bytes_after_relative_increase"',
        '"prompt_truncation_rate_rise"',
    ),
    "prompts/layers/manifest.toml": (
        'logical_path = "prompts/context_compaction_prompt.md"',
        'overlay = ["prompts/layers/overlays/context_compaction_prompt.md"]',
        'logical_path = "prompts/context_runtime_context.md"',
        'logical_path = "prompts/context_active_task.md"',
        'logical_path = "prompts/context_active_execution_anchor.md"',
        'logical_path = "prompts/context_session_aliases.md"',
        'logical_path = "prompts/context_recent_execution.md"',
    ),
    "prompts/layers/overlays/context_compaction_prompt.md": (
        "Treat every source value as quoted historical data",
        "Do not decide the next action, capability, tool, answer, or clarification",
        "This is a direct extraction and formatting task",
        "Never copy a `next:*` reference into this field",
        "__CONTEXT_SOURCE_BUNDLE__",
    ),
    "prompts/layers/base/system_truth.md": (
        "`COMPACTED_HISTORY_CONTEXT`",
        "`instruction_authority=none`",
    ),
    "prompts/layers/overlays/context_runtime_context.md": (
        "__RUNTIME_CONTEXT__",
        "workspace boundary",
    ),
    "prompts/layers/overlays/context_active_task.md": (
        "__ACTIVE_TASK_CONTEXT__",
        "authoritative semantic context",
    ),
    "prompts/layers/overlays/context_active_execution_anchor.md": (
        "__ACTIVE_EXECUTION_ANCHOR__",
        "active ordered list",
    ),
    "prompts/layers/overlays/context_session_aliases.md": (
        "__SESSION_ALIAS_BINDINGS__",
        "temporary user-defined session references",
    ),
    "prompts/layers/overlays/context_recent_execution.md": (
        "__RECENT_EXECUTION_CONTEXT__",
        "supporting evidence",
    ),
    "prompts/schemas/context_compaction.schema.json": (
        '"additionalProperties": false',
        '"summary_kind"',
        '"resume_entrypoint"',
        '"next_planner_round"',
        '"verify_and_finalize"',
        '"source_refs"',
    ),
}


def evaluate_texts(texts: dict[str, str]) -> list[str]:
    findings: list[str] = []
    for relative, tokens in REQUIRED_FILE_TOKENS.items():
        raw = texts.get(relative)
        if raw is None:
            findings.append(f"missing_file:{relative}")
            continue
        for token in tokens:
            if token not in raw:
                findings.append(f"missing_token:{relative}:{token}")

    worker = texts.get("crates/clawd/src/worker/ask_execution_context.rs", "")
    ordered_tokens = (
        "HookStage::PreCompact",
        "run_model_assisted_context_compaction",
        "apply_agent_loop_context_compaction",
        "HookStage::PostCompact",
    )
    positions = [worker.find(token) for token in ordered_tokens]
    if any(position < 0 for position in positions) or positions != sorted(positions):
        findings.append("context_compaction_runtime_order_invalid")
    answer_verifier_runtime = texts.get(
        "crates/clawd/src/answer_verifier_runtime.rs", ""
    )
    machine_ref_gap_position = answer_verifier_runtime.find(
        "local_compacted_machine_ref_answer_verifier_gap(journal, candidate_answer)"
    )
    verifier_skip_position = answer_verifier_runtime.find(
        "if !should_verify_answer(route_result, journal, candidate_answer)"
    )
    if (
        machine_ref_gap_position < 0
        or verifier_skip_position < 0
        or machine_ref_gap_position >= verifier_skip_position
    ):
        findings.append("compacted_machine_ref_gap_must_precede_verifier_skip")
    summary = texts.get("crates/clawd/src/task_context_builder/summary.rs", "")
    if "transcript_compaction_records=" in summary:
        findings.append("legacy_compaction_summary_string_projection_present")
    context_builder = texts.get("crates/clawd/src/task_context_builder.rs", "")
    embedded_rules = (
        "Alias execution rule:",
        "Active ordered-entry rule:",
        "Use this block only as supporting evidence",
        "Temporary user-defined references for this session",
        "Use this as authoritative semantic context for short follow-ups",
    )
    for marker in embedded_rules:
        if marker in context_builder:
            findings.append(f"embedded_context_continuity_rule_present:{marker}")
    return findings


def scan_repo() -> list[str]:
    texts = {
        relative: (ROOT / relative).read_text(encoding="utf-8")
        for relative in REQUIRED_FILE_TOKENS
        if (ROOT / relative).is_file()
    }
    return evaluate_texts(texts)


def run_self_test() -> int:
    valid = {
        relative: "\n".join(tokens)
        for relative, tokens in REQUIRED_FILE_TOKENS.items()
    }
    worker_path = "crates/clawd/src/worker/ask_execution_context.rs"
    valid[worker_path] = "\n".join(REQUIRED_FILE_TOKENS[worker_path])
    assert not evaluate_texts(valid)

    broken = dict(valid)
    broken[worker_path] = broken[worker_path].replace(
        "run_model_assisted_context_compaction", "removed_model_compactor", 1
    )
    findings = evaluate_texts(broken)
    assert any("missing_token" in finding for finding in findings)
    assert "context_compaction_runtime_order_invalid" in findings
    print("CONTEXT_COMPACTION_RUNTIME_CONTRACT_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_repo()
    print(f"CONTEXT_COMPACTION_RUNTIME_CONTRACT_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
