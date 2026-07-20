#!/usr/bin/env python3
"""Guard the planner-owned runtime against legacy pre-loop routing regressions."""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path

from planner_runtime_user_text_guards import (
    scan_async_job_start_user_text_command_selection,
    scan_config_change_preview_user_text_selection,
    scan_git_deterministic_user_text_action_selection,
    scan_runtime_surface_user_text_token_selection,
    scan_sqlite_route_request_semantic_fallback,
    scan_task_control_legacy_token_fallback,
    scan_task_control_task_id_user_text_selection,
    scan_web_search_user_text_query_selection,
)


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"

FORBIDDEN_CONTROL_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("legacy_migration_debt", re.compile(r"\blegacy_migration_debt\b")),
    ("legacy_semantic_reroute", re.compile(r"\blegacy_semantic_reroute\b")),
    ("agent_loop_semantic_defer", re.compile(r"\bagent_loop_semantic_defer\b")),
    (
        "post_route_semantic_clarify_deferred",
        re.compile(r"\bpost_route_semantic_clarify_deferred_to_agent_loop\b"),
    ),
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    return rel_path.endswith(("_tests.rs", "tests.rs")) or any(
        part == "tests" or part.endswith("_tests") for part in parts
    )


def production_rust_files() -> list[Path]:
    return sorted(
        path
        for path in SRC_ROOT.rglob("*.rs")
        if path.is_file() and not is_test_path(path)
    )


def scan_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in FORBIDDEN_CONTROL_PATTERNS:
            if pattern.search(line):
                findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_journal_plan_contract_boundary() -> list[Finding]:
    path = SRC_ROOT / "task_journal_decision_envelope.rs"
    text = path.read_text(encoding="utf-8")
    required = (
        "fn agent_loop_round_plan_contract_envelope_json(",
        "let output_contract = plan.output_contract.clone().unwrap_or_default();",
        "output_contract_ref(&output_contract)",
    )
    if all(token in text for token in required) and "output_contract_ref_for_route" not in text:
        return []
    return [
        Finding(
            rel(path),
            1,
            "journal_output_contract_not_planner_owned",
            "decision envelopes must consume PlanResult.output_contract directly",
        )
    ]


def scan_static_capability_compat_boundary() -> list[Finding]:
    paths = (
        SRC_ROOT / "capability_resolver.rs",
        SRC_ROOT / "capability_resolver_tests.rs",
        SRC_ROOT / "agent_engine" / "dispatch_support.rs",
    )
    forbidden_tokens = (
        "resolve_static_capability",
        "resolve_static_capability_action_for_state",
        "static_capability_compat_enabled",
        "static_capabilities",
        "capability_resolver_static_compat_resolved",
        '"static_compat"',
    )
    findings: list[Finding] = []
    for path in paths:
        text = path.read_text(encoding="utf-8")
        for line_no, line in enumerate(text.splitlines(), start=1):
            for token in forbidden_tokens:
                if token in line:
                    findings.append(
                        Finding(
                            rel(path),
                            line_no,
                            "static_capability_compat_forbidden",
                            line.strip(),
                        )
                    )
    return findings


def scan_removed_lightweight_preclassification() -> list[Finding]:
    targets = (
        ROOT / "prompts/layers/overlays/agent_planning.md",
        SRC_ROOT / "agent_engine/planning.rs",
        SRC_ROOT / "agent_engine.rs",
        ROOT / "prompts/layers/manifest.toml",
        SRC_ROOT / "bootstrap/prompts.rs",
    )
    forbidden_tokens = (
        "PlanningPromptClass",
        "build_lightweight_tool_spec",
        "lightweight_execution_prompt",
        "lightweight_incremental_plan_prompt",
    )
    findings: list[Finding] = []
    for path in targets:
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8")
        for token in forbidden_tokens:
            if token in text:
                findings.append(
                    Finding(
                        rel(path),
                        1,
                        "removed_lightweight_preclassification_present",
                        f"obsolete pre-planner classification token remains: {token}",
                    )
                )
    return findings

REQUIRED_PLANNER_FRONTDOOR_FILES: tuple[Path, ...] = (
    SRC_ROOT / "turn_boundary_envelope.rs",
    SRC_ROOT / "worker/ask_input.rs",
    SRC_ROOT / "worker/ask_execution_context.rs",
    SRC_ROOT / "worker/ask_planner_frontdoor.rs",
    SRC_ROOT / "worker/ask_runtime.rs",
)

