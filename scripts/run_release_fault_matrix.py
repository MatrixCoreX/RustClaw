#!/usr/bin/env python3
"""Run the release fault-injection matrix and write machine-readable evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASE_BINARY = ROOT / "target" / "release" / "clawd"


@dataclass(frozen=True)
class FaultCase:
    track: str
    case_id: str
    test_name: str
    package: str = "clawd"
    binary: str | None = "clawd"


CASES = (
    FaultCase(
        "D",
        "sandbox_read_only_write_denied",
        "verifier::permission_tests::read_only_sandbox_blocks_workspace_write",
    ),
    FaultCase(
        "D",
        "subprocess_parent_secret_not_inherited",
        "skills::tests::run_cmd_does_not_inherit_undeclared_parent_secret",
    ),
    FaultCase(
        "D",
        "approval_binding_task_and_expiry",
        "approval_grant::tests::pending_request_is_task_bound_and_expiring",
    ),
    FaultCase(
        "E",
        "stale_patch_rejected_without_mutation",
        "skills::builtin::builtin_workspace_patch::tests::stale_precondition_rejects_patch_without_mutation",
    ),
    FaultCase(
        "E",
        "partial_mutation_restored",
        "skills::builtin::builtin_workspace_mutation::tests::failed_operation_restores_partial_mutation",
    ),
    FaultCase(
        "E",
        "agent_phase_restart_matrix",
        "agent_engine::checkpoint_resume_state::tests::restart_matrix_restores_all_agent_phase_machine_state",
    ),
    FaultCase(
        "E",
        "checkpoint_snapshot_bounded",
        "agent_engine::checkpoint_resume_state::tests::restart_snapshot_is_bounded_and_ignores_unknown_stage_tokens",
    ),
    FaultCase(
        "E",
        "former_loop_thresholds_are_observational",
        "task_budget_contract::tests::progress_crosses_former_round_and_tool_thresholds",
    ),
    FaultCase(
        "E",
        "administrator_ceiling_is_authoritative",
        "task_budget_contract::tests::administrator_ceiling_is_terminal_even_with_progress",
    ),
    FaultCase(
        "E",
        "budget_checkpoint_resumes_once",
        "task_budget_contract::tests::checkpoint_round_trip_resumes_once",
    ),
    FaultCase(
        "F",
        "expired_event_cursor_bounded",
        "task_event_transport::tests::bounded_replay_marks_an_expired_cursor",
    ),
    FaultCase(
        "F",
        "event_secret_fields_redacted",
        "task_event_transport::tests::secrets_and_raw_teaching_fields_are_redacted_before_persistence",
    ),
    FaultCase(
        "H",
        "mcp_reconnect_without_tool_replay",
        "mcp_runtime::tests::health_tick_reconnects_closed_transport_without_replaying_a_tool",
    ),
    FaultCase(
        "H",
        "mcp_untrusted_schema_fails_closed",
        "mcp_runtime::tests::untrusted_and_invalid_schema_servers_fail_closed",
    ),
    FaultCase(
        "I",
        "hook_hash_change_fails_validation",
        "agent_hooks::tests::changed_or_untrusted_command_hook_fails_validation_before_execution",
    ),
    FaultCase(
        "I",
        "hook_timeout_fails_closed",
        "agent_hooks::tests::slow_command_hook_times_out_with_fail_closed_decision",
    ),
    FaultCase(
        "K",
        "overlapping_child_patches_require_parent",
        "skills::builtin::builtin_child_task_patch::tests::overlapping_child_patches_require_parent_resolution",
    ),
    FaultCase(
        "K",
        "dirty_parent_blocks_child_patch",
        "skills::builtin::builtin_child_task_patch::tests::parent_dirty_change_blocks_child_patch_and_preserves_review_artifacts",
    ),
    FaultCase(
        "L",
        "mutation_response_loss_not_reacquired",
        "repo::task_mutation_ledger::tests::response_loss_restart_leaves_mutation_uncertain_instead_of_reacquiring",
    ),
    FaultCase(
        "L",
        "mutation_all_phases_survive_restart",
        "repo::task_mutation_ledger::tests::deterministic_key_and_every_durable_phase_survive_database_reopen",
    ),
    FaultCase(
        "L",
        "mutation_intent_transfers_before_attempt",
        "repo::task_mutation_ledger::tests::intent_only_restart_can_transfer_to_new_claim_without_replaying_an_attempt",
    ),
    FaultCase(
        "L",
        "mutation_reconciled_phase_suppresses_replay",
        "repo::task_mutation_ledger::tests::applied_reconciliation_is_committable_without_original_action_replay",
    ),
    FaultCase(
        "L",
        "mutation_reconciliation_suppresses_replay",
        "agent_engine::mutation_ledger::tests::structured_reconciliation_commits_applied_effect_without_replaying_action",
    ),
    FaultCase(
        "L",
        "mutation_prose_cannot_reconcile",
        "agent_engine::mutation_ledger::tests::prose_resume_input_cannot_resolve_mutation_without_machine_directive",
    ),
    FaultCase(
        "L",
        "direct_skill_mutation_checkpoints_ambiguity",
        "worker::run_skill_mutation::tests::direct_run_skill_ambiguous_failure_checkpoints_instead_of_terminal_retry",
    ),
    FaultCase(
        "L",
        "resume_lease_renews_claim_chain",
        "repo::tasks::tests::task_resume_execution_tests::resume_lease::active_resume_dispatch_lease_renews_the_complete_claim_chain",
    ),
    FaultCase(
        "L",
        "checkpoint_handoff_requires_machine_state",
        "agent_engine::loop_control::tests::soft_budget_checkpoint::checkpoint_handoff_requires_matching_nonterminal_machine_state",
    ),
    FaultCase(
        "M",
        "durable_partial_metadata_cleanup",
        "skills::runner::tests::durable_job_metadata_write_failure_removes_partial_job_directory",
    ),
    FaultCase(
        "M",
        "durable_non_utf8_cursor",
        "local_process_job::tests::non_utf8_output_advances_exact_byte_cursor_without_replay",
    ),
    FaultCase(
        "M",
        "durable_stream_rotation_cursor_reset",
        "local_process_job::tests::output_cursor_resets_after_stream_truncation",
    ),
    FaultCase(
        "M",
        "durable_quiet_process_poll",
        "worker::async_poll_executor::tests::async_poll_quiet_local_process_stays_running_and_advances_cursor",
    ),
    FaultCase(
        "M",
        "durable_large_output_artifact",
        "worker::async_poll_executor::tests::async_poll_large_output_keeps_exact_artifact_and_bounded_preview",
    ),
    FaultCase(
        "M",
        "durable_partial_pid_metadata",
        "worker::async_poll_executor::tests::async_poll_missing_pid_metadata_is_a_stable_machine_failure",
    ),
    FaultCase(
        "M",
        "durable_invalid_terminal_record",
        "worker::async_poll_executor::tests::async_poll_invalid_exit_record_is_a_stable_machine_failure",
    ),
    FaultCase(
        "M",
        "durable_pid_identity_mismatch",
        "worker::async_poll_executor::tests::async_poll_identity_mismatch_allows_terminal_record_grace",
    ),
    FaultCase(
        "M",
        "durable_term_kill_escalation",
        "local_process_job::tests::durable_cancellation_escalates_a_verified_process_group",
    ),
    FaultCase(
        "M",
        "durable_restart_cancel_recovery",
        "local_process_job::tests::restart_recovery_escalates_a_durable_pending_cancellation",
    ),
    FaultCase(
        "M",
        "durable_runner_exit_child_cleanup",
        "local_process_job::tests::wrapper_exit_keeps_its_live_process_group_supervisable",
    ),
    FaultCase(
        "M",
        "durable_restart_terminal_recovery",
        "worker::tests::runtime_recovery_reaches_terminal_state_after_file_backed_restart",
    ),
    FaultCase(
        "M",
        "provider_multi_day_virtual_clock",
        "worker::async_poll_executor::tests::async_poll_virtual_multi_day_runtime_keeps_deadline_empty",
    ),
    FaultCase(
        "M",
        "provider_pinned_binding_drift",
        "worker::async_poll_executor::tests::skill_poll_adapter_rejects_checkpoint_binding_skill_drift",
    ),
    FaultCase(
        "M",
        "background_version_lease_expiry",
        "receipt::background_version_lease_tests::multi_day_lease_uses_explicit_expiry_and_unknown_schema_fails_safe",
        package="agent-skill-sdk",
        binary=None,
    ),
    FaultCase(
        "M",
        "sdk_process_group_cancellation",
        "process::tests::cancellation_terminates_the_complete_process_group",
        package="agent-skill-sdk",
        binary=None,
    ),
    FaultCase(
        "M",
        "sdk_operation_restart_recovery",
        "operation::tests::operations_are_durable_cancelable_and_recover_interrupted_state",
        package="agent-skill-sdk",
        binary=None,
    ),
)


def default_output_dir() -> Path:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return ROOT / "logs" / "release_evidence" / stamp / "fault_matrix"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=default_output_dir())
    parser.add_argument(
        "--track",
        action="append",
        choices=tuple(sorted({case.track for case in CASES})),
        help="Run only the selected Track; repeat to select more than one.",
    )
    parser.add_argument("--list", action="store_true", help="Print the matrix as JSON and exit.")
    return parser.parse_args()


def selected_cases(tracks: list[str] | None) -> tuple[FaultCase, ...]:
    if not tracks:
        return CASES
    selected = set(tracks)
    return tuple(case for case in CASES if case.track in selected)


def relative_artifact_ref(path: Path, output_dir: Path) -> str:
    return path.relative_to(output_dir).as_posix()


def command_text(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def sha256_file(path: Path) -> str | None:
    if not path.is_file():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_case(case: FaultCase, output_dir: Path, index: int, total: int) -> dict[str, object]:
    log_path = output_dir / "cases" / f"{case.track}_{case.case_id}.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    command = [
        "cargo",
        "test",
        "--locked",
        "-p",
        case.package,
    ]
    if case.binary:
        command.extend(["--bin", case.binary])
    command.extend([case.test_name, "--", "--exact", "--quiet"])
    print(
        f"FAULT_CASE {index}/{total} track={case.track} "
        f"case={case.case_id} test={case.test_name}",
        flush=True,
    )
    started = time.monotonic()
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    with log_path.open("w", encoding="utf-8") as log_file:
        completed = subprocess.run(
            command,
            cwd=ROOT,
            env=env,
            stdout=log_file,
            stderr=subprocess.STDOUT,
            check=False,
        )
    elapsed_ms = round((time.monotonic() - started) * 1000)
    status = "passed" if completed.returncode == 0 else "failed"
    print(
        f"FAULT_RESULT track={case.track} case={case.case_id} "
        f"status={status} exit_code={completed.returncode} elapsed_ms={elapsed_ms}",
        flush=True,
    )
    return {
        **asdict(case),
        "status": status,
        "exit_code": completed.returncode,
        "elapsed_ms": elapsed_ms,
        "log_ref": relative_artifact_ref(log_path, output_dir),
    }


def main() -> int:
    args = parse_args()
    cases = selected_cases(args.track)
    if args.list:
        print(json.dumps([asdict(case) for case in cases], indent=2, sort_keys=True))
        return 0

    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    started_at = datetime.now(timezone.utc).isoformat()
    source_commit = command_text("git", "rev-parse", "HEAD")
    changed_paths = [
        line
        for line in command_text("git", "status", "--porcelain").splitlines()
        if line.strip()
    ]
    results = [
        run_case(case, output_dir, index, len(cases))
        for index, case in enumerate(cases, start=1)
    ]
    failed = sum(result["status"] == "failed" for result in results)
    tracks = sorted({case.track for case in cases})
    summary = {
        "schema_version": 1,
        "suite": "release_fault_matrix",
        "status": "passed" if failed == 0 else "failed",
        "started_at": started_at,
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "source_commit": source_commit,
        "worktree": {
            "status": "clean" if not changed_paths else "dirty",
            "changed_path_count": len(changed_paths),
        },
        "binary": {
            "path": RELEASE_BINARY.relative_to(ROOT).as_posix(),
            "sha256": sha256_file(RELEASE_BINARY),
        },
        "platform": platform.system().lower(),
        "arch": platform.machine().lower(),
        "tracks": tracks,
        "case_count": len(results),
        "passed": len(results) - failed,
        "failed": failed,
        "cases": results,
    }
    summary_path = output_dir / "summary.json"
    summary_path.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        f"FAULT_MATRIX status={summary['status']} cases={len(results)} "
        f"passed={summary['passed']} failed={failed} summary_ref=summary.json",
        flush=True,
    )
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
