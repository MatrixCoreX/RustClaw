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
        "mod http",
        "mod mcp",
        "mod runtime",
        "mod shared",
        "lifecycle_stage_outcome_for_state",
        "pre_tool_use_outcome_for_state",
        "default_pre_tool_use_outcome",
        "HookEvaluation",
        "machine_observations",
        "merge_hook_decision",
        "PolicyDecision::parse_token",
    ),
    "crates/clawd/src/agent_hooks/shared.rs": (
        "HOOK_OUTPUT_SCHEMA_VERSION",
        "HookHandlerConfig",
        "#[serde(default, deny_unknown_fields)]",
        "#[serde(deny_unknown_fields)]",
        "validate_common_handler",
        "parse_handler_output_value",
        "hook_async_decision_forbidden",
        "hook_handler_blocking_stage_invalid",
        "lifecycle_hook_event",
        "lifecycle_metadata_key_allowed",
        "sanitize_lifecycle_metadata_value",
        '"argument_fields"',
        '"argument_byte_count"',
        '"handler_id"',
        '"failure_policy"',
        '"trust_status"',
        '"content_sha256"',
        '"output_truncated"',
    ),
    "crates/clawd/src/agent_hooks/runtime.rs": (
        "load_hook_configuration",
        "lifecycle_stage_outcome_for_state",
        "evaluate_loaded_handlers",
        "lifecycle_reason_code",
        "run_command_handler",
        "run_http_handler",
        "run_mcp_handler",
        "record_validation_failure",
        "merge_hook_decision",
    ),
    "crates/clawd/src/agent_hooks/command.rs": (
        "validate_command_handler",
        "hook_handler_hash_mismatch",
        "ToolSandboxMode::ReadOnly",
        "ProcessNetworkPolicy::Deny",
        ".env_clear()",
        "CancellationToken",
        "hook_handler_timeout",
        "hook_handler_output_too_large",
    ),
    "crates/clawd/src/agent_hooks/http.rs": (
        "validate_http_handler",
        "redirect(Policy::none())",
        "hook_http_https_required",
        "allow_insecure_loopback",
        "is_literal_loopback_host",
        "is_env_reference",
        "hook_http_redirect_forbidden",
        "hook_handler_output_too_large",
        "max_attempts",
        "CancellationToken",
    ),
    "crates/clawd/src/agent_hooks/mcp.rs": (
        "validate_mcp_handler",
        'matches!(policy.effect.as_str(), "observe" | "validate")',
        'policy.risk_level != "low"',
        "policy.idempotent",
        "structured_content",
        "parse_handler_output_value",
        "hook_mcp_policy_unsafe",
        "hook_mcp_structured_output_required",
        "CancellationToken",
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
        "record_permission_request_hook",
        "record_post_tool_use_hook_observations",
        "record_subagent_hook_stage",
    ),
    "crates/clawd/src/agent_engine/loop_control.rs": (
        "record_session_start_hooks",
        "HookStage::SessionStart",
        "HookStage::UserPromptSubmit",
        "lifecycle_stage_outcome_for_state",
    ),
    "crates/clawd/src/worker/ask_execution_context.rs": (
        "plan_agent_loop_context_compaction",
        "HookStage::PreCompact",
        "apply_agent_loop_context_compaction",
        "HookStage::PostCompact",
        "initial_task_observations",
        "machine_observations",
    ),
    "crates/clawd/src/agent_engine/skill_execution_observations.rs": (
        "record_hook_evaluation_observation",
        "record_permission_request_hook",
        "record_post_tool_use_hook_observations",
        "HookStage::PermissionRequest",
        "HookStage::PostToolUse",
        "machine_observations",
    ),
    "crates/clawd/src/agent_engine/skill_execution_subagent.rs": (
        "record_subagent_hook_stage",
        "lifecycle_stage_outcome_for_state",
        'machine_observations("subagent")',
    ),
    "crates/clawd/src/agent_engine.rs": (
        "agent_loop.verifier_permission_request",
        "HookStage::PermissionRequest",
        "confirmation_can_proceed",
        '"agent_hook_events"',
        '"permission_hook_decision"',
    ),
    "crates/clawd/src/finalize/journal.rs": (
        "build_terminal_from_loop_state",
        "HookStage::Stop",
        "HookStage::SessionEnd",
        "lifecycle_stage_outcome_for_state",
    ),
    "crates/clawd/src/finalize/loop_reply.rs": (
        "build_terminal_from_loop_state as build_loop_journal",
        ".await",
    ),
    "crates/clawd/src/finalize/loop_reply_config_edit.rs": (
        "agent_hook_runtime_surface_answer",
        '"agent.hooks.handlers"',
        "PolicyDecision::all_tokens()",
        "HookStage::all()",
    ),
    "crates/clawd/src/finalize/loop_reply_machine_kv.rs": (
        "current_delivery_contains_agent_hook_runtime_surface",
        '"agent.hooks.handlers"',
        '"hook_stages"',
        '"hook_decisions"',
    ),
    "crates/clawd/src/finalize/loop_reply_synthesis_preference.rs": (
        "agent_hook_runtime_surface_payload_is_publishable",
        'Some("agent_hooks_runtime_surface")',
        'Some("agent.hooks.handlers")',
        "PolicyDecision::all_tokens()",
        "HookStage::all()",
    ),
    "crates/clawd/src/finalize/journal_tests.rs": (
        "terminal_builder_executes_stop_and_session_end_at_real_owner",
    ),
    "crates/clawcli/src/events.rs": (
        '"handler_id"',
        '"handler_kind"',
        '"trust_status"',
        '"failure_policy"',
        '"output_truncated"',
    ),
    "crates/clawcli/src/events_tests.rs": (
        "agent_hook_events_preserve_handler_execution_fields",
    ),
    "UI/src/lib/task-result.ts": (
        'eventType === "agent_hook"',
        '"handler_id"',
        '"handler_kind"',
        '"trust_status"',
        '"failure_policy"',
        '"output_truncated"',
    ),
    "UI/src/lib/task-result.test.ts": (
        "projects agent hook execution fields for teaching traces",
    ),
    "configs/agent_guard.toml": (
        "[[agent.hooks.handlers]]",
        'kind = "command"',
        'kind = "http"',
        'kind = "mcp"',
        "content_sha256",
        "auth_token_env",
        "allow_insecure_loopback",
        "event_argument",
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
        "lifecycle_event_drops_semantic_and_secret_metadata_fields",
        "blocking_handler_is_limited_to_decision_capable_stages",
    ),
    "crates/clawd/src/agent_engine/skill_execution_hook_policy_tests.rs": (
        "trusted_command_hook_blocks_through_production_pre_tool_path",
        "configured_post_tool_hook_runs_through_production_owner",
        "configured_permission_hook_can_deny_at_production_owner",
        "verifier_confirmation_runs_permission_hook_before_approval_creation",
        'handler["trust_status"]',
        'handler["content_sha256"]',
    ),
    "crates/clawd/src/agent_hooks_transport_tests.rs": (
        "trusted_loopback_http_hook_retries_and_returns_machine_decision",
        "http_hook_rejects_external_plaintext_and_redirects",
        "trusted_observation_only_mcp_hook_uses_structured_content_only",
        "mcp_hook_rejects_unavailable_or_unsafe_capabilities",
    ),
    "crates/clawd/tests/fixtures/mcp_stdio_fixture.py": (
        "hook_decision",
        '"structuredContent"',
        '"fixture_mcp_denied"',
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

LEGACY_FIXED_POLICY_TOKENS = (
    "agent.hooks.blocked_action_refs",
    "agent.hooks.blocked_tools",
    "agent.hooks.require_confirmation_action_refs",
    "agent.hooks.background_wait_action_refs",
)

LEGACY_FIXED_POLICY_SCAN_FILES = (
    "AGENTS.md",
    "configs/agent_guard.toml",
    "crates/clawd/src/agent_hooks.rs",
    "crates/clawd/src/agent_hooks/runtime.rs",
    "crates/clawd/src/agent_hooks/shared.rs",
    "crates/clawd/src/finalize/loop_reply_config_edit.rs",
    "crates/clawd/src/finalize/loop_reply_machine_kv.rs",
    "crates/clawd/src/finalize/loop_reply_synthesis_preference.rs",
    "prompts/layers/overlays/agent_tool_spec.md",
    "prompts/layers/overlays/loop_incremental_plan_prompt.md",
    "prompts/layers/overlays/single_plan_execution_prompt.md",
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

    production_files = tuple(
        relative
        for relative in texts
        if relative.startswith("crates/clawd/src/agent_hooks")
        and not relative.endswith(("_tests.rs", "tests.rs"))
    )
    for relative in production_files:
        production = texts.get(relative, "")
        for pattern, label in FORBIDDEN_PRODUCTION_PATTERNS:
            for match in pattern.finditer(production):
                line = production.count("\n", 0, match.start()) + 1
                findings.append(f"{label}:{relative}:{line}")

    for relative in LEGACY_FIXED_POLICY_SCAN_FILES:
        raw = read_text(root, relative)
        if raw is None:
            continue
        for token in LEGACY_FIXED_POLICY_TOKENS:
            if token in raw:
                findings.append(f"legacy_fixed_hook_policy_token:{relative}:{token}")

    for legacy_identifier in ("HookPolicy", "evaluate_pre_tool_use", "toml_string_array"):
        for relative in production_files:
            if legacy_identifier in texts.get(relative, ""):
                findings.append(
                    f"legacy_fixed_hook_policy_identifier:{relative}:{legacy_identifier}"
                )

    production = texts.get("crates/clawd/src/agent_hooks.rs", "")
    stage_match = re.search(
        r"pub\(crate\) fn all\(\).*?&\[(.*?)\]",
        production,
        flags=re.DOTALL,
    )
    if stage_match is None or stage_match.group(1).count("Self::") != 11:
        findings.append("hook_stage_count_not_eleven")
    journal = texts.get("crates/clawd/src/finalize/journal.rs", "")
    for legacy_terminal_constructor in ("stop_outcome(", "session_end_outcome("):
        if legacy_terminal_constructor in journal:
            findings.append(
                f"synthetic_terminal_hook_in_journal:{legacy_terminal_constructor}"
            )
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

        hooks = root / "crates/clawd/src/agent_hooks.rs"
        hooks.write_text(
            hooks.read_text(encoding="utf-8")
            + '\nconst LEGACY: &str = "agent.hooks.blocked_tools";\n',
            encoding="utf-8",
        )
        legacy_negative = evaluate(root)
        if not any(
            finding.startswith("legacy_fixed_hook_policy_token:")
            for finding in legacy_negative
        ):
            print(f"SELF_TEST_FAIL legacy findings={legacy_negative}")
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
