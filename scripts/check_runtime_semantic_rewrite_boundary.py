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


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
PREFERRED_RUN_CMD_FILE = SRC_ROOT / "agent_engine/scalar_count_deterministic_plan.rs"
PREFERRED_STRUCTURED_ACTION_FILE = SRC_ROOT / "agent_engine/preferred_structured_action.rs"
MIGRATION_CLASS_FILE = SRC_ROOT / "agent_engine/migration_class.rs"
ASK_PREPARE_FILE = SRC_ROOT / "worker/ask_prepare.rs"
ASK_PIPELINE_FILE = SRC_ROOT / "worker/ask_pipeline.rs"
TASK_JOURNAL_EVIDENCE_COVERAGE_FILE = SRC_ROOT / "task_journal_evidence_coverage.rs"
INTENT_ROUTER_OBSERVATION_REPAIR_FILE = SRC_ROOT / "intent_router_observation_repair.rs"
INTENT_ROUTER_CONTRACT_HINT_FILE = SRC_ROOT / "intent_router_contract_hint.rs"
INTENT_ROUTER_EXECUTION_CONTRACT_FILE = SRC_ROOT / "intent_router_execution_contract.rs"
INTENT_ROUTER_BINDING_REPAIR_FILES: tuple[Path, ...] = (
    SRC_ROOT / "intent_router_answer_candidate_binding.rs",
    SRC_ROOT / "intent_router_active_task_repair.rs",
    SRC_ROOT / "intent_router_current_turn_structural_repair.rs",
)
PRE_ROUTE_REPAIR_MARKER_ALLOWLIST_FILES: tuple[Path, ...] = (
    SRC_ROOT / "intent_router_active_task_repair.rs",
    SRC_ROOT / "intent_router_current_turn_structural_repair.rs",
    SRC_ROOT / "intent_router_observation_repair.rs",
)
ANSWER_VERIFIER_FILE = SRC_ROOT / "answer_verifier.rs"
PROMPT_UTILS_OUTPUT_CONTRACT_FILE = SRC_ROOT / "prompt_utils_output_contract.rs"
EXECUTION_RECIPE_SCHEMA_FILES: tuple[Path, ...] = (
    SRC_ROOT / "intent_router_execution_recipe_schema.rs",
    SRC_ROOT / "intent_router_execution_recipe_contract.rs",
    SRC_ROOT / "intent_router_normalizer_schema_core.rs",
    SRC_ROOT / "intent_router_schema_report.rs",
    SRC_ROOT / "intent_router_route_trace.rs",
    SRC_ROOT / "intent_router_contract_repair_report.rs",
)
CONTRACT_MATRIX_FILE = SRC_ROOT / "contract_matrix.rs"
TASK_CONTEXT_BUILDER_FILE = SRC_ROOT / "task_context_builder.rs"
TASK_CONTRACT_FILE = SRC_ROOT / "task_contract.rs"
VALUE_STRING_LIST_FILE = SRC_ROOT / "agent_engine/value_string_list.rs"
RUNTIME_SURFACE_PLAN_FILE = SRC_ROOT / "agent_engine/runtime_surface_plan.rs"
PLANNING_PROMPT_FILE = SRC_ROOT / "agent_engine/planning_prompt.rs"
READ_RANGE_ACTION_FILE = SRC_ROOT / "agent_engine/read_range_action.rs"
SINGLE_TARGET_STRUCTURED_FIELD_REWRITE_FILE = (
    SRC_ROOT / "agent_engine/single_target_structured_field_rewrite.rs"
)
FINALIZER_OBSERVED_OUTPUT_SCAN_ROOTS: tuple[Path, ...] = (
    SRC_ROOT / "finalize",
    SRC_ROOT / "agent_engine/observed_output.rs",
    SRC_ROOT / "agent_engine/observed_output_direct_answer.rs",
    SRC_ROOT / "agent_engine/observed_output_direct_scalar.rs",
    SRC_ROOT / "agent_engine/value_string_list.rs",
    SRC_ROOT / "agent_engine/direct_observed_finalize_support.rs",
    SRC_ROOT / "agent_engine/loop_control_answer_recovery.rs",
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

ALLOWED_PRODUCTION_FILES: set[str] = set()

PROMPT_LAYERS_ROOT = ROOT / "prompts/layers"
INTENT_NORMALIZER_SCHEMA = ROOT / "prompts/schemas/intent_normalizer.schema.json"
CONTRACT_REPAIR_JUDGE_SCHEMA = ROOT / "prompts/schemas/contract_repair_judge.schema.json"
SKILL_REGISTRY_METADATA_FILES: tuple[Path, ...] = (
    ROOT / "configs/skills_registry.toml",
    ROOT / "docker/config/skills_registry.toml",
)
FORBIDDEN_PROMPT_ORDINARY_SEMANTIC_TOKENS: tuple[str, ...] = (
    "rss_news_fetch",
    "web_page_summary",
    "web_search_summary",
    "weather_query",
    "market_quote",
    "image_understanding",
    "photo_organization",
    "publishing_preview",
    "package_manager_detection",
    "docker_ps",
    "docker_images",
    "docker_logs",
    "docker_container_lifecycle",
)
FORBIDDEN_SCHEMA_ORDINARY_SEMANTIC_TOKENS: tuple[str, ...] = (
    *FORBIDDEN_PROMPT_ORDINARY_SEMANTIC_TOKENS,
    "rss_latest_news",
    "rss_feed_fetch",
    "external_news_fetch",
    "webpage_summary",
    "web_content_summary",
    "url_content_summary",
    "browser_page_summary",
    "web_search_results",
    "search_results_summary",
    "weather_current",
    "weather_forecast",
    "weather_report",
    "stock_quote",
    "crypto_quote",
    "asset_quote",
    "market_price",
    "image_description",
    "image_describe",
    "image_vision",
    "image_extract",
    "image_compare",
    "screenshot_summary",
    "photo_organization",
    "photo_organize",
    "photo_organizing",
    "photo_source_candidates",
    "photo_discovery",
    "photo_organization_preview",
    "social_post_preview",
    "channel_draft_preview",
    "package_manager_detect",
    "package_detect_manager",
    "docker_containers",
    "docker_container_list",
    "docker_image_list",
    "docker_lifecycle",
    "git_commit_subject",
    "git_commit_title",
    "commit_subject",
    "commit_title",
    "latest_commit_subject",
    "latest_commit_title",
    "git_repository_state",
    "sqlite_table_listing",
    "sqlite_tables_listing",
    "sqlite_tables_summary",
    "sqlite_table_names_only",
    "sqlite_table_names",
    "sqlite_names_only",
    "sqlite_database_kind_judgment",
    "sqlite_db_kind",
    "database_kind_judgment",
    "sqlite_schema_version",
    "sqlite_db_schema_version",
    "config_validation",
    "structured_config_validation",
    "config_mutation",
    "structured_config_mutation",
    "config_risk_assessment",
    "config_risk",
    "structured_config_risk",
    "archive_list",
    "archive_read",
    "archive_pack",
    "archive_unpack",
)
FORBIDDEN_PREFERRED_RUN_CMD_SEMANTIC_ENUMS: tuple[str, ...] = (
    "OutputSemanticKind::PackageManagerDetection",
    "OutputSemanticKind::DockerPs",
    "OutputSemanticKind::DockerImages",
    "OutputSemanticKind::DockerLogs",
    "OutputSemanticKind::DockerContainerLifecycle",
)
FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS: tuple[str, ...] = (
    "OutputSemanticKind::RssNewsFetch",
    "OutputSemanticKind::WebPageSummary",
    "OutputSemanticKind::WebSearchSummary",
    "OutputSemanticKind::WeatherQuery",
    "OutputSemanticKind::MarketQuote",
    "OutputSemanticKind::ImageUnderstanding",
    "OutputSemanticKind::PhotoOrganization",
    "OutputSemanticKind::PublishingPreview",
    "OutputSemanticKind::PackageManagerDetection",
    "OutputSemanticKind::DockerPs",
    "OutputSemanticKind::DockerImages",
    "OutputSemanticKind::DockerLogs",
    "OutputSemanticKind::DockerContainerLifecycle",
)
FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS: tuple[str, ...] = (
    "package_manager_detection",
    "docker_ps",
    "docker_images",
    "docker_logs",
    "docker_container_lifecycle",
)
FORBIDDEN_PRE_ROUTE_REPAIR_MARKER_TOKENS: tuple[str, ...] = (
    "git_commit_subject",
    "git_repository_state",
    "sqlite_table_listing",
    "sqlite_table_names_only",
    "sqlite_database_kind_judgment",
    "sqlite_schema_version",
    "config_validation",
    "config_mutation",
    "config_risk_assessment",
    "archive_list",
    "archive_read",
    "archive_pack",
    "archive_unpack",
    "tool_discovery",
)
PRE_ROUTE_REPAIR_MARKER_ALLOWLIST_NAMES: tuple[str, ...] = (
    "FRESH_EVIDENCE_CONTRACT_MARKERS",
    "WORKSPACE_DEFAULT_OBSERVATION_MARKERS",
    "LOCATORLESS_DEFAULT_OBSERVATION_MARKERS",
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
    findings.extend(scan_normalizer_route_result_boundary())
    findings.extend(scan_journal_output_contract_ref_boundary())
    findings.extend(scan_static_capability_compat_boundary())
    findings.extend(scan_contract_repair_judge_boundary())
    findings.extend(scan_prompt_layer_ordinary_semantic_tokens())
    findings.extend(scan_intent_normalizer_schema_ordinary_semantic_tokens())
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
    findings.extend(scan_prompt_utils_output_contract_registry_bridge_tokens())
    findings.extend(scan_execution_recipe_registry_bridge_tokens())
    findings.extend(scan_contract_matrix_registry_bridge_bypass())
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


def scan_normalizer_route_result_boundary() -> list[Finding]:
    path = SRC_ROOT / "intent_router_route_output.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    findings: list[Finding] = []
    required_tokens = [
        "fn demote_output_contract_semantic_to_route_marker",
        'format!("contract:{}"',
        "output_contract.semantic_kind = OutputSemanticKind::None;",
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
    path = SRC_ROOT / "capability_resolver.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    forbidden_tokens = [
        "resolve_static_capability_action_for_state",
        "static_capability_compat_enabled",
        "registry_capability_surface_available",
        "capability_resolver_static_compat_resolved",
        '"static_compat"',
    ]
    findings: list[Finding] = []
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
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    required_tokens = [
        "fn contract_repair_judge_runtime_enabled() -> bool",
        "cfg!(test)",
        "contract_repair_judge_runtime_enabled()",
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
    findings.extend(scan_semantic_suspect_report_boundary(rel_path, text))
    return findings


def scan_semantic_suspect_report_boundary(rel_path: str, text: str) -> list[Finding]:
    semantic_report_pos = text.find('contract_repair_report.add("semantic_suspect"')
    if semantic_report_pos < 0:
        return []
    guard_pos = text.find("if contract_repair_judge_runtime_enabled() {")
    judge_call_pos = text.find(
        "if contract_repair_judge_runtime_enabled()\n"
        "        && contract_repair_report.needs_llm_contract_integrity_repair()"
    )
    if 0 <= guard_pos < semantic_report_pos and (
        judge_call_pos < 0 or semantic_report_pos < judge_call_pos
    ):
        return []
    return [
        Finding(
            rel_path,
            1,
            "semantic_suspect_report_not_test_gated",
            "semantic_suspect report collection must stay behind contract_repair_judge_runtime_enabled()",
        )
    ]


def scan_prompt_layer_ordinary_semantic_tokens() -> list[Finding]:
    findings: list[Finding] = []
    for path in sorted(PROMPT_LAYERS_ROOT.rglob("*.md")):
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        findings.extend(scan_prompt_layer_text(rel_path, text))
    return findings


def scan_prompt_layer_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in FORBIDDEN_PROMPT_ORDINARY_SEMANTIC_TOKENS:
            if token not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "prompt_layer_ordinary_semantic_token",
                    line.strip(),
                )
            )
    return findings


def scan_intent_normalizer_schema_ordinary_semantic_tokens() -> list[Finding]:
    rel_path = rel(INTENT_NORMALIZER_SCHEMA)
    return scan_schema_text(rel_path, INTENT_NORMALIZER_SCHEMA.read_text(encoding="utf-8"))


def scan_contract_repair_schema_ordinary_semantic_tokens() -> list[Finding]:
    rel_path = rel(CONTRACT_REPAIR_JUDGE_SCHEMA)
    return scan_schema_text(rel_path, CONTRACT_REPAIR_JUDGE_SCHEMA.read_text(encoding="utf-8"))


def scan_skill_registry_metadata_ordinary_semantic_tokens() -> list[Finding]:
    findings: list[Finding] = []
    for path in SKILL_REGISTRY_METADATA_FILES:
        findings.extend(
            scan_token_list_text(
                rel(path),
                path.read_text(encoding="utf-8"),
                FORBIDDEN_PROMPT_ORDINARY_SEMANTIC_TOKENS,
                "skill_registry_ordinary_semantic_token",
            )
        )
    return findings


def scan_schema_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in FORBIDDEN_SCHEMA_ORDINARY_SEMANTIC_TOKENS:
            if f'"{token}"' not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "normalizer_schema_ordinary_semantic_token",
                    line.strip(),
                )
            )
    return findings


