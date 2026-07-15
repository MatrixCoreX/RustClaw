#!/usr/bin/env python3
"""Guard runtime semantic rewrites do not return after agent-loop migration.

RustClaw's target is that ordinary semantic decisions live in the planner /
agent loop. Production runtime must not reintroduce legacy semantic rewrite
sources or migration-debt control markers.
"""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path

from runtime_semantic_rewrite_boundary_marker_guards import (
    scan_auto_locator_binding_marker_typing,
    scan_auto_locator_binding_marker_typing_text,
    scan_background_locator_loop_recovery_marker_typing,
    scan_background_locator_loop_recovery_marker_typing_text,
    scan_boundary_preflight_deferral_typing,
    scan_boundary_preflight_deferral_typing_text,
    scan_contract_repair_loop_observation_boundary,
    scan_contract_repair_loop_observation_boundary_text,
    scan_default_config_contract_deferral_typing,
    scan_default_config_contract_deferral_typing_text,
    scan_execution_context_sanitization_typing,
    scan_execution_context_sanitization_typing_text,
    scan_file_delivery_boundary_deferral_typing,
    scan_file_delivery_boundary_deferral_typing_text,
    scan_post_route_boundary_candidate_typing,
    scan_post_route_boundary_candidate_typing_text,
    scan_structured_anchor_evidence_marker_typing,
    scan_structured_anchor_evidence_marker_typing_text,
    scan_subagent_boundary_deferral_helper,
    scan_subagent_boundary_deferral_helper_text,
    scan_worker_loop_boundary_deferral_typing,
    scan_worker_loop_boundary_deferral_typing_text,
    scan_worker_route_marker_typing,
    scan_worker_route_marker_typing_text,
)
from runtime_semantic_rewrite_prompt_schema_guards import (
    FORBIDDEN_PROMPT_ORDINARY_SEMANTIC_TOKENS,
    scan_ask_mode_route_trace_label_tokens,
    scan_ask_mode_route_trace_label_tokens_text,
    scan_boundary_envelope_rust_type_machine_only,
    scan_boundary_envelope_rust_type_text,
    scan_boundary_envelope_schema_json,
    scan_boundary_envelope_schema_machine_only,
    scan_boundary_prompt_schema_legacy_semantic_kind_fields,
    scan_boundary_semantic_kind_text,
    scan_china_model_routing_patch_boundaries,
    scan_china_model_routing_patch_boundaries_text,
    scan_contract_repair_schema_ordinary_semantic_tokens,
    scan_first_layer_decision_test_only_boundary,
    scan_first_layer_decision_test_only_boundary_text,
    scan_intent_normalizer_legacy_decision_field_deleted,
    scan_intent_normalizer_legacy_decision_field_deleted_text,
    scan_intent_normalizer_prompt_contract_marker,
    scan_intent_normalizer_schema_ordinary_semantic_tokens,
    scan_intent_normalizer_schema_route_authority_fields,
    scan_intent_normalizer_schema_route_authority_json,
    scan_legacy_route_trace_reason_tokens,
    scan_legacy_route_trace_reason_tokens_text,
    scan_normalizer_route_trace_label_tokens,
    scan_normalizer_route_trace_label_tokens_text,
    scan_normalizer_run_route_trace_decision_type,
    scan_normalizer_run_route_trace_decision_type_text,
    scan_planner_prompt_legacy_semantic_kind_keys,
    scan_planner_prompt_legacy_semantic_kind_keys_text,
    scan_prompt_layer_ordinary_semantic_tokens,
    scan_prompt_layer_text,
    scan_route_trace_record_decision_type,
    scan_route_trace_record_decision_type_text,
    scan_runtime_journal_route_trace_decision_type,
    scan_runtime_journal_route_trace_decision_type_text,
    scan_schema_text,
    scan_skill_registry_metadata_ordinary_semantic_tokens,
)
from runtime_semantic_rewrite_registry_bridge_guards import (
    ANSWER_VERIFIER_FILE,
    ASK_PREPARE_FILE,
    FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS,
    FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
    INTENT_ROUTER_CONTRACT_HINT_FILE,
    INTENT_ROUTER_EXECUTION_CONTRACT_FILE,
    INTENT_ROUTER_OBSERVATION_REPAIR_FILE,
    MIGRATION_CLASS_FILE,
    PREFERRED_STRUCTURED_ACTION_FILE,
    PROMPT_UTILS_CONTRACT_REPAIR_JUDGE_FILE,
    PROMPT_UTILS_OUTPUT_CONTRACT_FILE,
    TASK_CONTEXT_BUILDER_FILE,
    TASK_CONTRACT_FILE,
    TASK_JOURNAL_EVIDENCE_COVERAGE_FILE,
    scan_answer_verifier_output_contract_prompt_marker,
    scan_answer_verifier_registry_bridge_markers,
    scan_ask_prepare_registry_bridge_marker_preservation,
    scan_binding_repair_registry_bridge_markers,
    scan_contract_hint_registry_bridge_semantic_markers,
    scan_contract_matrix_registry_bridge_bypass,
    scan_contract_matrix_trace_contract_marker,
    scan_current_workspace_scope_boundary_marker,
    scan_current_workspace_scope_legacy_semantic_marker_removed,
    scan_dry_run_contract_plan_marker_payloads,
    scan_execution_contract_registry_bridge_repairs,
    scan_execution_recipe_contract_marker_outputs,
    scan_execution_recipe_registry_bridge_tokens,
    scan_finalizer_observed_output_registry_bridge_markers,
    scan_intent_router_output_contract_schema_marker_only,
    scan_lightweight_tool_spec_contract_marker,
    scan_loop_control_output_contract_marker_key,
    scan_loop_recovery_contract_marker_fields,
    scan_migration_class_registry_bridge_fallback,
    scan_observation_repair_registry_bridge_markers,
    scan_observed_output_contract_marker_payload,
    scan_pre_route_repair_marker_allowlist_text,
    scan_pre_route_repair_marker_allowlists,
    scan_preferred_run_cmd_registry_bridge_fallback,
    scan_preferred_run_cmd_registry_bridge_text,
    scan_preferred_structured_action_registry_bridge_fallback,
    scan_prompt_utils_contract_repair_judge_marker_only,
    scan_prompt_utils_contract_repair_judge_marker_only_text,
    scan_prompt_utils_output_contract_marker_only,
    scan_prompt_utils_output_contract_registry_bridge_tokens,
    scan_route_guard_record_contract_marker,
    scan_runtime_status_recipe_contract_marker,
    scan_schedule_preview_contract_marker,
    scan_schema_report_contract_marker_fields,
    scan_task_context_builder_registry_bridge_budget,
    scan_task_contract_registry_bridge_semantic_defaults,
    scan_task_journal_step_contract_marker,
    scan_task_journal_evidence_registry_bridge_markers,
    scan_token_list_text,
    scan_verifier_contract_missing_detail_marker,
)
from runtime_semantic_rewrite_user_text_guards import (
    scan_async_job_start_user_text_command_selection,
    scan_async_job_start_user_text_command_text,
    scan_config_change_preview_user_text_selection,
    scan_config_change_preview_user_text_selection_text,
    scan_git_deterministic_text,
    scan_git_deterministic_user_text_action_selection,
    scan_runtime_surface_user_text_token_selection,
    scan_runtime_surface_user_text_token_text,
    scan_service_status_identity_text,
    scan_service_status_identity_user_text_selection,
    scan_service_status_process_text,
    scan_service_status_process_user_text_selection,
    scan_service_status_scalar_shape_health_selection,
    scan_service_status_scalar_shape_health_text,
    scan_service_status_url_text,
    scan_service_status_url_user_text_selection,
    scan_service_status_workspace_product_text,
    scan_service_status_workspace_product_text_selection,
    scan_sqlite_route_request_semantic_fallback,
    scan_sqlite_route_request_text,
    scan_task_control_legacy_token_fallback,
    scan_task_control_legacy_token_text,
    scan_task_control_task_id_user_text,
    scan_task_control_task_id_user_text_selection,
    scan_web_search_user_text_query_selection,
    scan_web_search_user_text_query_text,
)


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
MAIN_FILE = SRC_ROOT / "main.rs"
AGENT_ENGINE_FILE = SRC_ROOT / "agent_engine.rs"
PIPELINE_TYPES_FILE = SRC_ROOT / "pipeline_types.rs"
RUNTIME_ASK_MODE_FILE = SRC_ROOT / "runtime/ask_mode.rs"
RUNTIME_TYPES_FILE = SRC_ROOT / "runtime/types.rs"
INTENT_ROUTER_FILE = SRC_ROOT / "intent_router.rs"
INTENT_ROUTER_CONTRACT_REPAIR_JUDGE_FILE = (
    SRC_ROOT / "intent_router_contract_repair_judge.rs"
)
ASK_PIPELINE_FILE = SRC_ROOT / "worker/ask_pipeline.rs"
TASK_JOURNAL_FILE = SRC_ROOT / "task_journal.rs"
INTENT_ROUTER_PROMPT_RENDER_FILE = SRC_ROOT / "intent_router_prompt_render.rs"
INTENT_ROUTER_OUTPUT_TYPES_FILE = SRC_ROOT / "intent_router_output_types.rs"
INTENT_ROUTER_ROUTE_TRACE_FILE = SRC_ROOT / "intent_router_route_trace.rs"
INTENT_ROUTER_NORMALIZER_RUN_FILE = SRC_ROOT / "intent_router_normalizer_run.rs"
VALUE_STRING_LIST_FILE = SRC_ROOT / "agent_engine/value_string_list.rs"
RUNTIME_SURFACE_PLAN_FILE = SRC_ROOT / "agent_engine/runtime_surface_plan.rs"
READ_RANGE_ACTION_FILE = SRC_ROOT / "agent_engine/read_range_action.rs"
SINGLE_TARGET_STRUCTURED_FIELD_REWRITE_FILE = (
    SRC_ROOT / "agent_engine/single_target_structured_field_rewrite.rs"
)

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("semantic_rewrite", re.compile(r"\bsemantic_rewrite\b")),
    ("legacy_migration_debt", re.compile(r"\blegacy_migration_debt\b")),
    ("legacy_semantic_reroute", re.compile(r"\blegacy_semantic_reroute\b")),
    ("agent_loop_semantic_defer", re.compile(r"\bagent_loop_semantic_defer\b")),
    (
        "post_route_semantic_clarify_deferred",
        re.compile(r"\bpost_route_semantic_clarify_deferred_to_agent_loop\b"),
    ),
)

