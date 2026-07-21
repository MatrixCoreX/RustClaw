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
        "initialize_task_budget_slice",
        "profile_for_verified_plan",
        "observe_task_budget",
        '"task_budget_slice_exhausted"',
        '"budget_decision"',
        "publish_claimed_event",
    ),
    "crates/clawd/src/finalize/task.rs": (
        "journal_has_checkpointed_nonterminal_lifecycle",
        "update_task_checkpointed_result",
        "answer.resume_context.as_ref()",
        'obj.insert("resume_context".to_string(), resume_context.clone())',
        "finalize_ask_checkpointed",
    ),
    "crates/clawd/src/finalize/loop_reply_missing_delivery.rs": (
        "pending_confirmation_resume_payload",
        'pointer("/approval_request/status")',
        "publish_agent_loop_user_input_checkpoint_progress",
        '"confirmation_required"',
    ),
    "crates/clawd/src/repo/task_approval.rs": (
        "task_has_pending_approval_request",
        'status != "running"',
        "CheckpointResumeDirective::AwaitUserInput",
        "ResumeEntrypoint::NextPlannerRound",
        '"approval_grant_resume"',
        "WHERE task_id = ?1 AND status = 'running'",
        "SET status = 'failed'",
        '"confirmation_timeout"',
    ),
    "crates/clawd/src/worker/run_skill_finalize.rs": (
        "finalize_run_skill_confirmation_required",
        "TaskLifecycleState::NeedsUser",
        "ResumeEntrypoint::AwaitUserInput",
        '"approval_checkpoint_needs_user"',
        "update_task_checkpointed_result",
    ),
    "crates/clawd/src/repo/tasks.rs": (
        "is_task_claim_active",
        "worker_task_write_rejection",
        "WORKER_LEASE_LOST_STATUS_CODE",
        "automatic_checkpoint_resume_allowed",
        "ResumeEntrypoint::AwaitUserInput",
        "paused_lifecycle_owned_by_other_executor",
        "update_task_checkpointed_result",
        "lease_owner = NULL",
        "lease_expires_at = 0",
        "AND lease_expires_at <= ?1",
        "if task_lease_expires_at > now_ts",
        "AND lease_expires_at <= ?3",
        "merge_progress_with_active_resume_coordination",
        "task_progress_cas_exhausted",
        "AND claim_attempt = ?",
    ),
    "crates/clawd/src/repo/task_mutation_ledger.rs": (
        "TaskMutationClaimRejected",
        "enum TaskMutationPhase",
        "IntentRecorded",
        "AttemptStarted",
        "ReceiptRecorded",
        "VerificationPending",
        "ReconciliationRequired",
        "mutation_idempotency_key",
        "start_task_mutation_attempt",
        "record_task_mutation_receipt",
        "record_task_mutation_verification",
        "reconcile_task_mutation",
        "commit_task_mutation",
        "require_active_task_claim",
        "lease_owner",
        "claim_attempt",
        "WORKER_LEASE_LOST_STATUS_CODE",
    ),
    "crates/clawd/src/agent_engine/mutation_ledger.rs": (
        "load_task_mutation_reconciliation_directive",
        '"/task_lifecycle/resume_input/new_constraints/mutation_reconciliation"',
        '"fingerprint_hash"',
        '"disposition"',
        "safe_reconciliation_projection",
    ),
    "crates/clawd/src/skills/runner.rs": (
        '"execution"',
        '"idempotency_key"',
        '"attempt_no"',
    ),
    "crates/clawd/src/skills/external.rs": (
        '"Idempotency-Key"',
        '"RUSTCLAW_IDEMPOTENCY_KEY"',
        '"execution"',
    ),
    "crates/clawd/src/worker/run_skill_mutation.rs": (
        "prepare_direct_run_skill_mutation",
        "persist_direct_run_skill_mutation_result",
        "finalize_direct_run_skill_reconciliation",
        "DirectRunSkillMutationGuard",
        "mutation_reconciliation",
        "update_task_checkpointed_result",
    ),
    "crates/clawd/src/repo/task_resume_execution/resume_lease.rs": (
        "merge_progress_with_active_resume_coordination",
        "resume_execution_progress",
        "renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal",
        "active_claim_chain_matches",
        "resume_executor_dispatch_claim",
        "lease_expires_at > ?3",
        "claimed.task.claim_attempt",
    ),
    "crates/clawd/src/task_event_transport.rs": (
        "publish_claimed_event",
        "publish_claimed_journal_snapshot",
        "publish_claimed_task_event",
        "task.claim_attempt",
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
        "checkpointed_ask_finalization_preserves_pending_approval_context",
        "lease_owner.is_none()",
        "assert_eq!(lease_expires_at, 0)",
    ),
    "crates/clawd/src/repo/task_approval_tests.rs": (
        "failed_task_pending_approval_compatibility_is_rejected",
        "approval_resumes_checkpoint_and_consumes_exact_binding_once",
        "deny_closes_the_exact_request_without_requeueing",
    ),
    "crates/clawd/src/repo/task_resume_execution_tests/resume_lease.rs": (
        "active_resume_dispatch_lease_renews_the_complete_claim_chain",
        "stale_resume_generation_cannot_renew_same_worker_lease",
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
    "crates/clawd/src/task_budget_contract.rs": (
        "enum BudgetDecision",
        "Continue",
        "Finish",
        "CheckpointRequeue",
        "Waiting",
        "NeedsUser",
        "Terminal",
        "struct BudgetHardCeilings",
        "struct TaskBudgetSlice",
        "struct BudgetObservation",
        "profile_for_verified_plan",
        "advanced_from",
        "provider_call_timeout_seconds",
        "tool_call_timeout_seconds",
        "load_task_budget_policy",
    ),
    "configs/agent_guard.toml": (
        "[agent.task_budget]",
        "admin_max_model_turns",
        "admin_max_tool_calls",
        "admin_max_total_tokens",
        "admin_max_cost_usd_nanos",
        "admin_max_elapsed_seconds",
        "admin_max_continuations",
        "admin_max_non_resumable_tool_seconds",
        "[agent.task_budget.profiles.general]",
        "[agent.task_budget.profiles.fast_read]",
        "[agent.task_budget.profiles.grounded_summary]",
        "[agent.task_budget.profiles.multi_step_workspace]",
        "[agent.task_budget.profiles.ops_closed_loop]",
        "soft_slice_seconds",
        "stagnation_tolerance",
        "provider_timeout_class",
        "tool_timeout_class",
    ),
    "crates/clawcli/src/events.rs": (
        '"profile"',
        '"continuation_index"',
        '"cumulative_model_turns"',
        '"cumulative_tool_calls"',
        '"soft_slice_exhausted"',
        '"resumable"',
    ),
    "crates/clawd/src/providers/client.rs": (
        "timeout_seconds: Option<u64>",
        "effective_provider_timeout_seconds",
        "hints.timeout_seconds",
    ),
    "crates/clawd/src/agent_engine/planning.rs": (
        "provider_call_timeout_seconds",
        "provider_timeout_seconds",
        "run_with_fallback_with_hints",
    ),
    "crates/clawd/src/agent_engine/skill_execution.rs": (
        "tool_call_timeout_seconds",
        "run_with_tool_budget_timeout",
        '"agent_tool_timeout"',
        '"resumable": false',
    ),
    "UI/src/lib/task-result.ts": (
        '"budget_decision"',
        '"Task budget decision"',
        '"continuation_index"',
        '"cumulative_model_turns"',
        '"soft_slice_exhausted"',
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
        "`worker_lease_lost`",
        "`claim_attempt`",
        "`resume_entrypoint = \"poll_async_job\"`",
        "`clawcli resume-task <task_id>`",
        "`TaskBudgetSlice`",
        "`BudgetDecision`",
        "`checkpoint_requeue`",
        "administrator hard ceilings",
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

FORBIDDEN_INTERACTIVE_BUDGET_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "configs/agent_guard.toml": (
        "\nmax_rounds =",
        "\nmax_tool_calls =",
        "\nno_progress_limit =",
        "\nrecoverable_failure_extra_rounds =",
        "\nmulti_round_enabled =",
    ),
    "crates/clawd/src/agent_engine/support.rs": (
        "max_rounds:",
        "max_tool_calls:",
        "no_progress_limit:",
        "recoverable_failure_extra_rounds:",
        "multi_round_enabled:",
    ),
    "crates/clawd/src/agent_engine/loop_control.rs": (
        "agent_loop_max_rounds",
        "agent_loop_no_progress_limit",
        "budget_near_exhaustion",
        "recoverable_failure_extra_rounds",
        "multi_round_enabled",
    ),
    "crates/clawd/src/agent_engine/execution_loop.rs": (
        "agent_loop_max_tool_calls",
        "max_tool_calls_reached",
        "budget_near_exhaustion",
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
    for rel_path in FORBIDDEN_INTERACTIVE_BUDGET_TOKENS_BY_PATH:
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

    for rel_path, tokens in FORBIDDEN_INTERACTIVE_BUDGET_TOKENS_BY_PATH.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token in text:
                findings.append(f"forbidden_interactive_budget_token:{rel_path}:{token}")

    mutation_ledger = texts.get("crates/clawd/src/repo/task_mutation_ledger.rs") or ""
    for token in (
        "status             TEXT NOT NULL CHECK (status IN ('started', 'completed', 'uncertain'))",
        "BeginTaskMutationOutcome::Completed",
        "BeginTaskMutationOutcome::Uncertain",
    ):
        if token in mutation_ledger:
            findings.append(f"forbidden_legacy_mutation_ledger_token:{token}")

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
    for rel_path in FORBIDDEN_INTERACTIVE_BUDGET_TOKENS_BY_PATH:
        texts.setdefault(rel_path, "")
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

    legacy_budget = dict(good)
    legacy_budget["configs/agent_guard.toml"] += "\nmax_rounds = 4"
    findings = scan_texts(legacy_budget)
    assert any("forbidden_interactive_budget_token" in item for item in findings)

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