def scan_preferred_run_cmd_registry_bridge_fallback() -> list[Finding]:
    rel_path = rel(PREFERRED_RUN_CMD_FILE)
    return scan_preferred_run_cmd_registry_bridge_text(
        PREFERRED_RUN_CMD_FILE.read_text(encoding="utf-8"),
        rel_path=rel_path,
    )


def scan_preferred_run_cmd_registry_bridge_text(
    text: str,
    rel_path: str = rel(PREFERRED_RUN_CMD_FILE),
) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in FORBIDDEN_PREFERRED_RUN_CMD_SEMANTIC_ENUMS:
            if token not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "preferred_run_cmd_registry_bridge_semantic_fallback",
                    line.strip(),
                )
            )
    return findings


def scan_preferred_structured_action_registry_bridge_fallback() -> list[Finding]:
    return scan_token_list_text(
        rel(PREFERRED_STRUCTURED_ACTION_FILE),
        PREFERRED_STRUCTURED_ACTION_FILE.read_text(encoding="utf-8"),
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS,
        "preferred_structured_action_registry_bridge_fallback",
    )


def scan_migration_class_registry_bridge_fallback() -> list[Finding]:
    return scan_token_list_text(
        rel(MIGRATION_CLASS_FILE),
        MIGRATION_CLASS_FILE.read_text(encoding="utf-8"),
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS,
        "migration_class_registry_bridge_fallback",
    )