ROUTE_RESULT_RAW_SEMANTIC_ACCESS = re.compile(
    r"\b(?:route|route_result|execution_route_result)\.output_contract\.semantic_kind\b"
)
ROUTE_RESULT_RAW_SEMANTIC_CLEAR = re.compile(
    r"\b(?:route|route_result|execution_route_result)\.output_contract\.semantic_kind"
    r"\s*=\s*(?:crate::)?OutputSemanticKind::None\b"
)
LEGACY_JSON_SEMANTIC_FIELD_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r'"semantic_kind"\s*:'),
    re.compile(r'\\"semantic_kind\\"\s*:'),
    re.compile(r'\.get\("semantic_kind"\)'),
    re.compile(r'contains_key\("semantic_kind"\)'),
    re.compile(r'\.pointer\("/semantic_kind"\)'),
    re.compile(r'"semantic_kind"\.to_string\(\)'),
)
LEGACY_RUNTIME_SEMANTIC_OUTPUT_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("legacy_semantic_kv_output", re.compile(r'"(?:contract_)?semantic_kind[=:]')),
    ("legacy_semantic_trace_label", re.compile(r'"[^"]*\bsemantic[=:]')),
    ("legacy_semantic_colon_output", re.compile(r'"semantic_kind:\s')),
    ("legacy_semantic_prompt_instruction", re.compile(r"\bSet\s+semantic_kind\b")),
    ("legacy_expected_semantic_fact", re.compile(r"expected_semantic_kind:")),
)

ALLOWED_PRODUCTION_FILES: set[str] = set()

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
    if rel_path.endswith(("_tests.rs", "tests.rs")):
        return True
    return any(part == "tests" or part.endswith("_tests") for part in parts)


def production_rust_files() -> list[Path]:
    return sorted(
        path
        for path in SRC_ROOT.rglob("*.rs")
        if path.is_file() and not is_test_path(path)
    )


def finding_allowed(rel_path: str) -> bool:
    return rel_path in ALLOWED_PRODUCTION_FILES


def scan_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in FORBIDDEN_PATTERNS:
            if not pattern.search(line):
                continue
            if finding_allowed(rel_path):
                continue
            findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_repo() -> list[Finding]:
    findings: list[Finding] = []
    for path in production_rust_files():
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        findings.extend(scan_text(rel_path, text))
        findings.extend(scan_route_result_raw_semantic_access(rel_path, text))
        findings.extend(scan_legacy_json_semantic_fields(rel_path, text))
        findings.extend(scan_legacy_runtime_semantic_outputs(rel_path, text))
    findings.extend(scan_normalizer_route_result_boundary())
    findings.extend(scan_journal_output_contract_ref_boundary())
    findings.extend(scan_static_capability_compat_boundary())
    findings.extend(scan_contract_repair_judge_boundary())
    findings.extend(scan_contract_repair_loop_observation_boundary())
    findings.extend(scan_boundary_preflight_deferral_typing())
    findings.extend(scan_worker_loop_boundary_deferral_typing())
    findings.extend(scan_worker_route_marker_typing())
    findings.extend(scan_background_locator_loop_recovery_marker_typing())
    findings.extend(scan_structured_anchor_evidence_marker_typing())
    findings.extend(scan_subagent_boundary_deferral_helper())
    findings.extend(scan_file_delivery_boundary_deferral_typing())
    findings.extend(scan_default_config_contract_deferral_typing())
    findings.extend(scan_execution_context_sanitization_typing())
    findings.extend(scan_auto_locator_binding_marker_typing())
    findings.extend(scan_post_route_boundary_candidate_typing())
    findings.extend(scan_prompt_layer_ordinary_semantic_tokens())
    findings.extend(scan_planner_prompt_legacy_semantic_kind_keys())
    findings.extend(scan_intent_normalizer_prompt_contract_marker())
    findings.extend(scan_china_model_routing_patch_boundaries())
    findings.extend(scan_boundary_prompt_schema_legacy_semantic_kind_fields())
    findings.extend(scan_intent_normalizer_schema_ordinary_semantic_tokens())
    findings.extend(scan_intent_normalizer_schema_route_authority_fields())
    findings.extend(scan_boundary_envelope_schema_machine_only())
    findings.extend(scan_boundary_envelope_rust_type_machine_only())
    findings.extend(scan_route_trace_record_decision_type())
    findings.extend(scan_normalizer_run_route_trace_decision_type())
    findings.extend(scan_normalizer_route_trace_label_tokens())
    findings.extend(scan_runtime_journal_route_trace_decision_type())
    findings.extend(scan_ask_mode_route_trace_label_tokens())
    findings.extend(scan_first_layer_decision_test_only_boundary())
    findings.extend(scan_intent_normalizer_legacy_decision_field_deleted())
    findings.extend(scan_legacy_route_trace_reason_tokens())
    findings.extend(scan_contract_repair_schema_ordinary_semantic_tokens())
    findings.extend(scan_skill_registry_metadata_ordinary_semantic_tokens())
    findings.extend(scan_preferred_run_cmd_registry_bridge_fallback())
    findings.extend(scan_preferred_structured_action_registry_bridge_fallback())
    findings.extend(scan_migration_class_registry_bridge_fallback())
    findings.extend(scan_ask_prepare_registry_bridge_marker_preservation())
    findings.extend(scan_current_workspace_scope_boundary_marker())
    findings.extend(scan_lightweight_tool_spec_contract_marker())
    findings.extend(scan_task_journal_evidence_registry_bridge_markers())
    findings.extend(scan_observation_repair_registry_bridge_markers())
    findings.extend(scan_contract_hint_registry_bridge_semantic_markers())
    findings.extend(scan_execution_contract_registry_bridge_repairs())
    findings.extend(scan_binding_repair_registry_bridge_markers())
    findings.extend(scan_pre_route_repair_marker_allowlists())
    findings.extend(scan_answer_verifier_registry_bridge_markers())
    findings.extend(scan_answer_verifier_output_contract_prompt_marker())
    findings.extend(scan_verifier_contract_missing_detail_marker())
    findings.extend(scan_route_guard_record_contract_marker())
    findings.extend(scan_loop_control_output_contract_marker_key())
    findings.extend(scan_loop_recovery_contract_marker_fields())
    findings.extend(scan_dry_run_contract_plan_marker_payloads())
    findings.extend(scan_observed_output_contract_marker_payload())
    findings.extend(scan_prompt_utils_output_contract_registry_bridge_tokens())
    findings.extend(scan_execution_recipe_registry_bridge_tokens())
    findings.extend(scan_execution_recipe_contract_marker_outputs())
    findings.extend(scan_schema_report_contract_marker_fields())
    findings.extend(scan_contract_matrix_registry_bridge_bypass())
    findings.extend(scan_contract_matrix_trace_contract_marker())
    findings.extend(scan_task_journal_step_contract_marker())
    findings.extend(scan_schedule_preview_contract_marker())
    findings.extend(scan_current_workspace_scope_legacy_semantic_marker_removed())
    findings.extend(scan_runtime_status_recipe_contract_marker())
    findings.extend(scan_prompt_utils_contract_repair_judge_marker_only())
    findings.extend(scan_prompt_utils_output_contract_marker_only())
    findings.extend(scan_intent_router_output_contract_schema_marker_only())
    findings.extend(scan_task_context_builder_registry_bridge_budget())
    findings.extend(scan_task_contract_registry_bridge_semantic_defaults())
    findings.extend(scan_git_deterministic_user_text_action_selection())
    findings.extend(scan_sqlite_route_request_semantic_fallback())
    findings.extend(scan_service_status_identity_user_text_selection())
    findings.extend(scan_service_status_process_user_text_selection())
    findings.extend(scan_service_status_url_user_text_selection())
    findings.extend(scan_service_status_workspace_product_text_selection())
    findings.extend(scan_service_status_scalar_shape_health_selection())
    findings.extend(scan_task_control_task_id_user_text_selection())
    findings.extend(scan_task_control_legacy_token_fallback())
    findings.extend(scan_async_job_start_user_text_command_selection())
    findings.extend(scan_web_search_user_text_query_selection())
    findings.extend(scan_runtime_surface_user_text_token_selection())
    findings.extend(scan_config_change_preview_user_text_selection())
    findings.extend(scan_finalizer_observed_output_registry_bridge_markers())
    return findings


def scan_route_result_raw_semantic_access(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if not ROUTE_RESULT_RAW_SEMANTIC_ACCESS.search(line):
            continue
        if ROUTE_RESULT_RAW_SEMANTIC_CLEAR.search(line):
            continue
        findings.append(
            Finding(
                rel_path,
                line_no,
                "route_result_raw_semantic_access",
                line.strip(),
            )
        )
    return findings


def scan_legacy_json_semantic_fields(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for pattern in LEGACY_JSON_SEMANTIC_FIELD_PATTERNS:
            if not pattern.search(line):
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "legacy_json_semantic_kind_field",
                    line.strip(),
                )
            )
    return findings


def scan_legacy_runtime_semantic_outputs(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in LEGACY_RUNTIME_SEMANTIC_OUTPUT_PATTERNS:
            if not pattern.search(line):
                continue
            findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_normalizer_route_result_boundary() -> list[Finding]:
    path = SRC_ROOT / "intent_router_route_output.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    findings: list[Finding] = []
    required_tokens = [
        "fn demote_output_contract_semantic_to_route_marker",
        'format!("contract:{}"',
        "output_contract.apply_output_contract_ref(OutputContractRef::new(OutputSemanticKind::None));",
        "demote_output_contract_semantic_to_route_marker(&mut output_contract, &mut route_reason);",
    ]
    for token in required_tokens:
        if token in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "normalizer_route_result_semantic_demote_missing",
                f"missing required boundary token: {token}",
            )
        )
    return findings


def scan_journal_output_contract_ref_boundary() -> list[Finding]:
    path = SRC_ROOT / "task_journal_decision_envelope.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    if "let contract = route.effective_output_contract();" in text:
        return []
    return [
        Finding(
            rel_path,
            1,
            "journal_output_contract_ref_not_effective",
            "output_contract_ref_for_route must use route.effective_output_contract()",
        )
    ]


def scan_static_capability_compat_boundary() -> list[Finding]:
    paths = (
        SRC_ROOT / "capability_resolver.rs",
        SRC_ROOT / "capability_resolver_tests.rs",
        SRC_ROOT / "agent_engine" / "dispatch_support.rs",
    )
    forbidden_tokens = [
        "resolve_static_capability",
        "resolve_static_capability_action_for_state",
        "static_capability_compat_enabled",
        "static_capability",
        "static_capabilities",
        "registry_capability_surface_available",
        "capability_resolver_static_compat_resolved",
        "capability_resolver_unresolved",
        '"static_compat"',
    ]
    findings: list[Finding] = []
    for path in paths:
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        for line_no, line in enumerate(text.splitlines(), start=1):
            for token in forbidden_tokens:
                if token not in line:
                    continue
                findings.append(
                    Finding(
                        rel_path,
                        line_no,
                        "static_capability_compat_forbidden",
                        line.strip(),
                    )
                )
    return findings


