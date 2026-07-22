#!/usr/bin/env python3
"""Guard the generic MCP runtime architecture and machine-only boundaries."""

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_FILE_TOKENS = {
    "Cargo.toml": (
        'rmcp = { version = "2.2.0"',
        'default-features = false',
        '"auth"',
        '"client"',
        '"transport-child-process"',
        '"transport-streamable-http-client-reqwest"',
    ),
    "crates/claw-core/src/config.rs": (
        "pub struct McpConfig",
        "planner_visible_tools",
        "catalog_search_max_results",
        "pub struct McpServerConfig",
        "auth_token_env",
        "oauth_client_id_env",
        "oauth_client_secret_env",
        "oauth_scopes",
        "oauth_resource",
        "trusted",
        "allowed_tools",
        "tool_policies",
        "max_concurrency",
        "max_output_bytes",
        "max_schema_bytes",
        "health_check_seconds",
        "reconnect_base_seconds",
        "reconnect_max_seconds",
    ),
    "crates/clawd/src/mcp_runtime/client.rs": (
        "pub(crate) trait McpClient",
        "PeerRequestOptions",
        "send_cancellable_request",
        "notify_cancelled",
        "is_transport_closed",
        "close_with_timeout",
    ),
    "crates/clawd/src/mcp_runtime/manager.rs": (
        "TokioChildProcess",
        "StreamableHttpClientTransportConfig",
        "OAuthState",
        "ClientCredentialsConfig",
        "AuthClient",
        "reinit_on_expired_session(true)",
        "spawn_health_monitor",
        "schedule_reconnect",
        "MCP_CATALOG_SEARCH_CAPABILITY",
        "planner_tools",
        "search_catalog",
        "validate_input_schema",
        "project_call_result",
    ),
    "crates/clawd/src/mcp_runtime/types.rs": (
        "pub(crate) enum McpLifecycleState",
        "pub(crate) struct McpToolPolicy",
        "permission_policy_json",
        "pub(crate) struct McpCallOutcome",
        "structured_content",
        "error_code",
    ),
    "crates/clawd/src/capability_map.rs": (
        ".mcp_planner_tools()",
        "build_capability_map_for_task",
    ),
    "crates/clawd/src/agent_engine/skill_execution_preflight.rs": (
        "state.mcp_tool(canonical_skill)",
        "permission_policy_json",
        "PolicyDecision::from_permission_flags",
    ),
    "crates/clawd/src/agent_engine/skill_execution.rs": (
        "run_mcp_tool_observation",
        "task_cancellation_token",
        '"mcp_result"',
        "record_mcp_tool_execution_observation",
    ),
    "crates/clawd/src/agent_engine/skill_execution_observations.rs": (
        "record_mcp_tool_execution_observation",
        '"mcp.tool_call"',
        '"owner_layer": "mcp_runtime"',
        '"lifecycle_state"',
        '"policy_decision"',
        '"latency_ms"',
        '"output_bytes"',
        '"truncated"',
        '"error_code"',
    ),
    "crates/clawd/src/task_journal_event_stream.rs": (
        'Some("mcp_runtime")',
        '"mcp_tool_call"',
    ),
    "crates/clawd/src/mcp_admin_routes.rs": (
        "require_mcp_admin",
        "mcp_lifecycle_snapshots",
        "probe_mcp_server",
        "get_mcp_config",
        "update_mcp_config",
        "validate_configuration",
        "auth_token_env",
        "oauth_client_secret_env",
        "env_refs",
        "write_mcp_config",
    ),
    "crates/clawcli/src/commands/mcp.rs": (
        '"/admin/mcp/servers"',
        "/admin/mcp/tools",
        '.push("test")',
    ),
    "crates/clawd/src/mcp_runtime/tests.rs": (
        "stdio_runtime_discovers_paginated_tools_calls_bounds_and_stops",
        "streamable_http_runtime_initializes_discovers_and_calls",
        "large_catalog_uses_bounded_search_then_discloses_matching_schema",
        "health_tick_reconnects_closed_transport_without_replaying_a_tool",
        "duplicate_namespaces_fail_closed_before_connecting",
        "duplicate_tool_failure_cleans_up_stdio_process",
        "mutating_mcp_tool_requires_shared_permission_confirmation",
    ),
    "crates/clawd/src/agent_engine/skill_execution_mcp_tests.rs": (
        "mcp_execution_records_auditable_teaching_observation",
        "mcp_transport_failure_records_machine_error_without_raw_detail",
    ),
    "crates/clawd/src/mcp_runtime/agent_loop_tests.rs": (
        "ordinary_agent_loop_executes_safe_mcp_capability_with_event_evidence",
        '"call_capability"',
        '"mcp_tool_call"',
        "mcp.tool_call",
    ),
    "crates/clawd/src/mcp_admin_routes_tests.rs": (
        "config_update_preserves_unmanaged_fields_and_redacts_static_environment",
        "config_update_rejects_literal_secret_reference_and_duplicate_server_ids",
        "config_writer_preserves_distinct_workspace_and_mounted_content",
    ),
    "UI/src/components/McpConfigSection.tsx": (
        "Environment references (optional)",
        "Token environment variable",
        "Test protocol",
        "hasAdvancedPolicy",
    ),
    "UI/src/lib/mcp-config.test.ts": (
        "saves only secret reference names",
        "rejects incomplete lines",
    ),
    "crates/clawcli/tests/mcp_commands.rs": (
        "mcp_commands_use_authenticated_machine_endpoints",
    ),
}