def scan_ask_prepare_registry_bridge_marker_preservation() -> list[Finding]:
    return scan_token_list_text(
        rel(ASK_PREPARE_FILE),
        ASK_PREPARE_FILE.read_text(encoding="utf-8"),
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


def scan_task_journal_evidence_registry_bridge_markers() -> list[Finding]:
    return scan_token_list_text(
        rel(TASK_JOURNAL_EVIDENCE_COVERAGE_FILE),
        TASK_JOURNAL_EVIDENCE_COVERAGE_FILE.read_text(encoding="utf-8"),
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
        "task_journal_evidence_registry_bridge_marker",
    )


def scan_observation_repair_registry_bridge_markers() -> list[Finding]:
    return scan_token_list_text(
        rel(INTENT_ROUTER_OBSERVATION_REPAIR_FILE),
        INTENT_ROUTER_OBSERVATION_REPAIR_FILE.read_text(encoding="utf-8"),
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


def scan_contract_hint_registry_bridge_semantic_markers() -> list[Finding]:
    return scan_token_list_text(
        rel(INTENT_ROUTER_CONTRACT_HINT_FILE),
        INTENT_ROUTER_CONTRACT_HINT_FILE.read_text(encoding="utf-8"),
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS,
        "contract_hint_registry_bridge_marker",
    )


def scan_execution_contract_registry_bridge_repairs() -> list[Finding]:
    return scan_token_list_text(
        rel(INTENT_ROUTER_EXECUTION_CONTRACT_FILE),
        INTENT_ROUTER_EXECUTION_CONTRACT_FILE.read_text(encoding="utf-8"),
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS
        + FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
        + ("publishing_preview",),
        "execution_contract_registry_bridge_repair",
    )


def scan_binding_repair_registry_bridge_markers() -> list[Finding]:
    findings: list[Finding] = []
    for path in INTENT_ROUTER_BINDING_REPAIR_FILES:
        findings.extend(
            scan_token_list_text(
                rel(path),
                path.read_text(encoding="utf-8"),
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
        )
    return findings


def scan_current_workspace_scope_boundary_marker() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_FILE)
    text = ASK_PIPELINE_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn current_workspace_scope_observation(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "current_workspace_scope_observation_missing",
                "missing current_workspace_scope_observation boundary helper",
            )
        ]
    fn_end = text.find("\nfn current_workspace_scope_has_count_shape", fn_start)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    required_tokens = [
        '"task_shape": "scalar_count"',
        '"contract_marker": route.effective_output_contract_semantic_kind().as_str()',
    ]
    for token in required_tokens:
        if token in body:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "current_workspace_scope_marker_missing",
                f"missing required boundary token: {token}",
            )
        )
    forbidden_tokens = [
        '"semantic_kind": route.effective_output_contract_semantic_kind().as_str()',
        '"semantic_kind": crate::OutputSemanticKind::ScalarCount.as_str()',
    ]
    for token in forbidden_tokens:
        if token not in body:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "current_workspace_scope_semantic_kind_emission",
                f"forbidden boundary token: {token}",
            )
        )
    return findings


