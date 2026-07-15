#!/usr/bin/env python3
"""Validate clawcli session/TUI machine contracts.

This guard keeps Codex-like task continuity release-gated: local session
metadata, resume/checkpoint controls, TUI selected-task snapshots, operator
key tokens, and report/review/subagent/permission projections must stay
machine-readable protocol instead of prose/log-only behavior.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawcli/src/commands/session.rs": (
        "pub(crate) fn run_session_list",
        "pub(crate) fn run_session_show",
        "pub(crate) fn run_session_resume",
        "pub(crate) fn run_session_archive",
        "pub(crate) fn run_session_delete",
        "pub(crate) fn run_session_fork",
        "pub(super) fn session_list_json",
        "pub(super) fn session_show_json",
        "pub(super) fn session_resume_json",
        "pub(super) struct SessionStore",
        "pub(super) struct StoredSession",
        "pub(super) fn session_store_upsert_summary",
        "pub(super) fn session_store_archive_json",
        "pub(super) fn session_store_delete_json",
        "pub(super) fn session_store_fork_json",
        "session_kind",
        "user_chat_active_tasks",
        "task_session",
        "session_id",
        "task_ids",
        "active_goal_id",
        "workspace_root",
        "latest_checkpoint_id",
        "latest_event_seq",
        "archived",
        "forked_from",
        "session_resume",
        "resume_due",
        "resume_reason",
        "next_action_kind",
        "operation",
        "session_store_upsert",
        "session_archive",
        "session_delete",
        "session_fork",
        "RUSTCLAW_CLAWCLI_SESSION_STORE",
        "XDG_STATE_HOME",
        "clawcli_sessions.json",
    ),
    "crates/clawcli/src/commands/tui.rs": (
        "pub(crate) fn run_tui",
        "pub(super) enum TuiCommand",
        "pub(super) fn tui_snapshot_json",
        "pub(super) fn tui_export_json",
        "pub(super) fn tui_command_from_input",
        "pub(super) fn tui_selected_task_lines",
        "snapshot_kind",
        "rustclaw_cli_tui",
        "export_kind",
        "rustclaw_cli_tui_export",
        "selected_task",
        "selected_progress",
        "selected_summary",
        "TuiCommand::Refresh",
        "TuiCommand::Watch",
        "TuiCommand::Pause",
        "TuiCommand::Cancel",
        "TuiCommand::Resume",
        "TuiCommand::Continue",
        "TuiCommand::Export",
        "TuiCommand::Report",
        "TuiCommand::Review",
        "TuiCommand::Subagents",
        "TuiCommand::Permission",
        "TuiCommand::Quit",
        "task::pause_task_by_id",
        "task::cancel_task_by_id",
        "task::resume_task_by_id",
        'Some("operator_tui")',
        'Some("operator_tui_continue")',
        "task_report_json",
        "coding_review_json",
        "subagent_report_json",
        "permission_report_json",
        "tui_selected_checkpoint_id",
        "tui_selected_resume_due",
        "tui_selected_resume_wait_seconds",
        "tui_selected_next_action_kind",
        "tui_selected_pending_async_job_id",
        "tui_selected_poll_ref",
        "tui_selected_lease_owner",
        "tui_selected_heartbeat_at",
        "tui_selected_llm_call_count",
        "tui_selected_llm_budget_status",
        "tui_selected_changed_file_count",
        "tui_selected_verification_command_count",
        "tui_selected_verification_status",
        "tui_selected_completed_side_effect_count",
        "tui_selected_unverified_risk",
        "tui_selected_artifact_ref_count",
        "tui_selected_goal_id",
        "tui_selected_goal_status",
        "tui_selected_outcome_state",
        "tui_selected_done_condition_count",
        "tui_selected_current_progress_count",
        "tui_selected_remaining_work_count",
        "tui_keys: r,w,p,c,u,n,e,1,2,3,4,q",
        "tui_key.4=permission",
    ),
    "crates/clawcli/src/main.rs": (
        "enum SessionCommand",
        "SessionCommand::List",
        "SessionCommand::Show",
        "SessionCommand::Resume",
        "SessionCommand::Archive",
        "SessionCommand::Delete",
        "SessionCommand::Fork",
        "commands::run_session_list",
        "commands::run_session_show",
        "commands::run_session_resume",
        "commands::run_session_archive",
        "commands::run_session_delete",
        "commands::run_session_fork",
        "commands::run_tui",
        "clawcli_parses_session_subcommands",
    ),
    "crates/clawcli/src/commands_session_tests.rs": (
        "session_list_json_indexes_active_task_machine_fields",
        "session_show_json_wraps_task_goal_checkpoint_and_report",
        "session_resume_json_extracts_machine_resume_fields",
        "session_store_archive_delete_and_fork_use_machine_metadata",
        "user_chat_active_tasks",
        "task_session",
        "session_store_upsert",
        "session_archive",
        "session_fork",
        "session_delete",
        "latest_checkpoint_id",
        "latest_event_seq",
    ),
    "crates/clawcli/src/commands_tests.rs": (
        "tui_snapshot_json_wraps_active_and_selected_task",
        "tui_command_parser_accepts_basic_key_tokens",
        "tui_export_json_wraps_snapshot_and_selected_task_id",
        "tui_selected_task_lines_expose_resume_llm_and_coding_tokens",
        "rustclaw_cli_tui",
        "rustclaw_cli_tui_export",
        "selected_progress",
        "selected_summary",
        "tui_selected_checkpoint_id",
        "tui_selected_resume_due",
        "tui_selected_llm_call_count",
        "tui_selected_verification_status",
        "tui_selected_goal_status",
        "tui_selected_remaining_work_count",
    ),
    "README.md": (
        "clawcli session list",
        "clawcli session show",
        "clawcli session resume",
        "clawcli session archive",
        "clawcli session fork",
        "clawcli tui",
        "selected_progress",
        "selected_summary",
        "clawcli_session_tui_contracts.txt",
        "clawcli_session_tui_contracts=1",
        "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok",
        "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0",
    ),
    "README.zh-CN.md": (
        "clawcli session list",
        "clawcli session show",
        "clawcli session resume",
        "clawcli session archive",
        "clawcli session fork",
        "clawcli tui",
        "selected_progress",
        "selected_summary",
        "clawcli_session_tui_contracts.txt",
        "clawcli_session_tui_contracts=1",
        "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok",
        "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/README.md": (
        "clawcli_session_tui_contracts.txt",
        "clawcli_session_tui_contracts=1",
        "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok",
        "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0",
        "session store",
        "TUI selected task",
    ),
    "AGENTS.md": (
        "scripts/check_clawcli_session_tui_contracts.py",
        "clawcli_session_tui_contracts.txt",
        "clawcli_session_tui_contracts=1",
        "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok",
        "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/run_agent_parity_gate.sh": (
        "AGENT_PARITY_GATE_STEP clawcli_session_tui_contracts",
        "check_clawcli_session_tui_contracts.py",
        "clawcli_session_tui_contracts.txt",
        "clawcli_session_tui_contracts=1",
    ),
    "scripts/nl_tests/check_suite_artifact_contract.py": (
        "agent_parity_gate/clawcli_session_tui_contracts.txt",
        '"clawcli_session_tui_contracts": "1"',
        "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok",
        "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0",
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

    session_text = texts.get("crates/clawcli/src/commands/session.rs") or ""
    for token in (
        "session_id",
        "task_ids",
        "active_goal_id",
        "latest_checkpoint_id",
        "latest_event_seq",
        "archived",
        "forked_from",
    ):
        if session_text.count(token) < 2:
            findings.append(f"session_store_schema_token_too_weak:{token}")

    tui_text = texts.get("crates/clawcli/src/commands/tui.rs") or ""
    for key_token in (
        '"r"',
        '"w"',
        '"p"',
        '"c"',
        '"u"',
        '"n"',
        '"e"',
        '"1"',
        '"2"',
        '"3"',
        '"4"',
        '"q"',
    ):
        if key_token not in tui_text:
            findings.append(f"tui_missing_key_token:{key_token}")
    if tui_text.count("tui_selected_") < 18:
        findings.append("tui_selected_machine_line_contract_too_weak")

    tests = "\n".join(
        texts.get(path) or ""
        for path in (
            "crates/clawcli/src/commands_session_tests.rs",
            "crates/clawcli/src/commands_tests.rs",
        )
    )
    for test_name in (
        "session_list_json_indexes_active_task_machine_fields",
        "session_store_archive_delete_and_fork_use_machine_metadata",
        "tui_snapshot_json_wraps_active_and_selected_task",
        "tui_selected_task_lines_expose_resume_llm_and_coding_tokens",
    ):
        if test_name not in tests:
            findings.append(f"missing_rust_session_tui_contract_test:{test_name}")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    texts["crates/clawcli/src/commands/session.rs"] += "\n" + "\n".join(
        [
            "session_id",
            "task_ids",
            "active_goal_id",
            "latest_checkpoint_id",
            "latest_event_seq",
            "archived",
            "forked_from",
        ]
    )
    texts["crates/clawcli/src/commands/tui.rs"] += "\n" + "\n".join(
        [
            '"r"',
            '"w"',
            '"p"',
            '"c"',
            '"u"',
            '"n"',
            '"e"',
            '"1"',
            '"2"',
            '"3"',
            '"4"',
            '"q"',
            *["tui_selected_field" for _ in range(18)],
        ]
    )
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    good_findings = scan_texts(good)
    assert not good_findings, good_findings

    missing_store = dict(good)
    missing_store["crates/clawcli/src/commands/session.rs"] = "session_id"
    findings = scan_texts(missing_store)
    assert any("session_store_schema_token_too_weak" in item for item in findings), findings

    missing_selected_progress = dict(good)
    missing_selected_progress["crates/clawcli/src/commands/tui.rs"] = (
        missing_selected_progress["crates/clawcli/src/commands/tui.rs"] or ""
    ).replace("selected_progress", "")
    findings = scan_texts(missing_selected_progress)
    assert any("selected_progress" in item for item in findings), findings

    missing_key = dict(good)
    missing_key["crates/clawcli/src/commands/tui.rs"] = (
        missing_key["crates/clawcli/src/commands/tui.rs"] or ""
    ).replace('"4"', "")
    findings = scan_texts(missing_key)
    assert any("tui_missing_key_token" in item for item in findings), findings

    missing_tests = dict(good)
    missing_tests["crates/clawcli/src/commands_session_tests.rs"] = "session_id"
    findings = scan_texts(missing_tests)
    assert any("missing_rust_session_tui_contract_test" in item for item in findings), findings

    print("CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