REMOVED_SEMANTIC_FRONTDOOR_GLOBS: tuple[str, ...] = (
    "intent_router*.rs",
    "post_route_policy*.rs",
    "worker/ask_pipeline*.rs",
    "worker/ask_prepare*.rs",
    "agent_engine/migration_class*.rs",
    "agent_engine/action_route_locator_artifact.rs",
    "agent_engine/concrete_respond_structural_observation.rs",
    "agent_engine/config_guard_capability_repair.rs",
    "agent_engine/configured_command_prefix*.rs",
    "agent_engine/direct_observed_finalize_support.rs",
    "agent_engine/directory_entry_group_locator.rs",
    "agent_engine/directory_unique_entry.rs",
    "agent_engine/explicit_observed_paths.rs",
    "agent_engine/inline_transform_contract.rs",
    "agent_engine/legacy_file_config_capabilities.rs",
    "agent_engine/planning_followup.rs",
    "agent_engine/planning_path_metadata.rs",
    "agent_engine/planning_recent_artifacts.rs",
    "agent_engine/planning_registry_preference.rs",
    "agent_engine/planning_route_markers.rs",
    "agent_engine/planning_scalar_count_filter.rs",
    "agent_engine/planning_structured_field_exact.rs",
    "agent_engine/preferred_structured_action.rs",
    "agent_engine/read_range_action.rs",
    "agent_engine/runtime_status_scalar_plan.rs",
    "agent_engine/scalar_compare_observation.rs",
    "agent_engine/scalar_count_deterministic_plan.rs",
    "agent_engine/scalar_count_explicit_path.rs",
    "agent_engine/service_probe_contract.rs",
    "agent_engine/session_alias_target_coverage.rs",
    "agent_engine/shell_sequence_part.rs",
    "agent_engine/single_target_structured_field_rewrite.rs",
    "agent_engine/sqlite_table_listing_rewrite.rs",
    "agent_engine/structured_multi_field_read_rewrite.rs",
    "agent_engine/system_basic_action_path.rs",
    "agent_engine/value_string_list.rs",
)

REMOVED_SEMANTIC_RESOURCE_FILES: tuple[Path, ...] = (
    SRC_ROOT / "prompt_utils_contract_repair_judge.rs",
    SRC_ROOT / "prompt_utils_output_contract.rs",
    SRC_ROOT / "ask_flow_resume.rs",
    SRC_ROOT / "clarify_followup.rs",
    SRC_ROOT / "intent/continuation_resolver.rs",
    SRC_ROOT / "intent/resume_policy.rs",
    SRC_ROOT / "intent/safety_class.rs",
    SRC_ROOT / "runtime/ask_mode.rs",
    SRC_ROOT / "runtime/ask_mode_tests.rs",
    SRC_ROOT / "agent_engine/loop_control_executable_contract_observe.rs",
    SRC_ROOT / "agent_engine/skill_execution_preflight_executionless_tests.rs",
    ROOT / "prompts/layers/overlays/intent_normalizer_prompt.md",
    ROOT / "prompts/layers/overlays/contract_repair_judge_prompt.md",
    ROOT / "prompts/layers/overlays/resume_continue_execute_prompt.md",
    ROOT / "prompts/layers/overlays/resume_followup_discussion_prompt.md",
    ROOT / "prompts/schemas/intent_normalizer.schema.json",
    ROOT / "prompts/schemas/contract_repair_judge.schema.json",
    ROOT / "scripts/check_intent_normalizer_boundary_schema.py",
    ROOT / "scripts/runtime_semantic_rewrite_prompt_schema_guards.py",
)

REMOVED_SEMANTIC_PRODUCTION_TOKENS: tuple[str, ...] = (
    "AskMode",
    "ActFinalizeStyle",
    "RespondEntryStrategy",
    "RouteGateKind",
    "AskRouteTraceDecision",
    "route_trace_label_for_log",
    "route_trace_decision_for_journal",
    "uses_chat_finalizer",
    "is_execute_gate",
    "pure_chat_agent_loop_submode",
    "executionless_finalize_trace_plain",
    "executable_contract_preserved_for_agent_loop",
    "is_normalizer_schema_capability_bridge",
    "is_registry_capability_bridge",
    "service_status_deterministic_plan_result",
    "service_status_requests_system_basic_identity",
    "service_status_url_locator",
)

REMOVED_SEMANTIC_RESOURCE_TOKENS: tuple[tuple[Path, str], ...] = (
    (SRC_ROOT / "prompt_utils.rs", "IntentNormalizer"),
    (SRC_ROOT / "prompt_utils.rs", "ContractRepairJudge"),
    (ROOT / "crates/clawd/src/bootstrap/prompts.rs", "intent_normalizer_prompt.md"),
    (ROOT / "crates/clawd/src/bootstrap/prompts.rs", "contract_repair_judge_prompt.md"),
    (ROOT / "prompts/layers/manifest.toml", "intent_normalizer_prompt.md"),
    (ROOT / "prompts/layers/manifest.toml", "contract_repair_judge_prompt.md"),
    (ROOT / "crates/clawd/src/bootstrap/prompts.rs", "resume_continue_execute_prompt.md"),
    (ROOT / "crates/clawd/src/bootstrap/prompts.rs", "resume_followup_discussion_prompt.md"),
    (ROOT / "prompts/layers/manifest.toml", "resume_continue_execute_prompt.md"),
    (ROOT / "prompts/layers/manifest.toml", "resume_followup_discussion_prompt.md"),
    (SRC_ROOT / "schedule_service.rs", "schedule_context_for_normalizer"),
)