def scan_lightweight_tool_spec_contract_marker() -> list[Finding]:
    rel_path = rel(PLANNING_PROMPT_FILE)
    text = PLANNING_PROMPT_FILE.read_text(encoding="utf-8")
    fn_start = text.find("pub(super) fn build_lightweight_tool_spec(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "lightweight_tool_spec_missing",
                "missing build_lightweight_tool_spec",
            )
        ]
    fn_end = text.find("\nconst LIGHTWEIGHT_SKILL_PLAYBOOK_MAX_CHARS", fn_start)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if "contract_marker={}" not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "lightweight_tool_spec_contract_marker_missing",
                "lightweight tool spec should expose contract_marker, not legacy semantic_kind",
            )
        )
    if "semantic_kind={}" in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "lightweight_tool_spec_semantic_kind_emission",
                "lightweight tool spec must not expose legacy semantic_kind",
            )
        )
    return findings


def scan_pre_route_repair_marker_allowlists() -> list[Finding]:
    findings: list[Finding] = []
    for path in PRE_ROUTE_REPAIR_MARKER_ALLOWLIST_FILES:
        findings.extend(
            scan_pre_route_repair_marker_allowlist_text(
                rel(path),
                path.read_text(encoding="utf-8"),
            )
        )
    return findings


def scan_pre_route_repair_marker_allowlist_text(
    rel_path: str,
    text: str,
) -> list[Finding]:
    findings: list[Finding] = []
    in_allowlist = False
    for line_no, line in enumerate(text.splitlines(), start=1):
        if not in_allowlist and any(
            f"const {name}" in line for name in PRE_ROUTE_REPAIR_MARKER_ALLOWLIST_NAMES
        ):
            in_allowlist = True
        if in_allowlist:
            for token in FORBIDDEN_PRE_ROUTE_REPAIR_MARKER_TOKENS:
                if token in line:
                    findings.append(
                        Finding(
                            rel_path,
                            line_no,
                            "pre_route_repair_registry_bridge_marker",
                            line.strip(),
                        )
                    )
            if "];" in line:
                in_allowlist = False
    return findings


