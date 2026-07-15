#!/usr/bin/env python3
"""Validate clawcli exec/replay machine contracts.

This guard keeps the Codex-like CLI surface release-gated: `clawcli exec/code`
must expose stable exit/profile/artifact/compact fields, and `clawcli replay`
must stay recorded-only with coverage, view, and machine diff-class contracts.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawcli/src/commands/exec.rs": (
        "pub(super) enum ExecExitClass",
        "ExecExitClass::Success",
        "ExecExitClass::Timeout",
        "ExecExitClass::NeedsUser",
        "ExecExitClass::PolicyDenied",
        "ExecExitClass::ProviderUnavailable",
        "ExecExitClass::InvalidRequest",
        "pub(super) fn exec_effective_options",
        'Some("quick")',
        'Some("coding")',
        'Some("release-gate")',
        'Some("long-tail")',
        "pub(super) fn exec_summary_json",
        '"effective_config"',
        '"exit_class"',
        '"exit_code"',
        '"resume"',
        '"llm"',
        '"coding"',
        "pub(super) fn exec_compact_text_lines",
        "exec_compact_profile",
        "exec_compact_task_id",
        "exec_compact_budget_status",
        "exec_compact_resume_mode",
        "exec_compact_checkpoint_id",
        "exec_compact_resume_due",
        "exec_compact_changed_file_count",
        "exec_compact_verification_command_count",
        "exec_compact_verification_status",
        "exec_compact_next_step",
        "exec_compact_checkpoint_ref_count",
        "exec_compact_completed_side_effect_count",
        "exec_compact_unverified_risk",
        "exec_compact_artifact_index",
        "exec_compact_changed_file",
        "exec_compact_verification_command",
        "pub(super) fn write_exec_artifacts",
        "summary.json",
        "task.json",
        "events.jsonl",
        "verification.json",
        "diff_summary.json",
        "llm_summary.json",
        "resume.json",
        "index.json",
        "rustclaw_exec_artifact_index",
        "exec_failure_class_from_machine_tokens",
        "is_exec_machine_token",
    ),
    "crates/clawcli/src/replay.rs": (
        "pub(crate) fn run_export",
        "pub(crate) fn run_run",
        "pub(crate) fn run_diff",
        "fn replay_bundle_json",
        "fn replay_run_summary",
        "fn replay_view_json",
        "fn replay_diff_summary",
        "fn recorded_execution_replay",
        "fn replay_recording_coverage",
        "fn replay_diff_classes",
        "rustclaw_task_replay",
        "rustclaw_task_replay_diff",
        "recorded_only",
        '"live_provider": false',
        '"live_tool_invocations": false',
        '"strategy": "recorded_outputs_first"',
        '"provider_call_count": 0',
        '"tool_invocation_count": 0',
        '"coverage"',
        '"llm"',
        '"tools"',
        '"checkpoints"',
        '"summary"',
        "final_status_changed",
        "event_count_changed",
        "artifact_count_changed",
        "route_changed",
        "plan_changed",
        "verifier_changed",
        "permission_changed",
        "tool_result_changed",
    ),
    "crates/clawcli/src/main.rs": (
        "enum ReplayCommand",
        "Export {",
        "Run {",
        "Diff {",
        "coverage: bool",
        "view: ReplayView",
        "enum ReplayView",
        "Summary",
        "Llm",
        "Tools",
        "Checkpoints",
        "run_exec_command",
        "CodeCommand::Run",
        "replay::run_export",
        "replay::run_run",
        "replay::run_diff",
    ),
    "crates/clawcli/src/commands_tests.rs": (
        "exec_summary_json_exposes_stable_machine_fields",
        "exec_compact_text_lines_include_coding_budget_and_resume_tokens",
        "exec_summary_json_records_resume_source_task_id",
        "exec_offline_smoke_writes_machine_artifact_without_server",
        "exec_profile_resolves_machine_options_without_prompt_semantics",
        "write_exec_artifacts",
        "summary.json",
        "task.json",
        "events.jsonl",
        "verification.json",
        "diff_summary.json",
        "llm_summary.json",
        "resume.json",
        "index.json",
        "exec_compact_budget_status",
        "exec_compact_checkpoint_id",
        "exec_compact_artifact_index",
    ),
    "crates/clawcli/src/replay_tests.rs": (
        "replay_run_summary_is_recorded_only_machine_result",
        "replay_view_json_filters_llm_tools_and_checkpoints",
        "replay_run_summary_reports_failing_task_fixture_coverage",
        "replay_diff_summary_reports_machine_field_changes",
        "replay_offline_smoke_runs_bundle_and_diff_without_providers",
        "recorded_only",
        "live_provider",
        "live_tool_invocations",
        "recorded_outputs_first",
        "provider_call_count",
        "tool_invocation_count",
        "coverage",
        "final_status_changed",
        "route_changed",
        "plan_changed",
        "verifier_changed",
        "permission_changed",
        "tool_result_changed",
    ),
    "scripts/clawcli_smoke.sh": (
        "SMOKE exec-effective-config",
        "clawcli",
        "exec",
        "--print-effective-config",
        "SMOKE subagents",
        "SMOKE permission inspect",
        "SMOKE replay",
        "replay export",
        "replay run",
        "--coverage",
    ),
    "docs/clawcli_exec_replay.md": (
        "clawcli exec",
        "clawcli code",
        "clawcli replay",
        "summary.json",
        "task.json",
        "events.jsonl",
        "verification.json",
        "diff_summary.json",
        "llm_summary.json",
        "resume.json",
        "index.json",
        "exec_compact_budget_status",
        "exec_compact_checkpoint_id",
        "exec_compact_artifact_index",
        "Profiles only set CLI machine parameters",
        "recorded_only",
        "replay run artifacts/replay/task.json --coverage",
        "replay run artifacts/replay/task.json --view llm --json",
        "replay run artifacts/replay/task.json --view tools --json",
        "replay run artifacts/replay/task.json --view checkpoints --json",
        "replay diff artifacts/replay/before.json artifacts/replay/after.json --json",
        "diff_classes",
        "final_status_changed",
        "route_changed",
        "plan_changed",
        "permission_changed",
        "must consume machine fields",
        "must not parse `result_text` or `error_text`",
    ),
    "README.md": (
        "clawcli exec",
        "clawcli replay export/run/diff",
        "exec_compact_*",
        "summary.json",
        "llm_summary.json",
        "resume.json",
        "recorded_only",
        "clawcli_exec_replay_contracts.txt",
        "clawcli_exec_replay_contracts=1",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0",
    ),
    "README.zh-CN.md": (
        "clawcli exec",
        "clawcli replay export/run/diff",
        "exec_compact_*",
        "summary.json",
        "llm_summary.json",
        "resume.json",
        "recorded_only",
        "clawcli_exec_replay_contracts.txt",
        "clawcli_exec_replay_contracts=1",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/README.md": (
        "clawcli_exec_replay_contracts.txt",
        "clawcli_exec_replay_contracts=1",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0",
        "exec artifact",
        "recorded-only replay",
        "diff class",
    ),
    "AGENTS.md": (
        "scripts/check_clawcli_exec_replay_contracts.py",
        "clawcli_exec_replay_contracts.txt",
        "clawcli_exec_replay_contracts=1",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/run_agent_parity_gate.sh": (
        "AGENT_PARITY_GATE_STEP clawcli_exec_replay_contracts",
        "check_clawcli_exec_replay_contracts.py",
        "clawcli_exec_replay_contracts.txt",
        "clawcli_exec_replay_contracts=1",
    ),
    "scripts/nl_tests/check_suite_artifact_contract.py": (
        "agent_parity_gate/clawcli_exec_replay_contracts.txt",
        '"clawcli_exec_replay_contracts": "1"',
        "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok",
        "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0",
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

    exec_text = texts.get("crates/clawcli/src/commands/exec.rs") or ""
    for artifact in (
        "summary.json",
        "task.json",
        "events.jsonl",
        "verification.json",
        "diff_summary.json",
        "llm_summary.json",
        "resume.json",
        "index.json",
    ):
        if exec_text.count(artifact) < 2:
            findings.append(f"exec_artifact_contract_too_weak:{artifact}")
    compact_count = exec_text.count("exec_compact_")
    if compact_count < 12:
        findings.append("exec_compact_machine_line_contract_too_weak")

    replay_text = texts.get("crates/clawcli/src/replay.rs") or ""
    if replay_text.count("recorded_only") < 4:
        findings.append("replay_recorded_only_contract_too_weak")
    for forbidden_hint in ("openai", "minimax", "mimo", "qwen", "deepseek"):
        if forbidden_hint in replay_text.lower():
            findings.append(f"replay_should_not_reference_live_provider:{forbidden_hint}")
    for view in ('"llm"', '"tools"', '"checkpoints"', '"summary"'):
        if replay_text.count(view) < 1:
            findings.append(f"replay_missing_view:{view}")

    tests = "\n".join(
        texts.get(path) or ""
        for path in (
            "crates/clawcli/src/commands_tests.rs",
            "crates/clawcli/src/replay_tests.rs",
        )
    )
    for test_name in (
        "exec_compact_text_lines_include_coding_budget_and_resume_tokens",
        "exec_profile_resolves_machine_options_without_prompt_semantics",
        "replay_run_summary_is_recorded_only_machine_result",
        "replay_diff_summary_reports_machine_field_changes",
    ):
        if test_name not in tests:
            findings.append(f"missing_rust_clawcli_contract_test:{test_name}")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    texts["crates/clawcli/src/commands/exec.rs"] += "\n" + "\n".join(
        [
            "summary.json",
            "task.json",
            "events.jsonl",
            "verification.json",
            "diff_summary.json",
            "llm_summary.json",
            "resume.json",
            "index.json",
            *["exec_compact_field" for _ in range(12)],
        ]
    )
    texts["crates/clawcli/src/replay.rs"] += "\n" + "\n".join(
        [
            "recorded_only",
            "recorded_only",
            "recorded_only",
            "recorded_only",
        ]
    )
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    good_findings = scan_texts(good)
    assert not good_findings, good_findings

    missing_artifact = dict(good)
    missing_artifact["crates/clawcli/src/commands/exec.rs"] = (
        missing_artifact["crates/clawcli/src/commands/exec.rs"] or ""
    ).replace("llm_summary.json", "")
    findings = scan_texts(missing_artifact)
    assert any("llm_summary.json" in item for item in findings), findings

    missing_compact = dict(good)
    missing_compact["crates/clawcli/src/commands/exec.rs"] = "exec_compact_task_id"
    findings = scan_texts(missing_compact)
    assert any("exec_compact" in item for item in findings), findings

    missing_recorded_only = dict(good)
    missing_recorded_only["crates/clawcli/src/replay.rs"] = (
        missing_recorded_only["crates/clawcli/src/replay.rs"] or ""
    ).replace("recorded_only", "")
    findings = scan_texts(missing_recorded_only)
    assert any("recorded_only" in item for item in findings), findings

    missing_tests = dict(good)
    missing_tests["crates/clawcli/src/replay_tests.rs"] = "coverage"
    findings = scan_texts(missing_tests)
    assert any("missing_rust_clawcli_contract_test" in item for item in findings), findings

    print("CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
