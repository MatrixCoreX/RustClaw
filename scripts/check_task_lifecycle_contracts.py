#!/usr/bin/env python3
"""Validate task lifecycle / checkpoint / resume contracts.

This is a static guard for Codex/Claude-style durable execution. It checks that
long-running work is represented by machine-readable lifecycle, checkpoint, and
resume-executor fields, and that recovery paths reject user-visible prose fields
as protocol.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/task_lifecycle.rs": (
        "enum TaskLifecycleState",
        "Waiting",
        "Background",
        "NeedsUser",
        "enum ResumeEntrypoint",
        "NextPlannerRound",
        "PollAsyncJob",
        "AwaitUserInput",
        "VerifyAndFinalize",
        "enum CheckpointResumeDirective",
        "WaitForActiveLease",
        "task_query_lifecycle_projection",
        '"can_poll"',
        '"can_cancel"',
        '"last_heartbeat_ts"',
        '"resume_due"',
        '"resume_wait_seconds"',
        '"resume_executor_claim"',
        '"checkpoint_id"',
        '"poll_ref"',
        '"cancel_ref"',
        "struct TaskCheckpoint",
        "completed_side_effect_refs",
        "pending_async_job",
        "repair_signal",
        "struct AsyncJobRef",
        "missing_required_fields",
    ),
    "crates/clawd/src/repo/task_resume_execution.rs": (
        "PlannedPausedCheckpointResumeExecution",
        "HandoffPausedCheckpointResumeExecution",
        "ClaimedHandoffPausedCheckpointResumeExecution",
        "list_planned_paused_checkpoint_resume_executions_internal",
        "WHERE status = 'running'",
        "result_json IS NOT NULL",
        "planned_paused_checkpoint_resume_execution_from_result_json",
        "record_planned_paused_checkpoint_resume_handoff_internal",
        "resume_executor_handoff",
        "resume_executor_claim",
        "lease_expires_at",
        "resume_directive",
        "task_checkpoint_from_result_json",
    ),
    "crates/clawd/src/repo/task_resume_execution/dispatch_claim.rs": (
        "claim_dispatched_paused_checkpoint_resume_execution_internal",
        "record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal",
        "resume_executor_dispatch_claim",
        "resume_executor_dispatch_result",
        "resume_executor_handoff_dispatch",
        "resume_executor_dispatch_execution_state",
        "lease_expires_at",
        "task_checkpoint_from_result_json",
    ),
    "crates/clawd/src/worker/resume_replay_executor.rs": (
        "execute_seeded_agent_loop_dispatch_result",
        "claimed_seeded_agent_loop_dispatch_ready",
        "run_agent_with_tools_seeded",
        "ResumeEntrypoint::NextPlannerRound",
        "completed_side_effect_refs",
        'claimed.execution_plan.get("text").is_none()',
        'claimed.execution_plan.get("error_text").is_none()',
        'claimed.dispatch_payload.get("text").is_none()',
        'claimed.dispatch_payload.get("error_text").is_none()',
        'claimed.dispatch_claim.get("text").is_none()',
        'claimed.dispatch_claim.get("error_text").is_none()',
    ),
    "crates/clawd/src/agent_engine/loop_control.rs": (
        "loop_state_has_checkpoint_handoff",
        'matches!(lifecycle_state, "waiting" | "background" | "needs_user")',
        "if loop_state_has_checkpoint_handoff(&pre_finalize_loop_state)",
        "return Ok(reply);",
    ),
    "crates/clawd/src/finalize/task.rs": (
        "journal_has_checkpointed_nonterminal_lifecycle",
        "update_task_checkpointed_result",
        "answer.resume_context.is_none()",
        "finalize_ask_checkpointed",
    ),
    "crates/clawd/src/repo/tasks.rs": (
        "paused_lifecycle_owned_by_other_executor",
        "update_task_checkpointed_result",
        "lease_owner = NULL",
        "lease_expires_at = 0",
        "AND lease_expires_at <= ?1",
        "if task_lease_expires_at > now_ts",
        "AND lease_expires_at <= ?3",
        "merge_progress_with_active_resume_coordination",
        "task_progress_cas_exhausted",
    ),
    "crates/clawd/src/repo/task_resume_execution/resume_lease.rs": (
        "merge_progress_with_active_resume_coordination",
        "resume_execution_progress",
        "renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal",
        "active_claim_chain_matches",
        "resume_executor_dispatch_claim",
        "lease_expires_at > ?3",
    ),
    "crates/clawd/src/worker/runtime_support/resume_execution_lease.rs": (
        "run_with_renewable_resume_execution_lease",
        "RenewableResumeExecution::LeaseLost",
        "lease_seconds / 3",
        "renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal",
    ),
    "crates/clawd/src/repo/task_resume_execution/result_projection.rs": (
        "deferred_seeded_loop_checkpoint_result",
        "rescheduled_checkpoint",
        "previous_checkpoint_id",
        "lease_owner = NULL",
        "lease_expires_at = 0",
    ),
    "crates/clawd/src/worker/runtime_support/dispatch_result.rs": (
        "journal_has_matching_nonterminal_checkpoint",
        "seeded_loop_deferred",
        "deferred_checkpoint_id",
        "deferred_lifecycle_state",
    ),
    "crates/clawd/src/repo/tasks_tests.rs": (
        "due_checkpoint_waits_for_frontend_worker_lease_and_claim_rechecks_it",
        "foreground_heartbeat_cannot_reclaim_a_published_checkpoint",
    ),
    "crates/clawd/src/agent_engine/loop_control_tests/soft_budget_checkpoint.rs": (
        "checkpoint_handoff_requires_matching_nonterminal_machine_state",
    ),
    "crates/clawd/src/finalize/task_tests/checkpoint_finalization.rs": (
        "checkpointed_ask_finalization_overrides_failure_metric",
        "lease_owner.is_none()",
        "assert_eq!(lease_expires_at, 0)",
    ),
    "crates/clawd/src/repo/task_resume_execution_tests/resume_lease.rs": (
        "active_resume_dispatch_lease_renews_the_complete_claim_chain",
        "resumed_agent_progress_cannot_erase_dispatch_coordination",
        "deferred_seeded_loop_projects_the_new_checkpoint_and_releases_its_lease",
    ),
    "crates/clawd/src/worker/runtime_support/dispatch_result_tests.rs": (
        "seeded_agent_loop_with_new_checkpoint_is_deferred_not_terminal",
    ),
    "crates/clawd/src/worker/runtime_support.rs": (
        "build_paused_checkpoint_resume_work_item",
        "plan_claimed_paused_checkpoint_resume_execution",
        "recover_stale_running_tasks_on_startup",
        "maybe_recover_stale_running_tasks_runtime",
        "sync_recovery_can_claim_dispatch_executor",
    ),
    "docs/task_lifecycle_lease_model.md": (
        "task-row worker leases",
        "checkpoint resume-executor leases",
        "must not parse user-visible `text` or `error_text`",
        "`task_lifecycle.state`",
        "`waiting`",
        "`background`",
        "`needs_user`",
        "`resume_executor_claim.owner`",
        "`resume_entrypoint = \"poll_async_job\"`",
        "`clawcli resume-task <task_id>`",
        "`cargo test -p clawd task_lifecycle -- --quiet`",
        "`cargo test -p clawd task_resume_execution -- --quiet`",
        "`cargo test -p clawd async_poll_executor -- --quiet`",
    ),
    "README.md": (
        "worker_once recovery tick",
        "task_lifecycle",
        "checkpoint_id",
        "resume-task",
        "poll_async_job",
        "docs/task_lifecycle_lease_model.md",
    ),
    "README.zh-CN.md": (
        "worker_once 恢复 tick",
        "task_lifecycle",
        "checkpoint_id",
        "resume-task",
        "poll_async_job",
        "docs/task_lifecycle_lease_model.md",
    ),
}

RESUME_PROTOCOL_FILES = (
    "crates/clawd/src/repo/task_resume_execution.rs",
    "crates/clawd/src/repo/task_resume_execution/dispatch_claim.rs",
    "crates/clawd/src/worker/resume_replay_executor.rs",
)


def read_repo_texts() -> dict[str, str | None]:
    out: dict[str, str | None] = {}
    for rel_path in REQUIRED_TOKENS_BY_PATH:
        path = ROOT / rel_path
        try:
            out[rel_path] = path.read_text(encoding="utf-8")
        except FileNotFoundError:
            out[rel_path] = None
        except UnicodeDecodeError:
            out[rel_path] = None
    for rel_path in RESUME_PROTOCOL_FILES:
        if rel_path in out:
            continue
        try:
            out[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            out[rel_path] = None
    return out


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

    combined_resume = "\n".join(
        texts.get(rel_path) or "" for rel_path in RESUME_PROTOCOL_FILES
    )
    text_rejection_count = combined_resume.count('get("text").is_some()') + combined_resume.count(
        'get("text").is_none()'
    )
    error_text_rejection_count = combined_resume.count(
        'get("error_text").is_some()'
    ) + combined_resume.count('get("error_text").is_none()')
    if text_rejection_count < 8:
        findings.append("resume_protocol_text_field_rejection_too_weak")
    if error_text_rejection_count < 8:
        findings.append("resume_protocol_error_text_field_rejection_too_weak")
    if "run_agent_with_tools_seeded" not in combined_resume:
        findings.append("seeded_agent_loop_resume_missing")
    if "resume_executor_claim" not in combined_resume:
        findings.append("resume_executor_claim_boundary_missing")
    if "task_checkpoint_from_result_json" not in combined_resume:
        findings.append("task_checkpoint_restore_boundary_missing")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    resume_protocol = "\n".join(
        [
            'payload.get("text").is_some()',
            'payload.get("error_text").is_some()',
            'payload.get("text").is_some()',
            'payload.get("error_text").is_some()',
            'payload.get("text").is_some()',
            'payload.get("error_text").is_some()',
            'payload.get("text").is_none()',
            'payload.get("error_text").is_none()',
            'payload.get("text").is_none()',
            'payload.get("error_text").is_none()',
            'payload.get("text").is_none()',
            'payload.get("error_text").is_none()',
            'payload.get("text").is_none()',
            'payload.get("error_text").is_none()',
            'payload.get("text").is_none()',
            'payload.get("error_text").is_none()',
            "run_agent_with_tools_seeded",
            "resume_executor_claim",
            "task_checkpoint_from_result_json",
        ]
    )
    for rel_path in RESUME_PROTOCOL_FILES:
        texts[rel_path] = (texts.get(rel_path) or "") + "\n" + resume_protocol
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    good_findings = scan_texts(good)
    assert not good_findings, good_findings

    missing_lifecycle = dict(good)
    missing_lifecycle["crates/clawd/src/task_lifecycle.rs"] = "enum TaskLifecycleState"
    findings = scan_texts(missing_lifecycle)
    assert any("missing_token:crates/clawd/src/task_lifecycle.rs:Waiting" in item for item in findings)

    weak_resume = dict(good)
    for rel_path in RESUME_PROTOCOL_FILES:
        weak_resume[rel_path] = "resume_executor_claim\ntask_checkpoint_from_result_json"
    findings = scan_texts(weak_resume)
    assert "resume_protocol_text_field_rejection_too_weak" in findings
    assert "resume_protocol_error_text_field_rejection_too_weak" in findings
    assert "seeded_agent_loop_resume_missing" in findings

    missing_doc = dict(good)
    missing_doc["docs/task_lifecycle_lease_model.md"] = "`task_lifecycle.state`"
    findings = scan_texts(missing_doc)
    assert any("docs/task_lifecycle_lease_model.md" in item for item in findings)

    print("TASK_LIFECYCLE_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"TASK_LIFECYCLE_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("TASK_LIFECYCLE_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