def scan_answer_verifier_registry_bridge_markers() -> list[Finding]:
    return scan_token_list_text(
        rel(ANSWER_VERIFIER_FILE),
        ANSWER_VERIFIER_FILE.read_text(encoding="utf-8"),
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


def scan_prompt_utils_output_contract_registry_bridge_tokens() -> list[Finding]:
    return scan_token_list_text(
        rel(PROMPT_UTILS_OUTPUT_CONTRACT_FILE),
        PROMPT_UTILS_OUTPUT_CONTRACT_FILE.read_text(encoding="utf-8"),
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


def scan_execution_recipe_registry_bridge_tokens() -> list[Finding]:
    findings: list[Finding] = []
    for path in EXECUTION_RECIPE_SCHEMA_FILES:
        findings.extend(
            scan_token_list_text(
                rel(path),
                path.read_text(encoding="utf-8"),
                FORBIDDEN_REGISTRY_BRIDGE_MACHINE_TOKENS
                + ("execution_recipe_package_manager_detection",),
                "execution_recipe_registry_bridge_token",
            )
        )
    return findings


def scan_contract_matrix_registry_bridge_bypass() -> list[Finding]:
    text = CONTRACT_MATRIX_FILE.read_text(encoding="utf-8")
    required = re.compile(
        r"fn\s+match_output_contract\b[\s\S]*?"
        r"output\s*\.\s*semantic_kind\s*!=\s*OutputSemanticKind::None[\s\S]*?"
        r"!\s*output\s*\.\s*semantic_kind\s*\.\s*is_normalizer_schema_capability_bridge\s*\(",
        re.MULTILINE,
    )
    if required.search(text):
        return []
    return [
        Finding(
            rel(CONTRACT_MATRIX_FILE),
            1,
            "contract_matrix_registry_bridge_bypass_missing",
            "match_output_contract must not match normalizer schema capability bridge semantic kinds as semantic contracts",
        )
    ]


def scan_task_context_builder_registry_bridge_budget() -> list[Finding]:
    return scan_token_list_text(
        rel(TASK_CONTEXT_BUILDER_FILE),
        TASK_CONTEXT_BUILDER_FILE.read_text(encoding="utf-8"),
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
        "task_context_builder_registry_bridge_semantic_budget",
    )


def scan_task_contract_registry_bridge_semantic_defaults() -> list[Finding]:
    return scan_token_list_text(
        rel(TASK_CONTRACT_FILE),
        TASK_CONTRACT_FILE.read_text(encoding="utf-8"),
        FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
        "task_contract_registry_bridge_semantic_default",
    )


def scan_git_deterministic_user_text_action_selection() -> list[Finding]:
    return scan_git_deterministic_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_git_deterministic_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if "git_repository_state_action_from_text" in line:
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "git_deterministic_user_text_action_selection",
                    line.strip(),
                )
            )
    block = function_block(text, "git_repository_state_deterministic_plan_result")
    if block is None:
        return findings
    block_start, block_text = block
    for offset, line in enumerate(block_text.splitlines(), start=0):
        if "structural_token_present(" not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                block_start + offset,
                "git_deterministic_user_text_action_selection",
                line.strip(),
            )
        )
    return findings


