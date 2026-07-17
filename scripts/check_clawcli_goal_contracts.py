#!/usr/bin/env python3
"""Validate clawcli goal machine contracts."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawcli/src/commands/goal.rs": (
        "pub(crate) fn run_goal_start",
        "pub(crate) fn run_goal_status",
        "pub(crate) fn run_goal_pause",
        "pub(crate) fn run_goal_resume",
        "pub(crate) fn run_goal_edit",
        "pub(crate) fn run_goal_clear",
        "pub(super) fn goal_request_payload",
        "pub(super) fn goal_edit_patch_json",
        "pub(super) fn goal_status_summary_json",
        "pub(super) fn goal_status_text_lines",
        "pub(super) fn goal_control_summary_json",
        "fn goal_sensitive_field_name",
        "fn goal_public_json",
        "goal_start_wait_detach_conflict",
        "goal_resume_constraints_json_parse_failed",
        "goal_json_parse_failed",
        "goal_json_must_be_object",
        "goal_patch_empty",
        '"schema_version"',
        '"text"',
        '"goal"',
        '"objective"',
        '"done_conditions"',
        '"verification_commands"',
        '"constraints"',
        '"allowed_files_or_scopes"',
        '"forbidden_actions"',
        '"goal_status"',
        '"created"',
        '"report_kind"',
        '"rustclaw_goal_status"',
        '"task_id"',
        '"status"',
        '"execution_state"',
        '"lifecycle_state"',
        '"terminal"',
        "goal_task_id",
        "goal_task_status",
        "goal_execution_state",
        "goal_lifecycle_state",
        "goal_terminal",
        "goal_id",
        "goal_status_source",
        "goal_objective",
        "goal_done_condition_count",
        "goal_verification_command_count",
        "goal_constraint_count",
        "goal_current_progress_count",
        "goal_remaining_work_count",
        '"operation"',
        '"checkpoint_id"',
        '"resume_due"',
        '"resume_wait_seconds"',
        '"resume_entrypoint"',
        '"resume_directive"',
        '"resume_reason"',
        '"next_action_kind"',
        '"payload_json"',
        '"response"',
        "[REDACTED]",
        "contains(\"token\")",
        "contains(\"secret\")",
        "contains(\"password\")",
        "contains(\"credential\")",
        "contains(\"authorization\")",
    ),
    "crates/clawcli/src/main.rs": (
        "enum GoalCommand",
        "GoalCommand::Start",
        "GoalCommand::Status",
        "GoalCommand::Pause",
        "GoalCommand::Resume",
        "GoalCommand::Edit",
        "GoalCommand::Clear",
        "commands::run_goal_start",
        "commands::run_goal_status",
        "commands::run_goal_pause",
        "commands::run_goal_resume",
        "commands::run_goal_edit",
        "commands::run_goal_clear",
        '#[path = "main_tests.rs"]',
    ),
    "crates/clawcli/src/main_tests.rs": (
        'for required in ["start", "status", "pause", "resume", "edit", "clear"]',
    ),
    "crates/clawcli/src/commands_tests.rs": (
        "goal_request_payload_preserves_structured_goal_fields",
        "goal_edit_patch_json_merges_flags_over_goal_json",
        "goal_status_summary_and_text_lines_use_goal_projection",
        "goal_control_summary_json_extracts_resume_machine_fields",
        "schema_version",
        "done_conditions",
        "verification_commands",
        "constraints",
        "allowed_files_or_scopes",
        "forbidden_actions",
        "rustclaw_goal_status",
        "goal_done_condition_count",
        "goal_verification_command_count",
        "goal_current_progress_count",
        "resume_entrypoint",
        "resume_directive",
        "resume_reason",
        "next_action_kind",
        "[REDACTED]",
    ),
    "README.md": (
        "clawcli goal",
        "goal start",
        "goal status",
        "goal pause",
        "goal resume",
        "goal edit",
        "goal clear",
        "done_conditions",
        "verification_commands",
        "clawcli_goal_contracts.txt",
        "clawcli_goal_contracts=1",
        "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok",
        "CLAWCLI_GOAL_CONTRACT_CHECK findings=0",
    ),
    "README.zh-CN.md": (
        "clawcli goal",
        "goal start",
        "goal status",
        "goal pause",
        "goal resume",
        "goal edit",
        "goal clear",
        "done_conditions",
        "verification_commands",
        "clawcli_goal_contracts.txt",
        "clawcli_goal_contracts=1",
        "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok",
        "CLAWCLI_GOAL_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/README.md": (
        "clawcli_goal_contracts.txt",
        "clawcli_goal_contracts=1",
        "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok",
        "CLAWCLI_GOAL_CONTRACT_CHECK findings=0",
        "goal payload",
        "goal status",
    ),
    "AGENTS.md": (
        "scripts/check_clawcli_goal_contracts.py",
        "clawcli_goal_contracts.txt",
        "clawcli_goal_contracts=1",
        "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok",
        "CLAWCLI_GOAL_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/run_agent_parity_gate.sh": (
        "AGENT_PARITY_GATE_STEP clawcli_goal_contracts",
        "check_clawcli_goal_contracts.py",
        "clawcli_goal_contracts.txt",
        "clawcli_goal_contracts=1",
    ),
    "scripts/nl_tests/check_suite_artifact_contract.py": (
        "agent_parity_gate/clawcli_goal_contracts.txt",
        '"clawcli_goal_contracts": "1"',
        "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok",
        "CLAWCLI_GOAL_CONTRACT_CHECK findings=0",
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

    goal_text = texts.get("crates/clawcli/src/commands/goal.rs") or ""
    for token in (
        '"done_conditions"',
        '"verification_commands"',
        '"constraints"',
        '"resume_entrypoint"',
        '"resume_directive"',
        '"resume_reason"',
        '"next_action_kind"',
        "[REDACTED]",
    ):
        if goal_text.count(token) < 1:
            findings.append(f"goal_contract_token_missing:{token}")
    if goal_text.count("goal_") < 30:
        findings.append("goal_machine_projection_too_weak")

    tests = texts.get("crates/clawcli/src/commands_tests.rs") or ""
    for test_name in (
        "goal_request_payload_preserves_structured_goal_fields",
        "goal_edit_patch_json_merges_flags_over_goal_json",
        "goal_status_summary_and_text_lines_use_goal_projection",
        "goal_control_summary_json_extracts_resume_machine_fields",
    ):
        if test_name not in tests:
            findings.append(f"missing_rust_goal_contract_test:{test_name}")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    texts["crates/clawcli/src/commands/goal.rs"] += "\n" + "\n".join(
        [
            *["goal_projection" for _ in range(30)],
            '"done_conditions"',
            '"verification_commands"',
            '"constraints"',
            '"resume_entrypoint"',
            '"resume_directive"',
            '"resume_reason"',
            '"next_action_kind"',
            "[REDACTED]",
        ]
    )
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    good_findings = scan_texts(good)
    assert not good_findings, good_findings

    missing_payload = dict(good)
    missing_payload["crates/clawcli/src/commands/goal.rs"] = (
        missing_payload["crates/clawcli/src/commands/goal.rs"] or ""
    ).replace('"done_conditions"', "")
    findings = scan_texts(missing_payload)
    assert any("done_conditions" in item for item in findings), findings

    missing_resume = dict(good)
    missing_resume["crates/clawcli/src/commands/goal.rs"] = (
        missing_resume["crates/clawcli/src/commands/goal.rs"] or ""
    ).replace('"resume_entrypoint"', "")
    findings = scan_texts(missing_resume)
    assert any("resume_entrypoint" in item for item in findings), findings

    missing_redaction = dict(good)
    missing_redaction["crates/clawcli/src/commands/goal.rs"] = (
        missing_redaction["crates/clawcli/src/commands/goal.rs"] or ""
    ).replace("[REDACTED]", "")
    findings = scan_texts(missing_redaction)
    assert any("REDACTED" in item for item in findings), findings

    missing_tests = dict(good)
    missing_tests["crates/clawcli/src/commands_tests.rs"] = "goal_status"
    findings = scan_texts(missing_tests)
    assert any("missing_rust_goal_contract_test" in item for item in findings), findings

    print("CLAWCLI_GOAL_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CLAWCLI_GOAL_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("CLAWCLI_GOAL_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