def scan_contract_repair_judge_boundary() -> list[Finding]:
    path = SRC_ROOT / "intent_router_normalizer_answer_repair.rs"
    if not path.exists():
        return []
    return scan_contract_repair_judge_boundary_text(rel(path), path.read_text(encoding="utf-8"))


def scan_contract_repair_judge_boundary_text(rel_path: str, text: str) -> list[Finding]:
    required_tokens = [
        "#[cfg(test)]\nasync fn apply_contract_judge_repair(",
        "#[cfg(not(test))]\nasync fn apply_contract_judge_repair(",
        "contract_repair_report.needs_llm_contract_integrity_repair()",
    ]
    findings: list[Finding] = []
    for token in required_tokens:
        if token in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_repair_judge_boundary_missing",
                f"missing required boundary token: {token}",
            )
        )
    if "contract_repair_judge_runtime_enabled" in text or "cfg!(test)" in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_repair_judge_runtime_switch",
                "pre-agent LLM repair must be compile-time test-only, not a runtime switch",
            )
        )
    findings.extend(scan_semantic_suspect_report_boundary(rel_path, text))
    return findings


def scan_semantic_suspect_report_boundary(rel_path: str, text: str) -> list[Finding]:
    semantic_report_pos = text.find('contract_repair_report.add("semantic_suspect"')
    if semantic_report_pos < 0:
        return []
    test_only_repair_pos = text.find(
        "#[cfg(test)]\nasync fn apply_contract_judge_repair("
    )
    if 0 <= test_only_repair_pos < semantic_report_pos:
        return []
    return [
        Finding(
            rel_path,
            1,
            "semantic_suspect_report_not_test_gated",
            "semantic_suspect report collection must stay behind contract_repair_judge_runtime_enabled()",
        )
    ]


def print_report(findings: list[Finding]) -> int:
    print(f"RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings={len(findings)}")
    for item in findings:
        print(f"  - {item.path}:{item.line} [{item.kind}] {item.text}")
    return 1 if findings else 0