def scan_sqlite_route_request_semantic_fallback() -> list[Finding]:
    return scan_sqlite_route_request_text(
        rel(SINGLE_TARGET_STRUCTURED_FIELD_REWRITE_FILE),
        SINGLE_TARGET_STRUCTURED_FIELD_REWRITE_FILE.read_text(encoding="utf-8"),
    )


def scan_sqlite_route_request_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for function_name in (
        "route_requests_sqlite_table_listing",
        "route_requests_sqlite_schema_version",
    ):
        block = rust_private_or_pub_function_block(text, function_name)
        if block is None:
            continue
        block_start, block_text = block
        for offset, line in enumerate(block_text.splitlines(), start=0):
            if (
                "OutputSemanticKind::Sqlite" not in line
                and "sqlite_schema_version_target" not in line
                and "text_has_sqlite_schema_version_machine_token" not in line
            ):
                continue
            findings.append(
                Finding(
                    rel_path,
                    block_start + offset,
                    "sqlite_route_request_semantic_fallback",
                    line.strip(),
                )
            )
    return findings


def scan_service_status_identity_user_text_selection() -> list[Finding]:
    return scan_service_status_identity_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_service_status_identity_text(rel_path: str, text: str) -> list[Finding]:
    block = function_block(text, "service_status_requests_system_basic_identity")
    if block is None:
        return []
    findings: list[Finding] = []
    block_start, block_text = block
    for offset, line in enumerate(block_text.splitlines(), start=0):
        if (
            "structural_token_present(" not in line
            and "hostname" not in line
            and "host_name" not in line
            and "current_user" not in line
            and "whoami" not in line
        ):
            continue
        findings.append(
            Finding(
                rel_path,
                block_start + offset,
                "service_status_identity_user_text_selection",
                line.strip(),
            )
        )
    return findings


def scan_service_status_process_user_text_selection() -> list[Finding]:
    return scan_service_status_process_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_service_status_process_text(rel_path: str, text: str) -> list[Finding]:
    block = function_block(text, "service_status_deterministic_plan_result")
    if block is None:
        return []
    findings: list[Finding] = []
    block_start, block_text = block
    for offset, line in enumerate(block_text.splitlines(), start=0):
        if (
            "first_port_filter_token(user_text)" not in line
            and "process_status_filter_token(user_text)" not in line
        ):
            continue
        findings.append(
            Finding(
                rel_path,
                block_start + offset,
                "service_status_process_user_text_selection",
                line.strip(),
            )
        )
    return findings


def scan_service_status_url_user_text_selection() -> list[Finding]:
    return scan_service_status_url_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_service_status_url_text(rel_path: str, text: str) -> list[Finding]:
    block = function_block(text, "service_status_url_locator")
    if block is None:
        return []
    findings: list[Finding] = []
    block_start, block_text = block
    for offset, line in enumerate(block_text.splitlines(), start=0):
        if (
            "extract_explicit_locator_for_fallback" not in line
            and "[user_text" not in line
            and "user_text," not in line
        ):
            continue
        findings.append(
            Finding(
                rel_path,
                block_start + offset,
                "service_status_url_user_text_selection",
                line.strip(),
            )
        )
    return findings


def scan_service_status_workspace_product_text_selection() -> list[Finding]:
    return scan_service_status_workspace_product_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_service_status_workspace_product_text(rel_path: str, text: str) -> list[Finding]:
    block = function_block(text, "service_status_deterministic_plan_result")
    if block is None:
        return []
    findings: list[Finding] = []
    block_start, block_text = block
    for offset, line in enumerate(block_text.splitlines(), start=0):
        if "request_mentions_workspace_product" not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                block_start + offset,
                "service_status_workspace_product_text_selection",
                line.strip(),
            )
        )
    return findings