def removed_frontdoor_finding(path: Path, pattern: str) -> Finding:
    return Finding(
        rel(path),
        1,
        "removed_semantic_frontdoor_file_present",
        f"obsolete semantic frontdoor file matches {pattern}",
    )


def scan_removed_semantic_resources() -> list[Finding]:
    findings: list[Finding] = []
    for path in REMOVED_SEMANTIC_RESOURCE_FILES:
        if path.is_file():
            findings.append(
                Finding(
                    rel(path),
                    1,
                    "removed_semantic_resource_present",
                    "obsolete pre-planner semantic resource must stay deleted",
                )
            )
    for path, token in REMOVED_SEMANTIC_RESOURCE_TOKENS:
        if not path.is_file():
            continue
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            if token in line:
                findings.append(
                    Finding(
                        rel(path),
                        line_no,
                        "removed_semantic_resource_registered",
                        f"obsolete semantic resource token is registered: {token}",
                    )
                )
    return findings


def scan_removed_semantic_production_tokens() -> list[Finding]:
    findings: list[Finding] = []
    for path in production_rust_files():
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            for token in REMOVED_SEMANTIC_PRODUCTION_TOKENS:
                if token not in line:
                    continue
                findings.append(
                    Finding(
                        rel(path),
                        line_no,
                        "removed_semantic_production_token",
                        f"obsolete pre-loop route type must stay deleted: {token}",
                    )
                )
    return findings


def scan_planner_frontdoor_terminal_shape() -> list[Finding]:
    findings: list[Finding] = []
    for path in REQUIRED_PLANNER_FRONTDOOR_FILES:
        if path.is_file():
            continue
        findings.append(
            Finding(
                rel(path),
                1,
                "planner_frontdoor_file_missing",
                "required planner-owned frontdoor module is missing",
            )
        )

    for pattern in REMOVED_SEMANTIC_FRONTDOOR_GLOBS:
        for path in sorted(SRC_ROOT.glob(pattern)):
            if not path.is_file():
                continue
            findings.append(removed_frontdoor_finding(path, pattern))
    return findings


def scan_repo() -> list[Finding]:
    findings: list[Finding] = []
    for path in production_rust_files():
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        findings.extend(scan_text(rel_path, text))

    findings.extend(scan_planner_frontdoor_terminal_shape())
    findings.extend(scan_removed_semantic_resources())
    findings.extend(scan_removed_semantic_production_tokens())
    findings.extend(scan_removed_lightweight_preclassification())
    findings.extend(scan_journal_plan_contract_boundary())
    findings.extend(scan_static_capability_compat_boundary())
    findings.extend(scan_git_deterministic_user_text_action_selection())
    findings.extend(scan_sqlite_route_request_semantic_fallback())
    findings.extend(scan_task_control_task_id_user_text_selection())
    findings.extend(scan_task_control_legacy_token_fallback())
    findings.extend(scan_async_job_start_user_text_command_selection())
    findings.extend(scan_web_search_user_text_query_selection())
    findings.extend(scan_runtime_surface_user_text_token_selection())
    findings.extend(scan_config_change_preview_user_text_selection())
    return findings


def print_report(findings: list[Finding]) -> int:
    print(f"PLANNER_RUNTIME_BOUNDARY_CHECK findings={len(findings)}")
    for item in findings:
        print(f"  - {item.path}:{item.line} [{item.kind}] {item.text}")
    return 1 if findings else 0


def run_self_test() -> int:
    blocked = scan_text(
        "crates/clawd/src/agent_engine/planning.rs",
        '"decision_source": "legacy_migration_debt",\n',
    )
    assert blocked and blocked[0].kind == "legacy_migration_debt"

    blocked_file = removed_frontdoor_finding(
        SRC_ROOT / "intent_router_self_test.rs", "intent_router*.rs"
    )
    assert blocked_file.kind == "removed_semantic_frontdoor_file_present"
    assert blocked_file.path.endswith("intent_router_self_test.rs")

    assert not scan_planner_frontdoor_terminal_shape()
    assert not scan_removed_semantic_resources()
    assert not scan_removed_semantic_production_tokens()
    assert not scan_removed_lightweight_preclassification()
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    return print_report(scan_repo())


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
