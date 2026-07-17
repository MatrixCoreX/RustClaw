#!/usr/bin/env python3
"""Guard trusted agent-hook execution and machine-only event contracts."""

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_FILE_TOKENS = {
    "crates/clawd/src/agent_hooks.rs": (
        "HOOK_EVENT_SCHEMA_VERSION",
        "pub(crate) enum HookStage",
        "PermissionRequest",
        "PreCompact",
        "PostCompact",
        "SubagentStart",
        "SubagentStop",
        "mod command",
        "pub(crate) use command::pre_tool_use_outcome_for_state",
        "HookEvaluation",
        "merge_hook_decision",
        "PolicyDecision::parse_token",
    ),
    "crates/clawd/src/agent_hooks/command.rs": (
        "HOOK_OUTPUT_SCHEMA_VERSION",
        "HookHandlerConfig",
        "#[serde(default, deny_unknown_fields)]",
        "#[serde(deny_unknown_fields)]",
        "validate_command_handler",
        "hook_handler_hash_mismatch",
        "ToolSandboxMode::ReadOnly",
        "ProcessNetworkPolicy::Deny",
        ".env_clear()",
        "CancellationToken",
        "hook_handler_timeout",
        "hook_handler_output_too_large",
        "hook_async_decision_forbidden",
        "PolicyDecision::parse_token",
        '"argument_fields"',
        '"argument_byte_count"',
        '"handler_id"',
        '"trust_status"',
        '"content_sha256"',
        '"output_truncated"',
    ),
    "crates/clawd/src/agent_runtime_contract.rs": (
        "crate::agent_hooks::HookStage::all()",
        "expected_hook_stages",
        ".map(|stage| stage.as_token())",
    ),
    "crates/clawd/src/agent_engine/skill_execution.rs": (
        "pre_tool_use_outcome_for_state",
        ".await",
        "record_hook_evaluation_observation",
    ),
    "crates/clawd/src/agent_engine/skill_execution_observations.rs": (
        "record_hook_evaluation_observation",
        "handler_observations",
        "to_machine_json",
    ),
    "configs/agent_guard.toml": (
        "[[agent.hooks.handlers]]",
        'kind = "command"',
        "content_sha256",
        "timeout_ms",
        "max_input_bytes",
        "max_output_bytes",
        "max_attempts",
        "failure_policy",
    ),
    "crates/clawd/src/agent_hooks_tests.rs": (
        "hook_stage_contract_exposes_all_versioned_lifecycle_events",
        "pre_tool_event_exposes_machine_shape_without_argument_values",
        "trusted_hash_bound_command_hook_returns_structured_decision",
        "changed_or_untrusted_command_hook_fails_validation_before_execution",
        "slow_command_hook_times_out_with_fail_closed_decision",
        "command_hook_cancellation_stops_the_child_and_fails_closed",
        "command_hook_output_rejects_semantic_rewrite_fields_and_merges_conservatively",
    ),
    "crates/clawd/src/agent_engine/skill_execution_hook_policy_tests.rs": (
        "trusted_command_hook_blocks_through_production_pre_tool_path",
        'handler["trust_status"]',
        'handler["content_sha256"]',
    ),
}

FORBIDDEN_PRODUCTION_PATTERNS = (
    (re.compile(r'"(?:user_prompt|final_answer|response_text|raw_response)"\s*:'), "semantic_payload_field"),
    (
        re.compile(r'args\s*\.\s*get\s*\(\s*"(?:text|prompt|query|command|content)"'),
        "raw_argument_value_read",
    ),
    (re.compile(r"std::process::Command|tokio::process::Command"), "direct_process_escape"),
    (re.compile(r"ToolSandboxMode::DangerFull"), "danger_full_hook_execution"),
    (
        re.compile(r"(?:reason_code|status_code|decision)\s*\.\s*(?:contains|starts_with|ends_with)\s*\("),
        "prose_control_match",
    ),
)


def read_text(root: Path, relative: str) -> str | None:
    path = root / relative
    if not path.is_file():
        return None
    return path.read_text(encoding="utf-8")


def evaluate(root: Path) -> list[str]:
    findings: list[str] = []
    texts: dict[str, str] = {}
    for relative, tokens in REQUIRED_FILE_TOKENS.items():
        raw = read_text(root, relative)
        if raw is None:
            findings.append(f"missing_file:{relative}")
            continue
        texts[relative] = raw
        for token in tokens:
            if token not in raw:
                findings.append(f"missing_token:{relative}:{token}")

    production_files = (
        "crates/clawd/src/agent_hooks.rs",
        "crates/clawd/src/agent_hooks/command.rs",
    )
    for relative in production_files:
        production = texts.get(relative, "")
        for pattern, label in FORBIDDEN_PRODUCTION_PATTERNS:
            for match in pattern.finditer(production):
                line = production.count("\n", 0, match.start()) + 1
                findings.append(f"{label}:{relative}:{line}")

    production = texts.get("crates/clawd/src/agent_hooks.rs", "")
    stage_match = re.search(
        r"pub\(crate\) fn all\(\).*?&\[(.*?)\]",
        production,
        flags=re.DOTALL,
    )
    if stage_match is None or stage_match.group(1).count("Self::") != 11:
        findings.append("hook_stage_count_not_eleven")
    return findings


def write_fixture(root: Path) -> None:
    for relative, tokens in REQUIRED_FILE_TOKENS.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        body = "\n".join(tokens)
        if relative == "crates/clawd/src/agent_hooks.rs":
            body += "\npub(crate) fn all() { &[" + ",".join(f"Self::S{i}" for i in range(11)) + "] }\n"
        path.write_text(body + "\n", encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="agent-hook-contract-") as tmp:
        root = Path(tmp)
        write_fixture(root)
        findings = evaluate(root)
        if findings:
            print(f"SELF_TEST_FAIL positive findings={findings}")
            return 1

        production = root / "crates/clawd/src/agent_hooks/command.rs"
        production.write_text(
            production.read_text(encoding="utf-8")
            + '\nlet leaked = {"user_prompt": raw};\n'
            + "let child = tokio::process::Command::new(path);\n"
            + "let mode = ToolSandboxMode::DangerFull;\n",
            encoding="utf-8",
        )
        negative = evaluate(root)
        labels = {finding.split(":", 1)[0] for finding in negative}
        expected = {
            "semantic_payload_field",
            "direct_process_escape",
            "danger_full_hook_execution",
        }
        if not expected.issubset(labels):
            print(f"SELF_TEST_FAIL negative findings={negative}")
            return 1

    print("AGENT_HOOK_RUNTIME_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    findings = evaluate(ROOT)
    if findings:
        print(f"AGENT_HOOK_RUNTIME_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print("AGENT_HOOK_RUNTIME_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