def scan_service_status_scalar_shape_health_selection() -> list[Finding]:
    return scan_service_status_scalar_shape_health_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_service_status_scalar_shape_health_text(rel_path: str, text: str) -> list[Finding]:
    block = function_block(text, "service_status_deterministic_plan_result")
    if block is None:
        return []
    lines = block[1].splitlines()
    findings: list[Finding] = []
    block_start = block[0]
    for idx, line in enumerate(lines):
        if "OutputResponseShape::Scalar" not in line:
            continue
        window = "\n".join(lines[idx : idx + 8])
        if "health_check_available_for_plan" in window and "route_requests_health_check" not in window:
            findings.append(
                Finding(
                    rel_path,
                    block_start + idx,
                    "service_status_scalar_shape_health_selection",
                    line.strip(),
                )
            )
    return findings


def scan_task_control_task_id_user_text_selection() -> list[Finding]:
    return scan_task_control_task_id_user_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_task_control_task_id_user_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if (
            "first_task_id_token(route, user_text)" not in line
            and "fn first_task_id_token(route: &RouteResult, user_text" not in line
        ):
            continue
        findings.append(
            Finding(
                rel_path,
                line_no,
                "task_control_task_id_user_text_selection",
                line.strip(),
            )
        )
    block = function_block(text, "task_control_get_task_id")
    if block is None:
        return findings
    block_start, block_text = block
    for offset, line in enumerate(block_text.splitlines(), start=0):
        if "user_text" not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                block_start + offset,
                "task_control_task_id_user_text_selection",
                line.strip(),
            )
        )
    return findings


def scan_task_control_legacy_token_fallback() -> list[Finding]:
    return scan_task_control_legacy_token_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_task_control_legacy_token_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for function_name in ("route_mentions_task_control_list", "route_mentions_task_control_get"):
        block = rust_private_or_pub_function_block(text, function_name)
        if block is None:
            continue
        block_start, block_text = block
        for offset, line in enumerate(block_text.splitlines(), start=0):
            if (
                "route_reason_has_marker" not in line
                and "route_mentions_machine_token" not in line
            ):
                continue
            findings.append(
                Finding(
                    rel_path,
                    block_start + offset,
                    "task_control_legacy_token_fallback",
                    line.strip(),
                )
            )
    return findings


def scan_async_job_start_user_text_command_selection() -> list[Finding]:
    return scan_async_job_start_user_text_command_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_async_job_start_user_text_command_text(rel_path: str, text: str) -> list[Finding]:
    block = function_block(text, "async_job_start_deterministic_plan_result")
    if block is None:
        return []
    findings: list[Finding] = []
    block_start, block_text = block
    for offset, line in enumerate(block_text.splitlines(), start=0):
        if "explicit_command_segment(" not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                block_start + offset,
                "async_job_start_user_text_command_selection",
                line.strip(),
            )
        )
    return findings


def scan_web_search_user_text_query_selection() -> list[Finding]:
    return scan_web_search_user_text_query_text(
        rel(VALUE_STRING_LIST_FILE),
        VALUE_STRING_LIST_FILE.read_text(encoding="utf-8"),
    )


def scan_web_search_user_text_query_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for function_name in (
        "web_search_summary_deterministic_plan_result",
        "web_search_query_from_route",
    ):
        block = rust_private_or_pub_function_block(text, function_name)
        if block is None:
            continue
        block_start, block_text = block
        for offset, line in enumerate(block_text.splitlines(), start=0):
            if (
                "first_quoted_search_query" not in line
                and "user_text.trim()" not in line
                and "unwrap_or_else(|| user_text" not in line
                and "nonempty_search_query(&route.resolved_intent)" not in line
                and ".resolved_intent" not in line
            ):
                continue
            findings.append(
                Finding(
                    rel_path,
                    block_start + offset,
                    "web_search_user_text_query_selection",
                    line.strip(),
                )
            )
    for line_no, line in enumerate(text.splitlines(), start=1):
        if "fn first_quoted_search_query" not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                line_no,
                "web_search_user_text_query_selection",
                line.strip(),
            )
        )
    return findings


def scan_runtime_surface_user_text_token_selection() -> list[Finding]:
    return scan_runtime_surface_user_text_token_text(
        rel(RUNTIME_SURFACE_PLAN_FILE),
        RUNTIME_SURFACE_PLAN_FILE.read_text(encoding="utf-8"),
    )