def run_self_test() -> int:
    blocked_removed_path = scan_text(
        "crates/clawd/src/ask_flow_pre_planner_exit.rs",
        '"decision_source": "semantic_rewrite",\n',
    )
    assert blocked_removed_path and blocked_removed_path[0].kind == "semantic_rewrite"
    blocked = scan_text(
        "crates/clawd/src/agent_engine/planning.rs",
        '"decision_source": "semantic_rewrite",\n',
    )
    assert blocked and blocked[0].kind == "semantic_rewrite"
    blocked_debt = scan_text(
        "crates/clawd/src/finalize/task.rs",
        '"semantic_control_state": "legacy_migration_debt",\n',
    )
    assert blocked_debt and blocked_debt[0].kind == "legacy_migration_debt"
    blocked_legacy_class = scan_text(
        "crates/clawd/src/intent_router_route_trace.rs",
        '"legacy_semantic_reroute"\n',
    )
    assert blocked_legacy_class and blocked_legacy_class[0].kind == "legacy_semantic_reroute"
    blocked_semantic_defer_owner = scan_text(
        "crates/clawd/src/post_route_policy.rs",
        '"agent_loop_semantic_defer"\n',
    )
    assert (
        blocked_semantic_defer_owner
        and blocked_semantic_defer_owner[0].kind == "agent_loop_semantic_defer"
    )
    blocked_semantic_defer_reason = scan_text(
        "crates/clawd/src/post_route_policy.rs",
        '"post_route_semantic_clarify_deferred_to_agent_loop"\n',
    )
    assert (
        blocked_semantic_defer_reason
        and blocked_semantic_defer_reason[0].kind
        == "post_route_semantic_clarify_deferred"
    )
    blocked_raw_route = scan_route_result_raw_semantic_access(
        "crates/clawd/src/agent_engine/planning.rs",
        "if route.output_contract.semantic_kind == OutputSemanticKind::FilePaths {}\n",
    )
    assert blocked_raw_route and blocked_raw_route[0].kind == "route_result_raw_semantic_access"
    allowed_clear = scan_route_result_raw_semantic_access(
        "crates/clawd/src/worker/ask_prepare.rs",
        "route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;\n",
    )
    assert not allowed_clear
    blocked_legacy_json_pointer = scan_legacy_json_semantic_fields(
        "crates/clawd/src/finalize/loop_reply_contract_enforce.rs",
        '.pointer("/semantic_kind")\n',
    )
    assert (
        blocked_legacy_json_pointer
        and blocked_legacy_json_pointer[0].kind == "legacy_json_semantic_kind_field"
    )
    blocked_legacy_kv_output = scan_legacy_runtime_semantic_outputs(
        "crates/clawd/src/finalize/loop_reply_execution_status.rs",
        'lines.push(format!("semantic_kind={}", marker));\n',
    )
    assert (
        blocked_legacy_kv_output
        and blocked_legacy_kv_output[0].kind == "legacy_semantic_kv_output"
    )
    blocked_legacy_trace_eq = scan_legacy_runtime_semantic_outputs(
        "crates/clawd/src/intent_router_route_trace.rs",
        'format!("shape={};semantic={};locator={}", shape, marker, locator)\n',
    )
    assert (
        blocked_legacy_trace_eq
        and blocked_legacy_trace_eq[0].kind == "legacy_semantic_trace_label"
    )
    blocked_legacy_trace_colon = scan_legacy_runtime_semantic_outputs(
        "crates/clawd/src/task_journal_decision_envelope.rs",
        'format!("semantic:{}|shape:{}", marker, shape)\n',
    )
    assert (
        blocked_legacy_trace_colon
        and blocked_legacy_trace_colon[0].kind == "legacy_semantic_trace_label"
    )
    blocked_legacy_prompt_instruction = scan_legacy_runtime_semantic_outputs(
        "crates/clawd/src/intent_router_prompt_render.rs",
        '"Set semantic_kind=\\"none\\" in normalizer output."\n',
    )
    assert (
        blocked_legacy_prompt_instruction
        and blocked_legacy_prompt_instruction[0].kind
        == "legacy_semantic_prompt_instruction"
    )
    blocked_normalizer_output_raw_request = scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "struct IntentNormalizerOutput {\n    raw_user_request: String,\n}\n"
        "struct BoundaryEnvelope {\n    raw_chars: usize,\n}\n",
    )
    assert any(
        item.kind == "intent_normalizer_output_raw_user_request_field"
        for item in blocked_normalizer_output_raw_request
    )
    blocked_normalizer_output_attachment_field = scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "struct IntentNormalizerOutput {\n    attachment_processing_required: bool,\n}\n"
        "struct BoundaryEnvelope {\n    raw_chars: usize,\n}\n",
    )
    assert any(
        item.kind == "intent_normalizer_output_attachment_required_field"
        for item in blocked_normalizer_output_attachment_field
    )
    blocked_normalizer_output_route_trace_field = scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "struct IntentNormalizerOutput {\n    route_trace_decision: FirstLayerDecision,\n}\n"
        "struct BoundaryEnvelope {\n    raw_chars: usize,\n}\n",
    )
    assert any(
        item.kind == "intent_normalizer_output_route_trace_decision_field"
        for item in blocked_normalizer_output_route_trace_field
    )
    assert not scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "pub(crate) const BOUNDARY_ENVELOPE_SCHEMA_VERSION: u8 = 1;\n"
        "struct IntentNormalizerOutput {\n    boundary_envelope: BoundaryEnvelope,\n}\n"
        "struct BoundaryEnvelope {\n    raw_chars: usize,\n}\n"
        "impl BoundaryEnvelope {\n    pub(crate) fn schema_version(&self) -> u8 {\n"
        "        BOUNDARY_ENVELOPE_SCHEMA_VERSION\n    }\n}\n",
    )
    missing_route_trace_decision_enum = scan_route_trace_record_decision_type_text(
        "crates/clawd/src/intent_router_route_trace.rs",
        "struct RouteTraceRecord {\n    route_trace_decision: RouteTraceDecision,\n}\n",
    )
    assert any(
        item.kind == "route_trace_decision_enum_missing"
        for item in missing_route_trace_decision_enum
    )
    blocked_route_trace_first_layer_field = scan_route_trace_record_decision_type_text(
        "crates/clawd/src/intent_router_route_trace.rs",
        "enum RouteTraceDecision {}\n"
        "struct RouteTraceRecord {\n    route_trace_decision: FirstLayerDecision,\n}\n",
    )
    assert any(
        item.kind == "route_trace_record_first_layer_decision_field"
        for item in blocked_route_trace_first_layer_field
    )
    assert not scan_route_trace_record_decision_type_text(
        "crates/clawd/src/intent_router_route_trace.rs",
        "enum RouteTraceDecision {}\n"
        "struct RouteTraceRecord {\n    route_trace_decision: RouteTraceDecision,\n}\n",
    )
    blocked_normalizer_route_trace_return_type = (
        scan_normalizer_run_route_trace_decision_type_text(
            "crates/clawd/src/intent_router_normalizer_run.rs",
            "fn route_trace_decision_from_state() -> FirstLayerDecision {\n"
            "    RouteTraceDecision::Respond\n"
            "}\n",
        )
    )
    assert any(
        item.kind == "normalizer_route_trace_first_layer_return_type"
        for item in blocked_normalizer_route_trace_return_type
    )
    blocked_normalizer_route_trace_variant = (
        scan_normalizer_run_route_trace_decision_type_text(
            "crates/clawd/src/intent_router_normalizer_run.rs",
            "fn route_trace_decision_from_state() -> RouteTraceDecision {\n"
            "    FirstLayerDecision::DirectAnswer\n"
            "}\n",
        )
    )
    assert any(
        item.kind == "normalizer_route_trace_first_layer_variant"
        for item in blocked_normalizer_route_trace_variant
    )
    blocked_normalizer_route_trace_label_helper = (
        scan_normalizer_run_route_trace_decision_type_text(
            "crates/clawd/src/intent_router_normalizer_run.rs",
            "fn route_trace_decision_from_state() -> RouteTraceDecision {\n"
            "    RouteTraceDecision::Respond\n"
            "}\n"
            "fn route_trace_label_from_state() {\n"
            "    route_label_from_first_layer_decision(decision, finalize_style);\n"
            "}\n",
        )
    )
    assert any(
        item.kind == "normalizer_route_trace_first_layer_label_helper"
        for item in blocked_normalizer_route_trace_label_helper
    )
    assert not scan_normalizer_run_route_trace_decision_type_text(
        "crates/clawd/src/intent_router_normalizer_run.rs",
        "fn route_trace_decision_from_state() -> RouteTraceDecision {\n"
        "    RouteTraceDecision::Respond\n"
        "}\n"
        "fn route_trace_label_from_decision() {\n"
        "    RouteTraceDecision::Act.as_str();\n"
        "}\n",
    )
    blocked_normalizer_route_trace_label = scan_normalizer_route_trace_label_tokens_text(
        "crates/clawd/src/intent_router_normalizer_run.rs",
        'fn route_trace_label_from_decision() {\n    "ChatAct"\n}\n',
    )
    assert (
        blocked_normalizer_route_trace_label
        and blocked_normalizer_route_trace_label[0].kind
        == "normalizer_route_trace_legacy_label"
    )
    assert not scan_normalizer_route_trace_label_tokens_text(
        "crates/clawd/src/intent_router_normalizer_run.rs",
        'fn route_trace_label_from_decision() {\n'
        '    "respond";\n'
        '    "act_chat_finalizer";\n'
        '    "act_plain_finalizer";\n'
        '    "clarify";\n'
        "}\n",
    )
    blocked_runtime_route_trace_return_type = (
        scan_runtime_journal_route_trace_decision_type_text(
            "crates/clawd/src/runtime/ask_mode.rs",
            "pub(crate) fn route_trace_decision_for_journal(&self) -> FirstLayerDecision {\n"
            "    AskRouteTraceDecision::Respond\n"
            "}\n",
        )
    )
    assert any(
        item.kind == "runtime_journal_route_trace_first_layer_return_type"
        for item in blocked_runtime_route_trace_return_type
    )
    blocked_runtime_route_trace_variant = (
        scan_runtime_journal_route_trace_decision_type_text(
            "crates/clawd/src/runtime/ask_mode.rs",
            "pub(crate) fn route_trace_decision_for_journal(&self) -> AskRouteTraceDecision {\n"
            "    FirstLayerDecision::DirectAnswer\n"
            "}\n",
        )
    )
    assert any(
        item.kind == "runtime_journal_route_trace_first_layer_variant"
        for item in blocked_runtime_route_trace_variant
    )
    assert not scan_runtime_journal_route_trace_decision_type_text(
        "crates/clawd/src/runtime/ask_mode.rs",
        "pub(crate) fn route_trace_decision_for_journal(&self) -> AskRouteTraceDecision {\n"
        "    AskRouteTraceDecision::Respond\n"
        "}\n",
    )
    blocked_ask_mode_route_trace_label = scan_ask_mode_route_trace_label_tokens_text(
        "crates/clawd/src/runtime/ask_mode.rs",
        'pub(crate) fn route_trace_label_for_log(&self) -> &\'static str {\n'
        '    "ChatAct"\n'
        "}\n"
        "pub(crate) fn route_trace_decision_for_journal(&self) {}\n",
    )
    assert (
        blocked_ask_mode_route_trace_label
        and blocked_ask_mode_route_trace_label[0].kind
        == "ask_mode_route_trace_legacy_label"
    )
    assert not scan_ask_mode_route_trace_label_tokens_text(
        "crates/clawd/src/runtime/ask_mode.rs",
        'pub(crate) fn route_trace_label_for_log(&self) -> &\'static str {\n'
        '    "respond";\n'
        '    "clarify";\n'
        '    "respond_resume_discussion";\n'
        '    "act_plain_finalizer";\n'
        '    "act_chat_finalizer";\n'
        '    "act_resume_continue";\n'
        "}\n"
        "pub(crate) fn route_trace_decision_for_journal(&self) {}\n",
    )
    blocked_first_layer_enum = scan_first_layer_decision_test_only_boundary_text(
        "crates/clawd/src/runtime/types.rs",
        "#[derive(Debug)]\nenum FirstLayerDecision { DirectAnswer }\n",
    )
    assert any(
        item.kind == "first_layer_decision_enum_not_test_only"
        for item in blocked_first_layer_enum
    )
    blocked_first_layer_reexport = scan_first_layer_decision_test_only_boundary_text(
        "crates/clawd/src/main.rs",
        "pub(crate) use runtime::types::FirstLayerDecision;\n",
    )
    assert any(
        item.kind == "first_layer_decision_crate_reexport_not_test_only"
        for item in blocked_first_layer_reexport
    )
    blocked_first_layer_import = scan_first_layer_decision_test_only_boundary_text(
        "crates/clawd/src/intent_router.rs",
        "use crate::FirstLayerDecision;\n",
    )
    assert any(
        item.kind == "first_layer_decision_import_not_test_only"
        for item in blocked_first_layer_import
    )
    assert not scan_first_layer_decision_test_only_boundary_text(
        "crates/clawd/src/runtime/types.rs",
        "#[cfg(test)]\n#[derive(Debug)]\nenum FirstLayerDecision { DirectAnswer }\n",
    )
    assert not scan_first_layer_decision_test_only_boundary_text(
        "crates/clawd/src/main.rs",
        "#[cfg(test)]\npub(crate) use runtime::types::FirstLayerDecision;\n",
    )
    assert not scan_first_layer_decision_test_only_boundary_text(
        "crates/clawd/src/intent_router.rs",
        "#[cfg(test)]\nuse crate::FirstLayerDecision;\n",
    )
    blocked_normalizer_decision_field = (
        scan_intent_normalizer_legacy_decision_field_deleted_text(
            "crates/clawd/src/intent_router.rs",
            "struct IntentNormalizerOut {\n    decision: String,\n}\n",
        )
    )
    assert any(
        item.kind == "intent_normalizer_out_legacy_decision_field"
        for item in blocked_normalizer_decision_field
    )
    blocked_normalizer_decision_write = (
        scan_intent_normalizer_legacy_decision_field_deleted_text(
            "crates/clawd/src/intent_router_contract_repair_judge.rs",
            "fn repair(out: &mut IntentNormalizerOut) {\n    out.decision = \"act\".to_string();\n}\n",
        )
    )
    assert any(
        item.kind == "intent_normalizer_out_legacy_decision_write"
        for item in blocked_normalizer_decision_write
    )
    assert not scan_intent_normalizer_legacy_decision_field_deleted_text(
        "crates/clawd/src/intent_router.rs",
        "struct IntentNormalizerOut {\n    needs_clarify: bool,\n}\n",
    )
    blocked_legacy_route_trace_reason = scan_legacy_route_trace_reason_tokens_text(
        "crates/clawd/src/intent_router_route_trace.rs",
        'RouteTraceDecision::Act => "planner_execute_trace_inferred",\n',
    )
    assert (
        blocked_legacy_route_trace_reason
        and blocked_legacy_route_trace_reason[0].kind
        == "legacy_route_trace_reason_token"
    )
    assert not scan_legacy_route_trace_reason_tokens_text(
        "crates/clawd/src/intent_router_route_trace.rs",
        'RouteTraceDecision::Act => "act_trace_inferred",\n'
        'RouteTraceDecision::Respond => "respond_trace_inferred",\n',
    )
    assert not scan_normalizer_route_result_boundary()
    assert not scan_journal_output_contract_ref_boundary()
    assert not scan_static_capability_compat_boundary()
    assert not scan_contract_repair_judge_boundary()
    blocked_semantic_suspect = scan_semantic_suspect_report_boundary(
        "crates/clawd/src/intent_router_normalizer_answer_repair.rs",
        'contract_repair_report.add("semantic_suspect", detail);\n'
        "if contract_repair_judge_runtime_enabled()\n"
        "        && contract_repair_report.needs_llm_contract_integrity_repair() {}\n",
    )
    assert (
        blocked_semantic_suspect
        and blocked_semantic_suspect[0].kind == "semantic_suspect_report_not_test_gated"
    )
    allowed_semantic_suspect = scan_semantic_suspect_report_boundary(
        "crates/clawd/src/intent_router_normalizer_answer_repair.rs",
        "#[cfg(test)]\nasync fn apply_contract_judge_repair() {\n"
        '    contract_repair_report.add("semantic_suspect", detail);\n'
        "}\n",
    )
    assert not allowed_semantic_suspect
    blocked_runtime_repair_switch = scan_contract_repair_judge_boundary_text(
        "crates/clawd/src/intent_router_normalizer_answer_repair.rs",
        "fn contract_repair_judge_runtime_enabled() -> bool { cfg!(test) }\n",
    )
    assert (
        blocked_runtime_repair_switch
        and blocked_runtime_repair_switch[0].kind
        == "contract_repair_judge_boundary_missing"
    )
    contract_repair_route_mutation = scan_contract_repair_loop_observation_boundary_text(
        "crates/clawd/src/worker/ask_pipeline_contract_repair.rs",
        "fn f(route_result: &mut crate::RouteResult) {}\n"
        "route_result.output_contract.semantic_kind = OutputSemanticKind::None;\n"
        "route_result.route_reason.push_str(\";contract_repair\");\n"
        "route_result.set_clarify_gate();\n",
    )
    assert {
        "contract_repair_mutable_route_result_param",
        "contract_repair_route_result_field_assignment",
        "contract_repair_route_result_field_mutation_call",
        "contract_repair_route_gate_mutation",
    }.issubset({item.kind for item in contract_repair_route_mutation})
    assert not scan_contract_repair_loop_observation_boundary_text(
        "crates/clawd/src/worker/ask_pipeline_contract_repair.rs",
        'json!({ "source": "contract_repair", "contract_ref": contract_ref })',
    )
    assert not scan_contract_repair_loop_observation_boundary()
    blocked_boundary_preflight_string = scan_boundary_preflight_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline_boundary_preflight.rs",
        "enum BoundaryPreflightDeferral {}\n"
        'push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, "deictic_memory_only");\n'
        'log_route_guard_record(task, "worker_locator_guard", "locatorless_observation_deferred_to_agent_loop", "deferred", before, route);\n',
    )
    assert {
        "boundary_preflight_direct_candidate_push",
        "boundary_preflight_direct_guard_reason",
    }.issubset({item.kind for item in blocked_boundary_preflight_string})
    missing_boundary_preflight_enum = scan_boundary_preflight_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline_boundary_preflight.rs",
        'push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, "x");\n',
    )
    assert (
        missing_boundary_preflight_enum
        and missing_boundary_preflight_enum[0].kind
        == "boundary_preflight_deferral_enum_missing"
    )
    assert not scan_boundary_preflight_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline_boundary_preflight.rs",
        "enum BoundaryPreflightDeferral {}\n"
        "impl BoundaryPreflightDeferral { fn observation_token(self) -> &'static str { \"deictic_memory_only\" } }\n"
        "fn f(item: BoundaryPreflightDeferral) { log_route_guard_record(task, \"worker_locator_guard\", item.reason_code(), \"deferred\", before, route); }\n",
    )
    assert not scan_boundary_preflight_deferral_typing()
    blocked_worker_loop_boundary_string = scan_worker_loop_boundary_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        "enum WorkerLoopBoundaryDeferral {}\n"
        'pre_loop_clarify_candidates.push("bare_topic_context_expansion");\n'
        'push_pre_loop_clarify_candidate(&mut pre_loop_clarify_candidates, "deictic_bare_locator");\n'
        'log_route_guard_record(task, "worker_locator_guard", "directory_file_delivery_deferred_to_agent_loop", "deferred", before, route);\n',
    )
    assert {
        "worker_loop_boundary_direct_candidate_push",
        "worker_loop_boundary_direct_guard_reason",
    }.issubset({item.kind for item in blocked_worker_loop_boundary_string})
    missing_worker_loop_boundary_enum = scan_worker_loop_boundary_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        'pre_loop_clarify_candidates.push("x");\n',
    )
    assert (
        missing_worker_loop_boundary_enum
        and missing_worker_loop_boundary_enum[0].kind
        == "worker_loop_boundary_deferral_enum_missing"
    )
    assert not scan_worker_loop_boundary_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        "enum WorkerLoopBoundaryDeferral {}\n"
        "impl WorkerLoopBoundaryDeferral { fn observation_token(self) -> &'static str { \"bare_topic_context_expansion\" } }\n"
        "fn f(item: WorkerLoopBoundaryDeferral) { log_route_guard_record(task, \"worker_locator_guard\", item.guard_reason_code().unwrap_or(\"\"), \"deferred\", before, route); }\n",
    )
    assert not scan_worker_loop_boundary_deferral_typing()
    blocked_worker_route_marker_string = scan_worker_route_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        "enum WorkerRouteMarker {}\n"
        'append_route_reason(route, "agent_loop_default_entry");\n'
        'append_route_reason(route, "bare_topic_contextual_clarify_sanitized");\n'
        'append_route_reason(route, "auto_locator_suppressed_multiple_explicit_paths");\n',
    )
    assert blocked_worker_route_marker_string and all(
        item.kind == "worker_route_marker_direct_route_reason"
        for item in blocked_worker_route_marker_string
    )
    missing_worker_route_marker_enum = scan_worker_route_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        'append_route_reason(route, "x");\n',
    )
    assert (
        missing_worker_route_marker_enum
        and missing_worker_route_marker_enum[0].kind
        == "worker_route_marker_enum_missing"
    )
    assert not scan_worker_route_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        "enum WorkerRouteMarker {}\n"
        "impl WorkerRouteMarker { fn route_reason(self) -> &'static str { \"agent_loop_default_entry\" } }\n"
        "fn f(item: WorkerRouteMarker) { append_route_reason(route, item.route_reason()); }\n",
    )
    assert not scan_worker_route_marker_typing()
    blocked_background_locator_recovery_string = (
        scan_background_locator_loop_recovery_marker_typing_text(
            "crates/clawd/src/worker/ask_pipeline_background_locator_guard.rs",
            "enum BackgroundLocatorLoopRecoveryMarker {}\n"
            'append_route_reason(route, "active_observed_output_loop_recovery");\n'
            'append_route_reason(route, "recent_observed_results_background_locator_loop_recovery");\n',
        )
    )
    assert blocked_background_locator_recovery_string and all(
        item.kind == "background_locator_recovery_direct_route_reason"
        for item in blocked_background_locator_recovery_string
    )
    missing_background_locator_recovery_enum = (
        scan_background_locator_loop_recovery_marker_typing_text(
            "crates/clawd/src/worker/ask_pipeline_background_locator_guard.rs",
            'append_route_reason(route, "x");\n',
        )
    )
    assert (
        missing_background_locator_recovery_enum
        and missing_background_locator_recovery_enum[0].kind
        == "background_locator_recovery_marker_enum_missing"
    )
    assert not scan_background_locator_loop_recovery_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline_background_locator_guard.rs",
        "enum BackgroundLocatorLoopRecoveryMarker {}\n"
        "impl BackgroundLocatorLoopRecoveryMarker { fn route_reason(self) -> &'static str { \"active_observed_output_loop_recovery\" } }\n"
        "fn f(item: BackgroundLocatorLoopRecoveryMarker) { append_route_reason(route, item.route_reason()); }\n",
    )
    assert not scan_background_locator_loop_recovery_marker_typing()
    blocked_structured_anchor_evidence_string = (
        scan_structured_anchor_evidence_marker_typing_text(
            "crates/clawd/src/worker/ask_pipeline_structured_anchor_guard.rs",
            "enum StructuredAnchorEvidenceMarker {}\n"
            'append_route_reason(route, "structured_anchor_requires_evidence");\n',
        )
    )
    assert (
        blocked_structured_anchor_evidence_string
        and blocked_structured_anchor_evidence_string[0].kind
        == "structured_anchor_evidence_direct_route_reason"
    )
    missing_structured_anchor_evidence_enum = (
        scan_structured_anchor_evidence_marker_typing_text(
            "crates/clawd/src/worker/ask_pipeline_structured_anchor_guard.rs",
            'append_route_reason(route, "x");\n',
        )
    )
    assert (
        missing_structured_anchor_evidence_enum
        and missing_structured_anchor_evidence_enum[0].kind
        == "structured_anchor_evidence_marker_enum_missing"
    )
    assert not scan_structured_anchor_evidence_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline_structured_anchor_guard.rs",
        "enum StructuredAnchorEvidenceMarker {}\n"
        "impl StructuredAnchorEvidenceMarker { fn route_reason(self) -> &'static str { \"structured_anchor_requires_evidence\" } }\n"
        "fn f(item: StructuredAnchorEvidenceMarker) { append_route_reason(route, item.route_reason()); }\n",
    )
    assert not scan_structured_anchor_evidence_marker_typing()
    missing_subagent_boundary_helper = scan_subagent_boundary_deferral_helper_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        'fn other() { append_route_reason(route, "subagent_boundary_clarify_deferred_to_agent_loop"); }\n',
    )
    assert (
        missing_subagent_boundary_helper
        and missing_subagent_boundary_helper[0].kind
        == "subagent_boundary_deferral_helper_missing"
    )
    blocked_subagent_boundary_outside_helper = scan_subagent_boundary_deferral_helper_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        "fn defer_subagent_boundary_clarify_to_agent_loop() {\n"
        '    append_route_reason(route, "subagent_boundary_clarify_deferred_to_agent_loop");\n'
        "}\n"
        "fn build_loop_context_after_boundary_preflight() {\n"
        '    append_route_reason(route, "subagent_boundary_clarify_deferred_to_agent_loop");\n'
        "}\n",
    )
    assert (
        blocked_subagent_boundary_outside_helper
        and blocked_subagent_boundary_outside_helper[0].kind
        == "subagent_boundary_deferral_token_outside_helper"
    )
    assert not scan_subagent_boundary_deferral_helper_text(
        "crates/clawd/src/worker/ask_pipeline.rs",
        "fn defer_subagent_boundary_clarify_to_agent_loop() {\n"
        '    append_route_reason(route, "subagent_boundary_clarify_deferred_to_agent_loop");\n'
        '    PostRouteGateRecord::new("post_route_subagent_boundary_clarify_deferred_to_agent_loop", outcome);\n'
        "}\n"
        "fn build_loop_context_after_boundary_preflight() {}\n",
    )
    assert not scan_subagent_boundary_deferral_helper()
    blocked_file_delivery_boundary_string = scan_file_delivery_boundary_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline_file_delivery.rs",
        "enum FileDeliveryBoundaryDeferral {}\n"
        'PostRouteGateRecord::with_owner("boundary_delivery_gate", "post_route_file_delivery_current_request_locator_deferred_to_loop", outcome);\n'
        'append_route_reason(route, "unresolved_file_delivery_deferred_to_agent_loop");\n',
    )
    assert {
        "file_delivery_boundary_direct_gate_reason",
        "file_delivery_boundary_direct_route_reason",
    }.issubset({item.kind for item in blocked_file_delivery_boundary_string})
    missing_file_delivery_boundary_enum = scan_file_delivery_boundary_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline_file_delivery.rs",
        'append_route_reason(route, "x");\n',
    )
    assert (
        missing_file_delivery_boundary_enum
        and missing_file_delivery_boundary_enum[0].kind
        == "file_delivery_boundary_deferral_enum_missing"
    )
    assert not scan_file_delivery_boundary_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline_file_delivery.rs",
        "enum FileDeliveryBoundaryDeferral {}\n"
        "impl FileDeliveryBoundaryDeferral { fn route_reason(self) -> &'static str { \"unresolved_file_delivery_deferred_to_agent_loop\" } }\n"
        "fn f(item: FileDeliveryBoundaryDeferral) { append_route_reason(route, item.route_reason()); }\n",
    )
    assert not scan_file_delivery_boundary_deferral_typing()
    blocked_default_config_contract_string = (
        scan_default_config_contract_deferral_typing_text(
            "crates/clawd/src/worker/ask_pipeline_default_config.rs",
            "enum DefaultConfigContractDeferral {}\n"
            'append_route_reason(route, "config_contract_default_main_config_deferred_to_loop");\n',
        )
    )
    assert (
        blocked_default_config_contract_string
        and blocked_default_config_contract_string[0].kind
        == "default_config_contract_direct_route_reason"
    )
    missing_default_config_contract_enum = (
        scan_default_config_contract_deferral_typing_text(
            "crates/clawd/src/worker/ask_pipeline_default_config.rs",
            'append_route_reason(route, "x");\n',
        )
    )
    assert (
        missing_default_config_contract_enum
        and missing_default_config_contract_enum[0].kind
        == "default_config_contract_deferral_enum_missing"
    )
    assert not scan_default_config_contract_deferral_typing_text(
        "crates/clawd/src/worker/ask_pipeline_default_config.rs",
        "enum DefaultConfigContractDeferral {}\n"
        "impl DefaultConfigContractDeferral { fn route_reason(self) -> &'static str { \"config_contract_default_main_config_deferred_to_loop\" } }\n"
        "fn f(item: DefaultConfigContractDeferral) { append_route_reason(route, item.route_reason()); }\n",
    )
    assert not scan_default_config_contract_deferral_typing()
    blocked_execution_context_sanitization_string = (
        scan_execution_context_sanitization_typing_text(
            "crates/clawd/src/worker/ask_pipeline_execution_context.rs",
            "enum ExecutionContextSanitization {}\n"
            'append_route_reason(route, "untrusted_normalizer_answer_candidate_removed_from_execution_context");\n',
        )
    )
    assert (
        blocked_execution_context_sanitization_string
        and blocked_execution_context_sanitization_string[0].kind
        == "execution_context_sanitization_direct_route_reason"
    )
    missing_execution_context_sanitization_enum = (
        scan_execution_context_sanitization_typing_text(
            "crates/clawd/src/worker/ask_pipeline_execution_context.rs",
            'append_route_reason(route, "x");\n',
        )
    )
    assert (
        missing_execution_context_sanitization_enum
        and missing_execution_context_sanitization_enum[0].kind
        == "execution_context_sanitization_enum_missing"
    )
    assert not scan_execution_context_sanitization_typing_text(
        "crates/clawd/src/worker/ask_pipeline_execution_context.rs",
        "enum ExecutionContextSanitization {}\n"
        "impl ExecutionContextSanitization { fn route_reason(self) -> &'static str { \"untrusted_normalizer_answer_candidate_removed_from_execution_context\" } }\n"
        "fn f(item: ExecutionContextSanitization) { append_route_reason(route, item.route_reason()); }\n",
    )
    assert not scan_execution_context_sanitization_typing()
    blocked_auto_locator_binding_string = scan_auto_locator_binding_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline_auto_locator_binding.rs",
        "enum AutoLocatorBindingMarker {}\n"
        'append_route_reason(route, "structured_field_read_bound_to_auto_locator");\n',
    )
    assert (
        blocked_auto_locator_binding_string
        and blocked_auto_locator_binding_string[0].kind
        == "auto_locator_binding_direct_route_reason"
    )
    missing_auto_locator_binding_enum = scan_auto_locator_binding_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline_auto_locator_binding.rs",
        'append_route_reason(route, "x");\n',
    )
    assert (
        missing_auto_locator_binding_enum
        and missing_auto_locator_binding_enum[0].kind
        == "auto_locator_binding_marker_enum_missing"
    )
    assert not scan_auto_locator_binding_marker_typing_text(
        "crates/clawd/src/worker/ask_pipeline_auto_locator_binding.rs",
        "enum AutoLocatorBindingMarker {}\n"
        "impl AutoLocatorBindingMarker { fn route_reason(self) -> &'static str { \"structured_field_read_bound_to_auto_locator\" } }\n"
        "fn f(item: AutoLocatorBindingMarker) { append_route_reason(route, item.route_reason()); }\n",
    )
    assert not scan_auto_locator_binding_marker_typing()
    blocked_post_route_candidate_string = scan_post_route_boundary_candidate_typing_text(
        "crates/clawd/src/worker/ask_pipeline_post_route_refinement.rs",
        "enum BoundaryClarifyCandidate {}\n"
        "enum BoundaryContractDeferral {}\n"
        "enum PostRouteBoundaryReady {}\n"
        'if candidate == "post_route_unresolved_file_delivery_requires_locator" {}\n'
        'match candidate { "x" => "post_route_missing_path_scoped_locator", _ => "" }\n',
    )
    assert {
        "post_route_boundary_candidate_string_compare",
        "post_route_boundary_candidate_string_match",
    }.issubset({item.kind for item in blocked_post_route_candidate_string})
    blocked_post_route_deferral_string = scan_post_route_boundary_candidate_typing_text(
        "crates/clawd/src/worker/ask_pipeline_post_route_refinement.rs",
        "enum BoundaryClarifyCandidate {}\n"
        "enum BoundaryContractDeferral {}\n"
        "enum PostRouteBoundaryReady {}\n"
        'push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, "auto_locator_scalar_file_without_current_locator");\n'
        'PostRouteGateRecord::new("post_route_directory_file_delivery_deferred_to_agent_loop", outcome);\n',
    )
    assert {
        "post_route_boundary_deferral_direct_candidate_push",
        "post_route_boundary_deferral_direct_gate_record",
    }.issubset({item.kind for item in blocked_post_route_deferral_string})
    missing_post_route_deferral_enum = scan_post_route_boundary_candidate_typing_text(
        "crates/clawd/src/worker/ask_pipeline_post_route_refinement.rs",
        "enum BoundaryClarifyCandidate {}\n",
    )
    assert (
        missing_post_route_deferral_enum
        and missing_post_route_deferral_enum[0].kind
        == "post_route_boundary_deferral_enum_missing"
    )
    missing_post_route_ready_enum = scan_post_route_boundary_candidate_typing_text(
        "crates/clawd/src/worker/ask_pipeline_post_route_refinement.rs",
        "enum BoundaryClarifyCandidate {}\n"
        "enum BoundaryContractDeferral {}\n",
    )
    assert any(
        item.kind == "post_route_boundary_ready_enum_missing"
        for item in missing_post_route_ready_enum
    )
    blocked_post_route_ready_string = scan_post_route_boundary_candidate_typing_text(
        "crates/clawd/src/worker/ask_pipeline_post_route_refinement.rs",
        "enum BoundaryClarifyCandidate {}\n"
        "enum BoundaryContractDeferral {}\n"
        "enum PostRouteBoundaryReady {}\n"
        'PostRouteGateRecord::new("post_route_locator_guard_deferred_to_prompt_targets", outcome);\n'
        'append_route_reason(route, "locator_guard_deferred_to_prompt_targets");\n',
    )
    assert {
        "post_route_boundary_ready_direct_gate_record",
        "post_route_boundary_ready_direct_route_reason",
    }.issubset({item.kind for item in blocked_post_route_ready_string})
    assert not scan_post_route_boundary_candidate_typing_text(
        "crates/clawd/src/worker/ask_pipeline_post_route_refinement.rs",
        "enum BoundaryClarifyCandidate {}\n"
        "enum BoundaryContractDeferral {}\n"
        "enum PostRouteBoundaryReady {}\n"
        "impl BoundaryClarifyCandidate { fn observation_token(self) -> &'static str { \"post_route_missing_path_scoped_locator\" } }\n",
    )
    assert not scan_post_route_boundary_candidate_typing()
    blocked_prompt = scan_prompt_layer_text(
        "prompts/layers/overlays/intent_normalizer_prompt.md",
        "`weather_query`\n",
    )
    assert (
        blocked_prompt
        and blocked_prompt[0].kind == "prompt_layer_ordinary_semantic_token"
    )
    missing_china_model_boundary = scan_china_model_routing_patch_boundaries_text(
        "prompts/layers/vendor_patches/mimo/routing/common.md",
        "Do not emit legacy `decision`.\n",
    )
    assert (
        missing_china_model_boundary
        and missing_china_model_boundary[0].kind
        == "china_model_routing_boundary_token_missing"
    )
    blocked_planner_legacy_key = scan_planner_prompt_legacy_semantic_kind_keys_text(
        "prompts/layers/overlays/single_plan_execution_prompt.md",
        "- For `semantic_kind=directory_purpose_summary`, do something.\n",
    )
    assert (
        blocked_planner_legacy_key
        and blocked_planner_legacy_key[0].kind
        == "planner_prompt_legacy_semantic_kind_key"
    )
    blocked_boundary_semantic_kind = scan_boundary_semantic_kind_text(
        "prompts/schemas/intent_normalizer.schema.json",
        '"semantic_kind": {"type": "string"}\n',
    )
    assert (
        blocked_boundary_semantic_kind
        and blocked_boundary_semantic_kind[0].kind
        == "boundary_prompt_schema_legacy_semantic_kind"
    )
    assert not scan_prompt_layer_ordinary_semantic_tokens()
    assert not scan_boundary_prompt_schema_legacy_semantic_kind_fields()
    blocked_schema = scan_schema_text(
        "prompts/schemas/intent_normalizer.schema.json",
        '"weather_query"\n',
    )
    assert (
        blocked_schema
        and blocked_schema[0].kind == "normalizer_schema_ordinary_semantic_token"
    )
    blocked_schema_git = scan_schema_text(
        "prompts/schemas/intent_normalizer.schema.json",
        '"git_repository_state"\n',
    )
    assert (
        blocked_schema_git
        and blocked_schema_git[0].kind == "normalizer_schema_ordinary_semantic_token"
    )
    blocked_schema_route_authority_top_level = (
        scan_intent_normalizer_schema_route_authority_json(
            "prompts/schemas/intent_normalizer.schema.json",
            {"type": "object", "properties": {"decision": {"type": "string"}}},
        )
    )
    assert (
        blocked_schema_route_authority_top_level
        and blocked_schema_route_authority_top_level[0].kind
        == "normalizer_schema_route_authority_top_level_field"
    )
    blocked_schema_route_authority_required = (
        scan_intent_normalizer_schema_route_authority_json(
            "prompts/schemas/intent_normalizer.schema.json",
            {"type": "object", "required": ["answer_candidate"], "properties": {}},
        )
    )
    assert (
        blocked_schema_route_authority_required
        and blocked_schema_route_authority_required[0].kind
        == "normalizer_schema_route_authority_top_level_required"
    )
    blocked_schema_route_authority_output_contract = (
        scan_intent_normalizer_schema_route_authority_json(
            "prompts/schemas/intent_normalizer.schema.json",
            {
                "type": "object",
                "properties": {
                    "output_contract": {
                        "type": ["object", "null"],
                        "properties": {"semantic_kind": {"type": "string"}},
                    }
                },
            },
        )
    )
    assert (
        blocked_schema_route_authority_output_contract
        and blocked_schema_route_authority_output_contract[0].kind
        == "normalizer_schema_route_authority_output_contract_field"
    )
    blocked_boundary_envelope_raw_text = scan_boundary_envelope_schema_json(
        "prompts/schemas/boundary_envelope.schema.json",
        {
            "type": "object",
            "additionalProperties": False,
            "required": ["raw_chars"],
            "properties": {
                "raw_chars": {"type": "integer"},
                "raw_user_request": {"type": "string"},
            },
        },
    )
    assert (
        blocked_boundary_envelope_raw_text
        and any(
            item.kind == "boundary_envelope_forbidden_field"
            for item in blocked_boundary_envelope_raw_text
        )
    )
    blocked_boundary_envelope_open_schema = scan_boundary_envelope_schema_json(
        "prompts/schemas/boundary_envelope.schema.json",
        {
            "type": "object",
            "additionalProperties": True,
            "required": ["raw_chars"],
            "properties": {"raw_chars": {"type": "integer"}},
        },
    )
    assert any(
        item.kind == "boundary_envelope_schema_not_closed"
        for item in blocked_boundary_envelope_open_schema
    )
    blocked_boundary_envelope_missing_schema_version = scan_boundary_envelope_schema_json(
        "prompts/schemas/boundary_envelope.schema.json",
        {
            "type": "object",
            "additionalProperties": False,
            "required": ["raw_chars"],
            "properties": {"raw_chars": {"type": "integer"}},
        },
    )
    assert any(
        item.kind == "boundary_envelope_schema_version_missing"
        for item in blocked_boundary_envelope_missing_schema_version
    )
    blocked_boundary_envelope_rust_raw_text = scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "pub(crate) struct BoundaryEnvelope {\n    pub(crate) raw_user_request: String,\n}",
    )
    assert (
        blocked_boundary_envelope_rust_raw_text
        and any(
            item.kind == "boundary_envelope_rust_raw_user_request_field"
            for item in blocked_boundary_envelope_rust_raw_text
        )
    )
    blocked_boundary_envelope_rust_missing_raw_chars = scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "pub(crate) struct BoundaryEnvelope {\n    pub(crate) session_binding: Option<String>,\n}",
    )
    assert any(
        item.kind == "boundary_envelope_rust_raw_chars_missing"
        for item in blocked_boundary_envelope_rust_missing_raw_chars
    )
    blocked_boundary_envelope_rust_missing_schema_version = (
        scan_boundary_envelope_rust_type_text(
            "crates/clawd/src/intent_router_output_types.rs",
            "pub(crate) struct BoundaryEnvelope {\n    pub(crate) raw_chars: usize,\n}",
        )
    )
    assert any(
        item.kind == "boundary_envelope_schema_version_const_missing"
        for item in blocked_boundary_envelope_rust_missing_schema_version
    )
    assert not scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "pub(crate) const BOUNDARY_ENVELOPE_SCHEMA_VERSION: u8 = 1;\n"
        "pub(crate) struct BoundaryEnvelope {\n    pub(crate) raw_chars: usize,\n}\n"
        "impl BoundaryEnvelope {\n    pub(crate) fn schema_version(&self) -> u8 {\n"
        "        BOUNDARY_ENVELOPE_SCHEMA_VERSION\n    }\n}\n",
    )
    assert not scan_intent_normalizer_schema_ordinary_semantic_tokens()
    assert not scan_intent_normalizer_schema_route_authority_fields()
    assert not scan_boundary_envelope_schema_machine_only()
    assert not scan_boundary_envelope_rust_type_machine_only()
    blocked_contract_repair_schema = scan_schema_text(
        "prompts/schemas/contract_repair_judge.schema.json",
        '"docker_logs"\n',
    )
    assert (
        blocked_contract_repair_schema
        and blocked_contract_repair_schema[0].kind
        == "normalizer_schema_ordinary_semantic_token"
    )
    blocked_contract_repair_schema_sqlite = scan_schema_text(
        "prompts/schemas/contract_repair_judge.schema.json",
        '"sqlite_schema_version"\n',
    )
    assert (
        blocked_contract_repair_schema_sqlite
        and blocked_contract_repair_schema_sqlite[0].kind
        == "normalizer_schema_ordinary_semantic_token"
    )
    blocked_contract_repair_schema_archive = scan_schema_text(
        "prompts/schemas/contract_repair_judge.schema.json",
        '"archive_pack"\n',
    )
    assert (
        blocked_contract_repair_schema_archive
        and blocked_contract_repair_schema_archive[0].kind
        == "normalizer_schema_ordinary_semantic_token"
    )
    assert not scan_contract_repair_schema_ordinary_semantic_tokens()
    blocked_registry_metadata = scan_schema_text(
        "configs/skills_registry.toml",
        'semantic_tags = ["weather_query"]\n',
    )
    assert (
        blocked_registry_metadata
        and blocked_registry_metadata[0].kind == "normalizer_schema_ordinary_semantic_token"
    )
    blocked_registry_metadata = scan_token_list_text(
        "configs/skills_registry.toml",
        'semantic_tags = ["weather_query"]\n',
        FORBIDDEN_PROMPT_ORDINARY_SEMANTIC_TOKENS,
        "skill_registry_ordinary_semantic_token",
    )
    assert (
        blocked_registry_metadata
        and blocked_registry_metadata[0].kind == "skill_registry_ordinary_semantic_token"
    )
    assert not scan_skill_registry_metadata_ordinary_semantic_tokens()
    blocked_run_cmd = scan_preferred_run_cmd_registry_bridge_text(
        "OutputSemanticKind::DockerImages => \"docker images\".to_string(),\n",
    )
    assert (
        blocked_run_cmd
        and blocked_run_cmd[0].kind == "preferred_run_cmd_registry_bridge_semantic_fallback"
    )
    assert not scan_preferred_run_cmd_registry_bridge_fallback()
    blocked_preferred_structured = scan_token_list_text(
        rel(PREFERRED_STRUCTURED_ACTION_FILE),
        "route.output_contract_marker_is(crate::OutputSemanticKind::DockerImages)\n",
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS,
        "preferred_structured_action_registry_bridge_fallback",
    )
    assert (
        blocked_preferred_structured
        and blocked_preferred_structured[0].kind
        == "preferred_structured_action_registry_bridge_fallback"
    )
    assert not scan_preferred_structured_action_registry_bridge_fallback()
    blocked_migration_class = scan_token_list_text(
        rel(MIGRATION_CLASS_FILE),
        'const LOG_OBSERVATION_MARKERS: &[&str] = &["docker_logs"];\n',
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS,
        "migration_class_registry_bridge_fallback",
    )
    assert (
        blocked_migration_class
        and blocked_migration_class[0].kind == "migration_class_registry_bridge_fallback"
    )
    assert not scan_migration_class_registry_bridge_fallback()
    blocked_ask_prepare = scan_token_list_text(
        rel(ASK_PREPARE_FILE),
        '"weather_query" => Some("weather_query"),\n',
        FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + (
            "web_page_summary",
            "web_search_summary",
            "weather_query",
            "market_quote",
            "image_understanding",
            "photo_organization",
            "publishing_preview",
            "rss_news_fetch",
        ),
        "ask_prepare_registry_bridge_marker_preservation",
    )
    assert (
        blocked_ask_prepare
        and blocked_ask_prepare[0].kind == "ask_prepare_registry_bridge_marker_preservation"
    )
    assert not scan_ask_prepare_registry_bridge_marker_preservation()
    blocked_journal_evidence = scan_token_list_text(
        rel(TASK_JOURNAL_EVIDENCE_COVERAGE_FILE),
        "route.output_contract_marker_is(crate::OutputSemanticKind::PublishingPreview)\n",
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
        "task_journal_evidence_registry_bridge_marker",
    )
    assert (
        blocked_journal_evidence
        and blocked_journal_evidence[0].kind == "task_journal_evidence_registry_bridge_marker"
    )
    assert not scan_task_journal_evidence_registry_bridge_markers()
    blocked_observation_repair = scan_token_list_text(
        rel(INTENT_ROUTER_OBSERVATION_REPAIR_FILE),
        '"package_manager_detection",\n',
        FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + (
            "weather_query",
            "market_quote",
            "web_search_summary",
            "publishing_preview",
            "rss_news_fetch",
            "image_understanding",
            "photo_organization",
        ),
        "observation_repair_registry_bridge_marker",
    )
    assert (
        blocked_observation_repair
        and blocked_observation_repair[0].kind == "observation_repair_registry_bridge_marker"
    )
    assert not scan_observation_repair_registry_bridge_markers()
    blocked_contract_hint = scan_token_list_text(
        rel(INTENT_ROUTER_CONTRACT_HINT_FILE),
        "OutputSemanticKind::PackageManagerDetection => OutputSemanticKind::None,\n",
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS,
        "contract_hint_registry_bridge_marker",
    )
    assert (
        blocked_contract_hint
        and blocked_contract_hint[0].kind == "contract_hint_registry_bridge_marker"
    )
    assert not scan_contract_hint_registry_bridge_semantic_markers()
    blocked_execution_contract = scan_token_list_text(
        rel(INTENT_ROUTER_EXECUTION_CONTRACT_FILE),
        "declared_semantic_kind != OutputSemanticKind::PublishingPreview\n",
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + ("publishing_preview",),
        "execution_contract_registry_bridge_repair",
    )
    assert (
        blocked_execution_contract
        and blocked_execution_contract[0].kind == "execution_contract_registry_bridge_repair"
    )
    assert not scan_execution_contract_registry_bridge_repairs()
    blocked_binding_repair = scan_token_list_text(
        rel(SRC_ROOT / "intent_router_answer_candidate_binding.rs"),
        '"publishing_preview",\n',
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + (
            "publishing_preview",
            "weather_query",
            "market_quote",
            "web_search_summary",
            "image_understanding",
            "photo_organization",
        ),
        "binding_repair_registry_bridge_marker",
    )
    assert (
        blocked_binding_repair
        and blocked_binding_repair[0].kind == "binding_repair_registry_bridge_marker"
    )
    assert not scan_binding_repair_registry_bridge_markers()
    blocked_pre_route_repair_allowlist = scan_pre_route_repair_marker_allowlist_text(
        rel(SRC_ROOT / "intent_router_current_turn_structural_repair.rs"),
        'const FRESH_EVIDENCE_CONTRACT_MARKERS: &[&str] = &[\n'
        '    "git_repository_state",\n'
        "];\n",
    )
    assert (
        blocked_pre_route_repair_allowlist
        and blocked_pre_route_repair_allowlist[0].kind
        == "pre_route_repair_registry_bridge_marker"
    )
    assert not scan_pre_route_repair_marker_allowlists()
    blocked_answer_verifier = scan_token_list_text(
        rel(ANSWER_VERIFIER_FILE),
        '"weather_query",\n',
        FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + (
            "web_page_summary",
            "web_search_summary",
            "weather_query",
            "market_quote",
            "image_understanding",
            "photo_organization",
            "publishing_preview",
            "rss_news_fetch",
        ),
        "answer_verifier_registry_bridge_marker",
    )
    assert (
        blocked_answer_verifier
        and blocked_answer_verifier[0].kind == "answer_verifier_registry_bridge_marker"
    )
    assert not scan_answer_verifier_registry_bridge_markers()
    blocked_prompt_utils = scan_token_list_text(
        rel(PROMPT_UTILS_OUTPUT_CONTRACT_FILE),
        '"docker_logs" => "docker_logs",\n',
        FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + (
            "web_page_summary",
            "web_search_summary",
            "weather_query",
            "market_quote",
            "image_understanding",
            "photo_organization",
            "publishing_preview",
            "rss_news_fetch",
        ),
        "prompt_utils_output_contract_registry_bridge_token",
    )
    assert (
        blocked_prompt_utils
        and blocked_prompt_utils[0].kind == "prompt_utils_output_contract_registry_bridge_token"
    )
    assert not scan_prompt_utils_output_contract_registry_bridge_tokens()
    blocked_contract_repair_judge_decision_gate = (
        scan_prompt_utils_contract_repair_judge_marker_only_text(
            rel(PROMPT_UTILS_CONTRACT_REPAIR_JUDGE_FILE),
            'contract.get("contract_marker");\n'
            'if decision == "planner_execute" { return true; }\n',
        )
    )
    assert any(
        item.kind == "contract_repair_judge_planner_execute_decision_gate"
        for item in blocked_contract_repair_judge_decision_gate
    )
    assert not scan_prompt_utils_contract_repair_judge_marker_only()
    blocked_execution_recipe = scan_token_list_text(
        rel(SRC_ROOT / "intent_router_execution_recipe_schema.rs"),
        '"package_manager_detection"\n',
        FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + ("execution_recipe_package_manager_detection",),
        "execution_recipe_registry_bridge_token",
    )
    assert (
        blocked_execution_recipe
        and blocked_execution_recipe[0].kind == "execution_recipe_registry_bridge_token"
    )
    assert not scan_execution_recipe_registry_bridge_tokens()
    assert not scan_contract_matrix_registry_bridge_bypass()
    blocked_budget = scan_token_list_text(
        rel(TASK_CONTEXT_BUILDER_FILE),
        "OutputSemanticKind::WeatherQuery,\n",
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
        "task_context_builder_registry_bridge_semantic_budget",
    )
    assert (
        blocked_budget
        and blocked_budget[0].kind == "task_context_builder_registry_bridge_semantic_budget"
    )
    assert not scan_task_context_builder_registry_bridge_budget()
    blocked_contract = scan_token_list_text(
        rel(TASK_CONTRACT_FILE),
        "OutputSemanticKind::MarketQuote,\n",
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
        "task_contract_registry_bridge_semantic_default",
    )
    assert (
        blocked_contract
        and blocked_contract[0].kind == "task_contract_registry_bridge_semantic_default"
    )
    assert not scan_task_contract_registry_bridge_semantic_defaults()
    blocked_git_text_action = scan_git_deterministic_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn git_repository_state_deterministic_plan_result(\n"
        ") -> Option<PlanResult> {\n"
        '    let action = git_repository_state_action_from_text(user_text)?;\n'
        '    if structural_token_present(user_text, "status") {}\n'
        "}\n",
    )
    assert (
        blocked_git_text_action
        and blocked_git_text_action[0].kind
        == "git_deterministic_user_text_action_selection"
    )
    assert not scan_git_deterministic_user_text_action_selection()
    blocked_sqlite_route_request = scan_sqlite_route_request_text(
        rel(SINGLE_TARGET_STRUCTURED_FIELD_REWRITE_FILE),
        "pub(super) fn route_requests_sqlite_table_listing(route: &RouteResult) -> bool {\n"
        "    route.output_contract_marker_is(crate::OutputSemanticKind::SqliteTableListing)\n"
        "}\n"
        "pub(super) fn route_requests_sqlite_schema_version(route: &RouteResult) -> bool {\n"
        '    route.route_reason.contains("sqlite_schema_version_target")\n'
        "}\n",
    )
    assert (
        blocked_sqlite_route_request
        and blocked_sqlite_route_request[0].kind
        == "sqlite_route_request_semantic_fallback"
    )
    assert not scan_sqlite_route_request_semantic_fallback()
    blocked_service_status_identity = scan_service_status_identity_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn service_status_requests_system_basic_identity(\n"
        ") -> bool {\n"
        '    structural_token_present(user_text, "hostname")\n'
        '        || structural_token_present(route.resolved_intent.as_str(), "current_user")\n'
        "}\n",
    )
    assert (
        blocked_service_status_identity
        and blocked_service_status_identity[0].kind
        == "service_status_identity_user_text_selection"
    )
    assert not scan_service_status_identity_user_text_selection()
    blocked_service_status_process = scan_service_status_process_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn service_status_deterministic_plan_result(\n"
        ") -> Option<PlanResult> {\n"
        "    first_port_filter_token(user_text);\n"
        "    process_status_filter_token(user_text);\n"
        "}\n",
    )
    assert (
        blocked_service_status_process
        and blocked_service_status_process[0].kind
        == "service_status_process_user_text_selection"
    )
    assert not scan_service_status_process_user_text_selection()
    blocked_service_status_url = scan_service_status_url_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn service_status_url_locator(\n"
        ") -> Option<String> {\n"
        "    [user_text, route.resolved_intent.as_str()]\n"
        "        .into_iter()\n"
        "        .filter_map(crate::intent::locator_extractor::extract_explicit_locator_for_fallback)\n"
        "}\n",
    )
    assert (
        blocked_service_status_url
        and blocked_service_status_url[0].kind == "service_status_url_user_text_selection"
    )
    assert not scan_service_status_url_user_text_selection()
    blocked_service_status_workspace_product = scan_service_status_workspace_product_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn service_status_deterministic_plan_result(\n"
        ") -> Option<PlanResult> {\n"
        "    if request_mentions_workspace_product(state, user_text) {}\n"
        "}\n",
    )
    assert (
        blocked_service_status_workspace_product
        and blocked_service_status_workspace_product[0].kind
        == "service_status_workspace_product_text_selection"
    )
    assert not scan_service_status_workspace_product_text_selection()
    blocked_service_status_scalar_health = scan_service_status_scalar_shape_health_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn service_status_deterministic_plan_result(\n"
        ") -> Option<PlanResult> {\n"
        "    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar\n"
        "        && health_check_available_for_plan(state)\n"
        "    {}\n"
        "}\n",
    )
    assert (
        blocked_service_status_scalar_health
        and blocked_service_status_scalar_health[0].kind
        == "service_status_scalar_shape_health_selection"
    )
    assert not scan_service_status_scalar_shape_health_selection()
    blocked_task_control_task_id = scan_task_control_task_id_user_text(
        rel(VALUE_STRING_LIST_FILE),
        "fn first_task_id_token(route: &RouteResult, user_text: &str) -> Option<String> {\n"
        "    first_task_id_token(route, user_text)\n"
        "}\n"
        "fn task_control_get_task_id(route: &RouteResult) -> Option<String> {\n"
        "    user_text.trim();\n"
        "}\n",
    )
    assert (
        blocked_task_control_task_id
        and blocked_task_control_task_id[0].kind == "task_control_task_id_user_text_selection"
    )
    assert not scan_task_control_task_id_user_text_selection()
    blocked_task_control_legacy_token = scan_task_control_legacy_token_text(
        rel(VALUE_STRING_LIST_FILE),
        "fn route_mentions_task_control_list(route: &RouteResult) -> bool {\n"
        "    route_reason_has_marker(route, \"task_control.list\")\n"
        "        || route_mentions_machine_token(route, \"task_control.list\")\n"
        "}\n",
    )
    assert (
        blocked_task_control_legacy_token
        and blocked_task_control_legacy_token[0].kind == "task_control_legacy_token_fallback"
    )
    assert not scan_task_control_legacy_token_fallback()
    blocked_async_job_start = scan_async_job_start_user_text_command_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn async_job_start_deterministic_plan_result(\n"
        ") -> Option<PlanResult> {\n"
        "    explicit_command_segment(&state.policy.command_intent, user_text)\n"
        "}\n",
    )
    assert (
        blocked_async_job_start
        and blocked_async_job_start[0].kind == "async_job_start_user_text_command_selection"
    )
    assert not scan_async_job_start_user_text_command_selection()
    blocked_web_search_query = scan_web_search_user_text_query_text(
        rel(VALUE_STRING_LIST_FILE),
        "pub(super) fn web_search_summary_deterministic_plan_result(\n"
        ") -> Option<PlanResult> {\n"
        "    let query = web_search_query_from_route(route, user_text)\n"
        "        .unwrap_or_else(|| user_text.trim().to_string());\n"
        "}\n"
        "fn web_search_query_from_route(route: &RouteResult, user_text: &str) -> Option<String> {\n"
        "    first_quoted_search_query(user_text)\n"
        "        .or_else(|| nonempty_search_query(&route.resolved_intent))\n"
        "}\n"
        "fn first_quoted_search_query(text: &str) -> Option<String> { None }\n",
    )
    assert (
        blocked_web_search_query
        and blocked_web_search_query[0].kind == "web_search_user_text_query_selection"
    )
    assert not scan_web_search_user_text_query_selection()
    blocked_runtime_surface = scan_runtime_surface_user_text_token_text(
        rel(RUNTIME_SURFACE_PLAN_FILE),
        "fn runtime_surface_mentions_any_machine_token(\n"
        "    route: &RouteResult,\n"
        "    user_text: &str,\n"
        "    tokens: &[&str],\n"
        ") -> bool {\n"
        "    [user_text, route.route_reason.as_str()]\n"
        "        .into_iter()\n"
        "        .any(|text| text.contains(tokens[0]))\n"
        "}\n",
    )
    assert (
        blocked_runtime_surface
        and blocked_runtime_surface[0].kind == "runtime_surface_user_text_token_selection"
    )
    assert not scan_runtime_surface_user_text_token_selection()
    blocked_config_preview = scan_config_change_preview_user_text_selection_text(
        rel(READ_RANGE_ACTION_FILE),
        "pub(super) fn parse_config_change_preview(\n"
        "    user_text: &str,\n"
        "    route: &RouteResult,\n"
        "    auto_locator_path: Option<&str>,\n"
        ") -> Option<ParsedConfigChangePreview> {\n"
        "    let field_path = crate::intent::surface_signals::extract_dotted_field_selector(user_text)?;\n"
        "    let value = parse_config_change_value_after_field(user_text, &field_path)?;\n"
        "    None\n"
        "}\n"
        "pub(super) fn config_change_preview_path(\n"
        "    user_text: &str,\n"
        "    route: &RouteResult,\n"
        "    auto_locator_path: Option<&str>,\n"
        ") -> Option<String> {\n"
        "    crate::intent::locator_extractor::extract_explicit_locator_for_fallback(user_text).map(|locator| locator.locator_hint)\n"
        "}\n",
    )
    assert (
        blocked_config_preview
        and blocked_config_preview[0].kind == "config_change_preview_user_text_selection"
    )
    assert not scan_config_change_preview_user_text_selection()
    blocked_finalizer = scan_token_list_text(
        "crates/clawd/src/finalize/loop_reply_weather.rs",
        "route.output_contract_marker_is(crate::OutputSemanticKind::WeatherQuery)\n",
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
        "finalizer_observed_registry_bridge_semantic_marker",
    )
    assert (
        blocked_finalizer
        and blocked_finalizer[0].kind == "finalizer_observed_registry_bridge_semantic_marker"
    )
    assert not scan_finalizer_observed_output_registry_bridge_markers()
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
