#!/usr/bin/env python3
"""Run the release fault-injection matrix and write machine-readable evidence."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class FaultCase:
    track: str
    case_id: str
    test_name: str


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


def run_case(case: FaultCase, output_dir: Path, index: int, total: int) -> dict[str, object]:
    log_path = output_dir / "cases" / f"{case.track}_{case.case_id}.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    command = [
        "cargo",
        "test",
        "--locked",
        "-p",
        "clawd",
        "--bin",
        "clawd",
        case.test_name,
        "--",
        "--exact",
        "--quiet",
    ]
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