FORBIDDEN_PRODUCTION_PATTERNS = (
    (re.compile(r'"jsonrpc"\s*:'), "handwritten_jsonrpc"),
    (re.compile(r"\berror_text\b"), "natural_language_error_control"),
    (
        re.compile(
            r"\b(?:text|content|description)\s*\.\s*"
            r"(?:contains|starts_with|ends_with)\s*\("
        ),
        "provider_prose_control_match",
    ),
)


def read_text(root: Path, relative: str) -> str | None:
    path = root / relative
    if not path.is_file():
        return None
    return path.read_text(encoding="utf-8")


def production_mcp_files(root: Path) -> list[Path]:
    source = root / "crates/clawd/src/mcp_runtime"
    if not source.is_dir():
        return []
    return sorted(
        path
        for path in source.glob("*.rs")
        if path.name not in {"tests.rs", "test_support.rs"}
        and not path.name.endswith("_tests.rs")
    )


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

    manager_path = "crates/clawd/src/mcp_runtime/manager.rs"
    manager = texts.get(manager_path, "")
    call_count = manager.count(".call_tool(")
    if call_count != 1:
        findings.append(f"unsafe_tool_call_site_count:{manager_path}:{call_count}")

    admin_path = "crates/clawd/src/mcp_admin_routes.rs"
    admin = texts.get(admin_path, "")
    if ".call_tool(" in admin or ".mcp_runtime.call(" in admin:
        findings.append(f"admin_probe_executes_tool:{admin_path}")

    for path in production_mcp_files(root):
        raw = path.read_text(encoding="utf-8")
        relative = path.relative_to(root).as_posix()
        for pattern, label in FORBIDDEN_PRODUCTION_PATTERNS:
            for match in pattern.finditer(raw):
                line = raw.count("\n", 0, match.start()) + 1
                findings.append(f"{label}:{relative}:{line}")
    return findings


def write_fixture(root: Path) -> None:
    for relative, tokens in REQUIRED_FILE_TOKENS.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        body = "\n".join(tokens)
        if relative == "crates/clawd/src/mcp_runtime/manager.rs":
            body += "\nclient.call_tool(\n"
        path.write_text(body + "\n", encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="mcp-runtime-contract-") as tmp:
        root = Path(tmp)
        write_fixture(root)
        findings = evaluate(root)
        if findings:
            print(f"SELF_TEST_FAIL positive findings={findings}")
            return 1

        client = root / "crates/clawd/src/mcp_runtime/client.rs"
        client.write_text(
            client.read_text(encoding="utf-8")
            + '\nlet request = json!({"jsonrpc": "2.0"});\n'
            + "if content.contains(\"success\") {}\n",
            encoding="utf-8",
        )
        admin = root / "crates/clawd/src/mcp_admin_routes.rs"
        admin.write_text(
            admin.read_text(encoding="utf-8") + "\nruntime.call_tool(\n",
            encoding="utf-8",
        )
        manager = root / "crates/clawd/src/mcp_runtime/manager.rs"
        manager.write_text(
            manager.read_text(encoding="utf-8").replace(
                "reinit_on_expired_session(true)", "session_reinit_removed"
            ),
            encoding="utf-8",
        )
        findings = evaluate(root)
        expected_labels = {
            "handwritten_jsonrpc",
            "provider_prose_control_match",
            "admin_probe_executes_tool",
            "missing_token",
        }
        observed_labels = {finding.split(":", 1)[0] for finding in findings}
        if not expected_labels.issubset(observed_labels):
            print(f"SELF_TEST_FAIL negative findings={findings}")
            return 1
    print("MCP_RUNTIME_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = evaluate(ROOT)
    if findings:
        print(f"MCP_RUNTIME_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print("MCP_RUNTIME_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
