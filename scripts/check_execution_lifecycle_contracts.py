#!/usr/bin/env python3
"""Guard Codex/Claude-aligned long-running execution lifecycle boundaries."""

from __future__ import annotations

import argparse
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


REQUIRED_FRAGMENTS: dict[str, tuple[str, ...]] = {
    "configs/config.toml": (
        "cmd_async_timeout_seconds =",
        "cmd_async_retention_seconds =",
        "cmd_terminate_grace_seconds =",
    ),
    "crates/clawd/src/task_lifecycle.rs": (
        "runtime_deadline_at: Option<i64>",
        "retention_deadline_at: Option<i64>",
    ),
    "crates/clawd/src/async_job_contract.rs": (
        '"timeout_role": "runtime_and_poll_retention_separated"',
        '"runtime_deadline_owner": "adapter"',
        '"retention_remaining_seconds": retention_remaining_seconds',
    ),
    "crates/clawd/src/local_process_job.rs": (
        "read_output_delta",
        "process_identity_state",
        "maybe_escalate_cancel",
        "recover_pending_cancel_escalations",
        "terminate_verified_process_group",
    ),
    "crates/clawd/src/main.rs": (
        "recover_pending_cancel_escalations",
        "restored pending local process cancellation escalation",
    ),
    "crates/clawd/src/worker/async_poll_executor.rs": (
        '"process_observation"',
        '"runtime_deadline_at"',
        '"retention_deadline_at"',
        '"local_process_runtime_timeout"',
    ),
    "crates/clawd/src/worker/runtime_support/dispatch_result.rs": (
        '"retention_deadline_at"',
        '"retention_renewed"',
        "retry_after_seconds.saturating_mul(4).max(60)",
    ),
    "crates/clawd/src/repo/tasks/lifecycle_projection.rs": (
        "worker_timeout_preserves_recoverable_checkpoint",
        "PausedCheckpointRecoveryStatus::Waiting",
    ),
    "crates/clawd/src/mcp_runtime/types.rs": (
        "pub(crate) timeout_seconds: u64",
    ),
    "UI/src/lib/task-lifecycle.ts": (
        "Background process: Running",
        "Refreshing will not interrupt it",
        "async_job_runtime_deadline_at",
        "async_job_retention_deadline_at",
    ),
    "scripts/regression_long_tail_nl_flows.sh": (
        "non_x_dry_run",
    ),
    "scripts/regression_clawd_restart_boundaries.py": (
        'CASE_IDS = ("start_boundary", "poll_boundary", "cancel_boundary")',
        "cancel_escalated_signal",
        "mutation_count",
        "source_commit_pushed",
    ),
    "scripts/nl_tests/build_builtin_tool_skill_subset.py": (
        '"selected_non_x_dry_run_count"',
    ),
    "scripts/skill_calls/_run_skill.sh": (
        "expected_skill_version",
        "expected_manifest_digest",
        "expected_receipt_digest",
        "expected_registry_generation: 0",
    ),
}

FORBIDDEN_FRAGMENTS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/async_job_contract.rs": (
        '"max_runtime_seconds"',
        '"max_runtime_deadline_ts"',
        '"timeout_role": "poll_retention"',
    ),
    "crates/clawd/src/local_process_job.rs": (
        "read_output_tail",
    ),
    "crates/clawd/src/worker/runtime_support/dispatch_result.rs": (
        'reason_code = "async_poll_expired"',
    ),
}


def scan_texts(texts: dict[str, str]) -> list[str]:
    findings: list[str] = []
    for relative, fragments in REQUIRED_FRAGMENTS.items():
        text = texts.get(relative)
        if text is None:
            findings.append(f"{relative}: missing file")
            continue
        for fragment in fragments:
            if fragment not in text:
                findings.append(f"{relative}: missing required fragment {fragment!r}")
    for relative, fragments in FORBIDDEN_FRAGMENTS.items():
        text = texts.get(relative)
        if text is None:
            continue
        for fragment in fragments:
            if fragment in text:
                findings.append(f"{relative}: forbidden legacy fragment {fragment!r}")
    return findings


def repository_texts() -> dict[str, str]:
    paths = set(REQUIRED_FRAGMENTS) | set(FORBIDDEN_FRAGMENTS)
    return {
        relative: (ROOT / relative).read_text(encoding="utf-8")
        for relative in sorted(paths)
        if (ROOT / relative).is_file()
    }


def run_self_test() -> None:
    complete = {
        relative: "\n".join(fragments)
        for relative, fragments in REQUIRED_FRAGMENTS.items()
    }
    for relative, fragments in FORBIDDEN_FRAGMENTS.items():
        complete.setdefault(relative, "")
        assert all(fragment not in complete[relative] for fragment in fragments)
    assert scan_texts(complete) == []

    broken = dict(complete)
    broken["crates/clawd/src/async_job_contract.rs"] += '\n"max_runtime_seconds"'
    findings = scan_texts(broken)
    assert any("forbidden legacy fragment" in finding for finding in findings)
    del broken["UI/src/lib/task-lifecycle.ts"]
    findings = scan_texts(broken)
    assert any("missing file" in finding for finding in findings)
    print("EXECUTION_LIFECYCLE_CONTRACT_SELF_TEST_OK")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0

    findings = scan_texts(repository_texts())
    if findings:
        print("EXECUTION_LIFECYCLE_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print("EXECUTION_LIFECYCLE_CONTRACT_CHECK ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