def scan_runtime_surface_user_text_token_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for function_name in (
        "runtime_surface_mentions_any_machine_token",
        "runtime_surface_mentions_all_machine_token_groups",
        "runtime_surface_mentions_all_exact_machine_token_groups",
        "runtime_surface_mentions_any_exact_machine_token",
    ):
        block = rust_private_or_pub_function_block(text, function_name)
        if block is None:
            continue
        block_start, block_text = block
        for offset, line in enumerate(block_text.splitlines(), start=0):
            if "user_text" not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    block_start + offset,
                    "runtime_surface_user_text_token_selection",
                    line.strip(),
                )
            )
    return findings


def scan_config_change_preview_user_text_selection() -> list[Finding]:
    return scan_config_change_preview_user_text_selection_text(
        rel(READ_RANGE_ACTION_FILE),
        READ_RANGE_ACTION_FILE.read_text(encoding="utf-8"),
    )


def scan_config_change_preview_user_text_selection_text(
    rel_path: str,
    text: str,
) -> list[Finding]:
    findings: list[Finding] = []
    for function_name in ("parse_config_change_preview", "config_change_preview_path"):
        block = rust_private_or_pub_function_block(text, function_name)
        if block is None:
            continue
        block_start, block_text = block
        for offset, line in enumerate(block_text.splitlines(), start=0):
            if (
                "extract_dotted_field_selector" not in line
                and "parse_config_change_value_after_field" not in line
                and "extract_explicit_locator_for_fallback" not in line
            ):
                continue
            findings.append(
                Finding(
                    rel_path,
                    block_start + offset,
                    "config_change_preview_user_text_selection",
                    line.strip(),
                )
            )
    return findings


def rust_private_or_pub_function_block(text: str, function_name: str) -> tuple[int, str] | None:
    pattern = re.compile(
        rf"^(?:pub\(super\)\s+)?fn\s+{re.escape(function_name)}\b", re.MULTILINE
    )
    match = pattern.search(text)
    if not match:
        return None
    start_line = text.count("\n", 0, match.start()) + 1
    next_match = re.search(r"^(?:pub\(super\)\s+)?fn\s+", text[match.end() :], re.MULTILINE)
    end = match.end() + next_match.start() if next_match else len(text)
    return start_line, text[match.start() : end]


def function_block(text: str, function_name: str) -> tuple[int, str] | None:
    pattern = re.compile(rf"^pub\(super\)\s+fn\s+{re.escape(function_name)}\b", re.MULTILINE)
    match = pattern.search(text)
    if not match:
        return None
    start_line = text.count("\n", 0, match.start()) + 1
    next_match = re.search(r"^pub\(super\)\s+fn\s+", text[match.end() :], re.MULTILINE)
    end = match.end() + next_match.start() if next_match else len(text)
    return start_line, text[match.start() : end]


def scan_finalizer_observed_output_registry_bridge_markers() -> list[Finding]:
    findings: list[Finding] = []
    for root in FINALIZER_OBSERVED_OUTPUT_SCAN_ROOTS:
        paths = [root] if root.is_file() else sorted(root.rglob("*.rs"))
        for path in paths:
            if not path.is_file() or is_test_path(path):
                continue
            findings.extend(
                scan_token_list_text(
                    rel(path),
                    path.read_text(encoding="utf-8"),
                    FORBIDDEN_REGISTRY_BRIDGE_SEMANTIC_ENUMS,
                    "finalizer_observed_registry_bridge_semantic_marker",
                )
            )
    return findings


def scan_token_list_text(
    rel_path: str,
    text: str,
    tokens: tuple[str, ...],
    kind: str,
) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in tokens:
            if token not in line:
                continue
            findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


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
        "if contract_repair_judge_runtime_enabled() {\n"
        '    contract_repair_report.add("semantic_suspect", detail);\n'
        "}\n"
        "if contract_repair_judge_runtime_enabled()\n"
        "        && contract_repair_report.needs_llm_contract_integrity_repair() {}\n",
    )
    assert not allowed_semantic_suspect
    blocked_prompt = scan_prompt_layer_text(
        "prompts/layers/overlays/intent_normalizer_prompt.md",
        "`weather_query`\n",
    )
    assert (
        blocked_prompt
        and blocked_prompt[0].kind == "prompt_layer_ordinary_semantic_token"
    )
    assert not scan_prompt_layer_ordinary_semantic_tokens()
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
    assert not scan_intent_normalizer_schema_ordinary_semantic_tokens()
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
