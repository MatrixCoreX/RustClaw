#!/usr/bin/env python3
"""Guard runtime semantic rewrites do not return after agent-loop migration.

RustClaw's target is that ordinary semantic decisions live in the planner /
agent loop. Production runtime must not reintroduce legacy semantic rewrite
sources or migration-debt control markers.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
MAIN_FILE = SRC_ROOT / "main.rs"
AGENT_ENGINE_FILE = SRC_ROOT / "agent_engine.rs"
PIPELINE_TYPES_FILE = SRC_ROOT / "pipeline_types.rs"
RUNTIME_ASK_MODE_FILE = SRC_ROOT / "runtime/ask_mode.rs"
RUNTIME_TYPES_FILE = SRC_ROOT / "runtime/types.rs"
INTENT_ROUTER_FILE = SRC_ROOT / "intent_router.rs"
PREFERRED_RUN_CMD_FILE = SRC_ROOT / "agent_engine/scalar_count_deterministic_plan.rs"
PREFERRED_STRUCTURED_ACTION_FILE = SRC_ROOT / "agent_engine/preferred_structured_action.rs"
MIGRATION_CLASS_FILE = SRC_ROOT / "agent_engine/migration_class.rs"
ASK_PREPARE_FILE = SRC_ROOT / "worker/ask_prepare.rs"
ASK_PIPELINE_FILE = SRC_ROOT / "worker/ask_pipeline.rs"
ASK_PIPELINE_AUTO_LOCATOR_BINDING_FILE = (
    SRC_ROOT / "worker/ask_pipeline_auto_locator_binding.rs"
)
ASK_PIPELINE_BACKGROUND_LOCATOR_GUARD_FILE = (
    SRC_ROOT / "worker/ask_pipeline_background_locator_guard.rs"
)
ASK_PIPELINE_CONTRACT_REPAIR_FILE = SRC_ROOT / "worker/ask_pipeline_contract_repair.rs"
ASK_PIPELINE_BOUNDARY_PREFLIGHT_FILE = (
    SRC_ROOT / "worker/ask_pipeline_boundary_preflight.rs"
)
ASK_PIPELINE_EXECUTION_CONTEXT_FILE = SRC_ROOT / "worker/ask_pipeline_execution_context.rs"
ASK_PIPELINE_FILE_DELIVERY_FILE = SRC_ROOT / "worker/ask_pipeline_file_delivery.rs"
ASK_PIPELINE_DEFAULT_CONFIG_FILE = SRC_ROOT / "worker/ask_pipeline_default_config.rs"
ASK_PIPELINE_POST_ROUTE_REFINEMENT_FILE = (
    SRC_ROOT / "worker/ask_pipeline_post_route_refinement.rs"
)
ASK_PIPELINE_STRUCTURED_ANCHOR_GUARD_FILE = (
    SRC_ROOT / "worker/ask_pipeline_structured_anchor_guard.rs"
)
TASK_JOURNAL_EVIDENCE_COVERAGE_FILE = SRC_ROOT / "task_journal_evidence_coverage.rs"
TASK_JOURNAL_FILE = SRC_ROOT / "task_journal.rs"
INTENT_ROUTER_OBSERVATION_REPAIR_FILE = SRC_ROOT / "intent_router_observation_repair.rs"
INTENT_ROUTER_CONTRACT_HINT_FILE = SRC_ROOT / "intent_router_contract_hint.rs"
INTENT_ROUTER_EXECUTION_CONTRACT_FILE = SRC_ROOT / "intent_router_execution_contract.rs"
INTENT_ROUTER_RUNTIME_STATUS_RECIPE_FILE = (
    SRC_ROOT / "intent_router_runtime_status_recipe.rs"
)
INTENT_ROUTER_PROMPT_RENDER_FILE = SRC_ROOT / "intent_router_prompt_render.rs"
INTENT_ROUTER_OUTPUT_TYPES_FILE = SRC_ROOT / "intent_router_output_types.rs"
INTENT_ROUTER_ROUTE_TRACE_FILE = SRC_ROOT / "intent_router_route_trace.rs"
INTENT_ROUTER_NORMALIZER_RUN_FILE = SRC_ROOT / "intent_router_normalizer_run.rs"
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
ANSWER_VERIFIER_RUNTIME_FILE = SRC_ROOT / "answer_verifier_runtime.rs"
VERIFIER_FILE = SRC_ROOT / "verifier.rs"
PROMPT_UTILS_OUTPUT_CONTRACT_FILE = SRC_ROOT / "prompt_utils_output_contract.rs"
PROMPT_UTILS_CONTRACT_REPAIR_JUDGE_FILE = (
    SRC_ROOT / "prompt_utils_contract_repair_judge.rs"
)
EXECUTION_RECIPE_SCHEMA_FILES: tuple[Path, ...] = (
    SRC_ROOT / "intent_router_execution_recipe_schema.rs",
    SRC_ROOT / "intent_router_execution_recipe_contract.rs",
    SRC_ROOT / "intent_router_normalizer_schema_core.rs",
    SRC_ROOT / "intent_router_schema_report.rs",
    SRC_ROOT / "intent_router_route_trace.rs",
    SRC_ROOT / "intent_router_contract_repair_report.rs",
)
CONTRACT_MATRIX_FILE = SRC_ROOT / "contract_matrix.rs"
CONTRACT_MATRIX_RUNTIME_FILE = SRC_ROOT / "contract_matrix_runtime.rs"
TASK_CONTEXT_BUILDER_FILE = SRC_ROOT / "task_context_builder.rs"
TASK_CONTRACT_FILE = SRC_ROOT / "task_contract.rs"
SCHEDULE_SERVICE_FILE = SRC_ROOT / "schedule_service.rs"
VALUE_STRING_LIST_FILE = SRC_ROOT / "agent_engine/value_string_list.rs"
RUNTIME_SURFACE_PLAN_FILE = SRC_ROOT / "agent_engine/runtime_surface_plan.rs"
LOOP_CONTROL_FILE = SRC_ROOT / "agent_engine/loop_control.rs"
LOOP_CONTROL_FILESYSTEM_MUTATION_RECOVERY_FILE = (
    SRC_ROOT / "agent_engine/loop_control_filesystem_mutation_recovery.rs"
)
DRY_RUN_CONTRACT_PLAN_FILE = SRC_ROOT / "agent_engine/dry_run_contract_plan.rs"
OBSERVED_OUTPUT_FILE = SRC_ROOT / "agent_engine/observed_output.rs"
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

CONTRACT_REPAIR_LOOP_OBSERVATION_FORBIDDEN_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "contract_repair_mutable_route_result_param",
        re.compile(r"\broute_result\s*:\s*&mut\s+(?:crate::)?RouteResult\b"),
    ),
    (
        "contract_repair_mutable_route_result_binding",
        re.compile(r"\bmut\s+route_result\b"),
    ),
    (
        "contract_repair_route_result_field_assignment",
        re.compile(
            r"\broute_result\.[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?\s*="
        ),
    ),
    (
        "contract_repair_route_result_field_mutation_call",
        re.compile(
            r"\broute_result\.[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?"
            r"\.(?:push|push_str|clear|truncate|extend|insert|remove)\s*\("
        ),
    ),
    (
        "contract_repair_route_gate_mutation",
        re.compile(r"\b(?:route_result\.)?set_(?:clarify|chat|execute)_gate\s*\("),
    ),
    (
        "contract_repair_route_reason_mutation_helper",
        re.compile(r"\b(?:append|push|set)_route_reason(?:_marker)?\s*\("),
    ),
)

POST_ROUTE_BOUNDARY_CANDIDATE_FORBIDDEN_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "post_route_boundary_candidate_string_compare",
        re.compile(r"\bcandidate\s*==\s*\"post_route_"),
    ),
    (
        "post_route_boundary_candidate_string_match",
        re.compile(r"\bmatch\s+candidate\s*\{"),
    ),
)
POST_ROUTE_BOUNDARY_DEFERRAL_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "post_route_boundary_deferral_direct_candidate_push",
        re.compile(
            r"push_pre_loop_clarify_candidate\s*\(\s*pre_loop_clarify_candidates\s*,\s*"
            r'"(?:auto_locator_scalar_file_without_current_locator|directory_file_delivery_requires_structured_selection)"',
            re.DOTALL,
        ),
    ),
    (
        "post_route_boundary_deferral_direct_gate_record",
        re.compile(
            r"PostRouteGateRecord::new\s*\(\s*"
            r'"(?:post_route_auto_locator_scalar_file_deferred_to_agent_loop|post_route_directory_file_delivery_deferred_to_agent_loop)"',
            re.DOTALL,
        ),
    ),
)
POST_ROUTE_BOUNDARY_READY_REASON_CODES: tuple[str, ...] = (
    "post_route_locator_guard_deferred_to_prompt_targets",
    "post_route_structural_file_delivery_bound_target",
)
POST_ROUTE_BOUNDARY_READY_ROUTE_REASONS: tuple[str, ...] = (
    "locator_guard_deferred_to_prompt_targets",
)
POST_ROUTE_BOUNDARY_READY_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "post_route_boundary_ready_direct_gate_record",
        re.compile(
            r"PostRouteGateRecord::new\s*\(\s*"
            r'"(?:post_route_locator_guard_deferred_to_prompt_targets|post_route_structural_file_delivery_bound_target)"',
            re.DOTALL,
        ),
    ),
    (
        "post_route_boundary_ready_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            r'"(?:locator_guard_deferred_to_prompt_targets)"',
            re.DOTALL,
        ),
    ),
)
BOUNDARY_PREFLIGHT_DEFERRAL_TOKENS: tuple[str, ...] = (
    "deictic_memory_only",
    "unbound_model_context_target",
    "bare_topic_model_supplied_locator",
    "implicit_workspace_file_locator",
    "model_completed_workspace_file_locator",
    "inferred_missing_workspace_locator",
    "active_anchor_file_delivery_without_structured_reference",
    "background_only_locator",
    "locatorless_observation",
    "unbound_targeted_evidence",
)
BOUNDARY_PREFLIGHT_REASON_CODES: tuple[str, ...] = (
    "deictic_memory_only_deferred_to_agent_loop",
    "unbound_model_context_target_deferred_to_agent_loop",
    "bare_topic_model_supplied_locator_deferred_to_agent_loop",
    "implicit_workspace_file_locator_deferred_to_agent_loop",
    "model_completed_workspace_file_locator_deferred_to_agent_loop",
    "inferred_missing_workspace_locator_deferred_to_agent_loop",
    "active_anchor_file_delivery_deferred_to_agent_loop",
    "background_only_locator_deferred_to_agent_loop",
    "locatorless_observation_deferred_to_agent_loop",
    "unbound_targeted_evidence_deferred_to_agent_loop",
)


def quoted_token_alternation(tokens: tuple[str, ...]) -> str:
    return "|".join(re.escape(token) for token in tokens)


BOUNDARY_PREFLIGHT_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "boundary_preflight_direct_candidate_push",
        re.compile(
            r"push_pre_loop_clarify_candidate\s*\(\s*pre_loop_clarify_candidates\s*,\s*"
            rf'"(?:{quoted_token_alternation(BOUNDARY_PREFLIGHT_DEFERRAL_TOKENS)})"',
            re.DOTALL,
        ),
    ),
    (
        "boundary_preflight_direct_guard_reason",
        re.compile(
            r"log_route_guard_record\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(BOUNDARY_PREFLIGHT_REASON_CODES)})"',
            re.DOTALL,
        ),
    ),
)
WORKER_LOOP_BOUNDARY_DEFERRAL_TOKENS: tuple[str, ...] = (
    "bare_topic_context_expansion",
    "unbound_existing_file_delivery",
    "directory_file_delivery_without_structured_selection",
    "deictic_bare_locator",
)
WORKER_LOOP_BOUNDARY_REASON_CODES: tuple[str, ...] = (
    "unbound_existing_file_delivery_deferred_to_agent_loop",
    "directory_file_delivery_deferred_to_agent_loop",
    "deictic_bare_locator_deferred_to_agent_loop",
)
WORKER_LOOP_BOUNDARY_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "worker_loop_boundary_direct_candidate_push",
        re.compile(
            r"(?:push_pre_loop_clarify_candidate\s*\(\s*&mut\s+pre_loop_clarify_candidates\s*,"
            r"\s*|pre_loop_clarify_candidates\.push\s*\()\s*"
            rf'"(?:{quoted_token_alternation(WORKER_LOOP_BOUNDARY_DEFERRAL_TOKENS)})"',
            re.DOTALL,
        ),
    ),
    (
        "worker_loop_boundary_direct_guard_reason",
        re.compile(
            r"log_route_guard_record\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(WORKER_LOOP_BOUNDARY_REASON_CODES)})"',
            re.DOTALL,
        ),
    ),
)
WORKER_ROUTE_MARKER_REASON_CODES: tuple[str, ...] = (
    "agent_loop_default_entry",
    "bare_topic_contextual_clarify_sanitized",
    "auto_locator_suppressed_multiple_explicit_paths",
)
WORKER_ROUTE_MARKER_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "worker_route_marker_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(WORKER_ROUTE_MARKER_REASON_CODES)})"',
            re.DOTALL,
        ),
    ),
)
BACKGROUND_LOCATOR_LOOP_RECOVERY_ROUTE_REASONS: tuple[str, ...] = (
    "active_observed_output_loop_recovery",
    "recent_observed_results_background_locator_loop_recovery",
)
BACKGROUND_LOCATOR_LOOP_RECOVERY_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "background_locator_recovery_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(BACKGROUND_LOCATOR_LOOP_RECOVERY_ROUTE_REASONS)})"',
            re.DOTALL,
        ),
    ),
)
STRUCTURED_ANCHOR_EVIDENCE_ROUTE_REASONS: tuple[str, ...] = (
    "structured_anchor_requires_evidence",
)
STRUCTURED_ANCHOR_EVIDENCE_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "structured_anchor_evidence_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(STRUCTURED_ANCHOR_EVIDENCE_ROUTE_REASONS)})"',
            re.DOTALL,
        ),
    ),
)
FILE_DELIVERY_BOUNDARY_REASON_CODES: tuple[str, ...] = (
    "post_route_unresolved_file_delivery_deferred_to_agent_loop",
    "post_route_file_delivery_current_request_locator_deferred_to_loop",
)
FILE_DELIVERY_BOUNDARY_ROUTE_REASONS: tuple[str, ...] = (
    "unresolved_file_delivery_deferred_to_agent_loop",
    "file_delivery_current_request_locator_deferred_to_loop",
)
FILE_DELIVERY_BOUNDARY_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "file_delivery_boundary_direct_gate_reason",
        re.compile(
            r"PostRouteGateRecord::with_owner\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(FILE_DELIVERY_BOUNDARY_REASON_CODES)})"',
            re.DOTALL,
        ),
    ),
    (
        "file_delivery_boundary_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(FILE_DELIVERY_BOUNDARY_ROUTE_REASONS)})"',
            re.DOTALL,
        ),
    ),
)
DEFAULT_CONFIG_CONTRACT_ROUTE_REASONS: tuple[str, ...] = (
    "config_contract_default_main_config_deferred_to_loop",
)
DEFAULT_CONFIG_CONTRACT_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "default_config_contract_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(DEFAULT_CONFIG_CONTRACT_ROUTE_REASONS)})"',
            re.DOTALL,
        ),
    ),
)
EXECUTION_CONTEXT_SANITIZATION_ROUTE_REASONS: tuple[str, ...] = (
    "untrusted_normalizer_freeform_rewrite_removed_from_execution_context",
    "untrusted_normalizer_answer_candidate_removed_from_execution_context",
)
EXECUTION_CONTEXT_SANITIZATION_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "execution_context_sanitization_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(EXECUTION_CONTEXT_SANITIZATION_ROUTE_REASONS)})"',
            re.DOTALL,
        ),
    ),
)
AUTO_LOCATOR_BINDING_ROUTE_REASONS: tuple[str, ...] = (
    "structured_field_read_bound_to_auto_locator",
)
AUTO_LOCATOR_BINDING_FORBIDDEN_BLOCK_PATTERNS: tuple[
    tuple[str, re.Pattern[str]], ...
] = (
    (
        "auto_locator_binding_direct_route_reason",
        re.compile(
            r"append_route_reason\s*\([^;]*?"
            rf'"(?:{quoted_token_alternation(AUTO_LOCATOR_BINDING_ROUTE_REASONS)})"',
            re.DOTALL,
        ),
    ),
)
SUBAGENT_BOUNDARY_TOKENS: tuple[str, ...] = (
    "post_route_subagent_boundary_clarify_deferred_to_agent_loop",
    "subagent_boundary_clarify_deferred_to_agent_loop",
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

PROMPT_LAYERS_ROOT = ROOT / "prompts/layers"
INTENT_NORMALIZER_PROMPT = PROMPT_LAYERS_ROOT / "overlays/intent_normalizer_prompt.md"
INTENT_NORMALIZER_SCHEMA = ROOT / "prompts/schemas/intent_normalizer.schema.json"
BOUNDARY_ENVELOPE_SCHEMA = ROOT / "prompts/schemas/boundary_envelope.schema.json"
CONTRACT_REPAIR_JUDGE_SCHEMA = ROOT / "prompts/schemas/contract_repair_judge.schema.json"
PLANNER_EXECUTION_PROMPT_FILES: tuple[Path, ...] = (
    PROMPT_LAYERS_ROOT / "overlays/loop_incremental_plan_prompt.md",
    PROMPT_LAYERS_ROOT / "overlays/single_plan_execution_prompt.md",
)
BOUNDARY_PROMPT_SCHEMA_NO_LEGACY_SEMANTIC_KIND_FILES: tuple[Path, ...] = (
    INTENT_NORMALIZER_PROMPT,
    INTENT_NORMALIZER_SCHEMA,
    CONTRACT_REPAIR_JUDGE_SCHEMA,
    INTENT_ROUTER_PROMPT_RENDER_FILE,
    PROMPT_LAYERS_ROOT / "vendor_patches/minimax/routing/common.md",
)
CHINA_MODEL_ROUTING_PATCH_FILES: tuple[Path, ...] = (
    PROMPT_LAYERS_ROOT / "vendor_patches/minimax/routing/common.md",
    PROMPT_LAYERS_ROOT / "vendor_patches/mimo/routing/common.md",
)
CHINA_MODEL_ROUTING_PATCH_REQUIRED_TOKENS: tuple[str, ...] = (
    "Do not emit legacy `decision`",
    "Do not emit `answer_candidate`",
    "planner loop and finalizer",
)
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
FORBIDDEN_NORMALIZER_SCHEMA_ROUTE_AUTHORITY_TOP_LEVEL_FIELDS: frozenset[str] = frozenset(
    {
        "decision",
        "answer_candidate",
        "direct_answer",
        "direct_answer_candidate",
        "planner_execute",
        "route_authority",
        "semantic_route_authority",
    }
)
FORBIDDEN_NORMALIZER_SCHEMA_ROUTE_AUTHORITY_OUTPUT_CONTRACT_FIELDS: frozenset[str] = frozenset(
    {
        "semantic_kind",
        "semantic",
        "semantic_type",
        "semantic_route",
        "semantic_route_kind",
        "semantic_kind_hint",
        "answer_kind",
        "route_kind",
    }
)
FORBIDDEN_BOUNDARY_ENVELOPE_SCHEMA_FIELDS: frozenset[str] = frozenset(
    {
        "raw_user_request",
        "user_prompt",
        "resolved_user_intent",
        "reason",
        "decision",
        "answer_candidate",
        "direct_answer",
        "planner_execute",
        "route_authority",
        "semantic_route_authority",
        "semantic_kind",
        "output_contract",
        "capability_ref",
    }
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
    findings.extend(scan_runtime_journal_route_trace_decision_type())
    findings.extend(scan_first_layer_decision_test_only_boundary())
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


def scan_prompt_layer_ordinary_semantic_tokens() -> list[Finding]:
    findings: list[Finding] = []
    for path in sorted(PROMPT_LAYERS_ROOT.rglob("*.md")):
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        findings.extend(scan_prompt_layer_text(rel_path, text))
    return findings


def scan_planner_prompt_legacy_semantic_kind_keys() -> list[Finding]:
    findings: list[Finding] = []
    for path in PLANNER_EXECUTION_PROMPT_FILES:
        findings.extend(
            scan_planner_prompt_legacy_semantic_kind_keys_text(
                rel(path),
                path.read_text(encoding="utf-8"),
            )
        )
    return findings


def scan_planner_prompt_legacy_semantic_kind_keys_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if "semantic_kind" not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                line_no,
                "planner_prompt_legacy_semantic_kind_key",
                line.strip(),
            )
        )
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


def scan_intent_normalizer_prompt_contract_marker() -> list[Finding]:
    rel_path = rel(INTENT_NORMALIZER_PROMPT)
    text = INTENT_NORMALIZER_PROMPT.read_text(encoding="utf-8")
    findings: list[Finding] = []
    if "output_contract.contract_marker" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "intent_normalizer_contract_marker_missing",
                "intent normalizer prompt should emit output_contract.contract_marker",
            )
        )
    forbidden_tokens = [
        "Set `output_contract.semantic_kind",
        "`delivery_intent`, `semantic_kind`, `locator_hint`",
    ]
    for token in forbidden_tokens:
        if token not in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "intent_normalizer_semantic_kind_output_target",
                f"forbidden normalizer prompt output target: {token}",
            )
        )
    return findings


def scan_china_model_routing_patch_boundaries() -> list[Finding]:
    findings: list[Finding] = []
    for path in CHINA_MODEL_ROUTING_PATCH_FILES:
        findings.extend(
            scan_china_model_routing_patch_boundaries_text(
                rel(path), path.read_text(encoding="utf-8")
            )
        )
    return findings


def scan_china_model_routing_patch_boundaries_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    for token in CHINA_MODEL_ROUTING_PATCH_REQUIRED_TOKENS:
        if token in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "china_model_routing_boundary_token_missing",
                f"missing required routing boundary token: {token}",
            )
        )
    return findings


def scan_boundary_semantic_kind_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if "semantic_kind" not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                line_no,
                "boundary_prompt_schema_legacy_semantic_kind",
                line.strip(),
            )
        )
    return findings


def scan_boundary_prompt_schema_legacy_semantic_kind_fields() -> list[Finding]:
    findings: list[Finding] = []
    for path in BOUNDARY_PROMPT_SCHEMA_NO_LEGACY_SEMANTIC_KIND_FILES:
        findings.extend(
            scan_boundary_semantic_kind_text(rel(path), path.read_text(encoding="utf-8"))
        )
    return findings


def scan_intent_normalizer_schema_ordinary_semantic_tokens() -> list[Finding]:
    rel_path = rel(INTENT_NORMALIZER_SCHEMA)
    return scan_schema_text(rel_path, INTENT_NORMALIZER_SCHEMA.read_text(encoding="utf-8"))


def json_object_keys(value: Any) -> set[str]:
    if isinstance(value, dict):
        return set(value)
    return set()


def json_list_values(value: Any) -> set[str]:
    if isinstance(value, list):
        return {item for item in value if isinstance(item, str)}
    return set()


def scan_intent_normalizer_schema_route_authority_fields() -> list[Finding]:
    rel_path = rel(INTENT_NORMALIZER_SCHEMA)
    schema = json.loads(INTENT_NORMALIZER_SCHEMA.read_text(encoding="utf-8"))
    if not isinstance(schema, dict):
        return [
            Finding(
                rel_path,
                1,
                "normalizer_schema_not_object",
                "intent normalizer schema must be a JSON object",
            )
        ]
    return scan_intent_normalizer_schema_route_authority_json(rel_path, schema)


def scan_intent_normalizer_schema_route_authority_json(
    rel_path: str, schema: dict[str, Any]
) -> list[Finding]:
    findings: list[Finding] = []
    top_properties = json_object_keys(schema.get("properties"))
    top_required = json_list_values(schema.get("required"))
    output_contract = schema.get("properties", {}).get("output_contract", {})
    output_properties = json_object_keys(output_contract.get("properties"))
    output_required = json_list_values(output_contract.get("required"))

    for field in sorted(
        top_properties & FORBIDDEN_NORMALIZER_SCHEMA_ROUTE_AUTHORITY_TOP_LEVEL_FIELDS
    ):
        findings.append(
            Finding(
                rel_path,
                1,
                "normalizer_schema_route_authority_top_level_field",
                field,
            )
        )
    for field in sorted(
        top_required & FORBIDDEN_NORMALIZER_SCHEMA_ROUTE_AUTHORITY_TOP_LEVEL_FIELDS
    ):
        findings.append(
            Finding(
                rel_path,
                1,
                "normalizer_schema_route_authority_top_level_required",
                field,
            )
        )
    for field in sorted(
        output_properties
        & FORBIDDEN_NORMALIZER_SCHEMA_ROUTE_AUTHORITY_OUTPUT_CONTRACT_FIELDS
    ):
        findings.append(
            Finding(
                rel_path,
                1,
                "normalizer_schema_route_authority_output_contract_field",
                field,
            )
        )
    for field in sorted(
        output_required
        & FORBIDDEN_NORMALIZER_SCHEMA_ROUTE_AUTHORITY_OUTPUT_CONTRACT_FIELDS
    ):
        findings.append(
            Finding(
                rel_path,
                1,
                "normalizer_schema_route_authority_output_contract_required",
                field,
            )
        )
    return findings


def scan_boundary_envelope_schema_machine_only() -> list[Finding]:
    rel_path = rel(BOUNDARY_ENVELOPE_SCHEMA)
    schema = json.loads(BOUNDARY_ENVELOPE_SCHEMA.read_text(encoding="utf-8"))
    if not isinstance(schema, dict):
        return [
            Finding(
                rel_path,
                1,
                "boundary_envelope_schema_not_object",
                "BoundaryEnvelope schema must be a JSON object",
            )
        ]
    return scan_boundary_envelope_schema_json(rel_path, schema)


def scan_boundary_envelope_schema_json(
    rel_path: str, schema: dict[str, Any]
) -> list[Finding]:
    findings: list[Finding] = []
    properties = json_object_keys(schema.get("properties"))
    required = json_list_values(schema.get("required"))
    forbidden = (properties | required) & FORBIDDEN_BOUNDARY_ENVELOPE_SCHEMA_FIELDS

    if schema.get("additionalProperties") is not False:
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_envelope_schema_not_closed",
                "BoundaryEnvelope schema must set additionalProperties=false",
            )
        )
    if "raw_chars" not in properties:
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_envelope_raw_chars_missing",
                "BoundaryEnvelope schema must expose raw_chars count",
            )
        )
    for field in sorted(forbidden):
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_envelope_forbidden_field",
                field,
            )
        )
    return findings


def scan_boundary_envelope_rust_type_machine_only() -> list[Finding]:
    rel_path = rel(INTENT_ROUTER_OUTPUT_TYPES_FILE)
    return scan_boundary_envelope_rust_type_text(
        rel_path,
        INTENT_ROUTER_OUTPUT_TYPES_FILE.read_text(encoding="utf-8"),
    )


def scan_boundary_envelope_rust_type_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    normalizer_output_match = re.search(
        r"struct\s+IntentNormalizerOutput\s*\{(?P<body>.*?)\n\}",
        text,
        flags=re.DOTALL,
    )
    if normalizer_output_match:
        normalizer_output_body = normalizer_output_match.group("body")
        normalizer_output_body_start = normalizer_output_match.start("body")
        for field, kind, message in (
            (
                "raw_user_request",
                "intent_normalizer_output_raw_user_request_field",
                "IntentNormalizerOutput must not carry raw_user_request; use BoundaryEnvelope.raw_chars",
            ),
            (
                "attachment_processing_required",
                "intent_normalizer_output_attachment_required_field",
                "IntentNormalizerOutput must not keep attachment_processing_required after BoundaryEnvelope projection",
            ),
            (
                "route_trace_decision",
                "intent_normalizer_output_route_trace_decision_field",
                "IntentNormalizerOutput must not duplicate route_trace_decision outside RouteTraceRecord",
            ),
        ):
            field_offset = normalizer_output_body.find(field)
            if field_offset >= 0:
                findings.append(
                    Finding(
                        rel_path,
                        text.count("\n", 0, normalizer_output_body_start + field_offset) + 1,
                        kind,
                        message,
                    )
                )
    match = re.search(
        r"struct\s+BoundaryEnvelope\s*\{(?P<body>.*?)\n\}",
        text,
        flags=re.DOTALL,
    )
    if not match:
        return [
            Finding(
                rel_path,
                1,
                "boundary_envelope_rust_struct_missing",
                "BoundaryEnvelope struct not found",
            )
        ]

    body = match.group("body")
    body_start = match.start("body")
    raw_offset = body.find("raw_user_request")
    if raw_offset >= 0:
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, body_start + raw_offset) + 1,
                "boundary_envelope_rust_raw_user_request_field",
                "BoundaryEnvelope must not carry raw_user_request",
            )
        )
    if not re.search(r"\braw_chars\s*:\s*usize\b", body):
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, match.start()) + 1,
                "boundary_envelope_rust_raw_chars_missing",
                "BoundaryEnvelope must expose raw_chars: usize",
            )
        )
    return findings


def scan_route_trace_record_decision_type() -> list[Finding]:
    rel_path = rel(INTENT_ROUTER_ROUTE_TRACE_FILE)
    return scan_route_trace_record_decision_type_text(
        rel_path,
        INTENT_ROUTER_ROUTE_TRACE_FILE.read_text(encoding="utf-8"),
    )


def scan_route_trace_record_decision_type_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    if "enum RouteTraceDecision" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "route_trace_decision_enum_missing",
                "RouteTraceDecision enum is required for route-trace compatibility",
            )
        )
    match = re.search(
        r"struct\s+RouteTraceRecord\s*\{(?P<body>.*?)\n\}",
        text,
        flags=re.DOTALL,
    )
    if not match:
        return findings
    body = match.group("body")
    body_start = match.start("body")
    field_offset = body.find("route_trace_decision")
    if field_offset >= 0 and "FirstLayerDecision" in body[field_offset:]:
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, body_start + field_offset) + 1,
                "route_trace_record_first_layer_decision_field",
                "RouteTraceRecord must use RouteTraceDecision, not FirstLayerDecision",
            )
        )
    return findings


def scan_normalizer_run_route_trace_decision_type() -> list[Finding]:
    rel_path = rel(INTENT_ROUTER_NORMALIZER_RUN_FILE)
    return scan_normalizer_run_route_trace_decision_type_text(
        rel_path,
        INTENT_ROUTER_NORMALIZER_RUN_FILE.read_text(encoding="utf-8"),
    )


def scan_normalizer_run_route_trace_decision_type_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    match = re.search(
        r"fn\s+route_trace_decision_from_state\b(?P<body>.*?)(?=\nfn\s+route_trace_label_|\Z)",
        text,
        flags=re.DOTALL,
    )
    if not match:
        return [
            Finding(
                rel_path,
                1,
                "normalizer_route_trace_decision_helper_missing",
                "route_trace_decision_from_state helper not found",
            )
        ]
    body = match.group("body")
    body_start = match.start("body")
    signature = body.split("{", 1)[0]
    if "-> FirstLayerDecision" in signature:
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, body_start + signature.find("FirstLayerDecision")) + 1,
                "normalizer_route_trace_first_layer_return_type",
                "route_trace_decision_from_state must return RouteTraceDecision",
            )
        )
    first_layer_offset = body.find("FirstLayerDecision::")
    if first_layer_offset >= 0:
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, body_start + first_layer_offset) + 1,
                "normalizer_route_trace_first_layer_variant",
                "normalizer route-trace derivation must not construct FirstLayerDecision variants",
            )
        )
    old_label_offset = text.find("route_label_from_first_layer_decision")
    if old_label_offset >= 0:
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, old_label_offset) + 1,
                "normalizer_route_trace_first_layer_label_helper",
                "normalizer route-trace labels must be derived from RouteTraceDecision",
            )
        )
    return findings


def scan_runtime_journal_route_trace_decision_type() -> list[Finding]:
    findings: list[Finding] = []
    for path in (RUNTIME_ASK_MODE_FILE, PIPELINE_TYPES_FILE):
        findings.extend(
            scan_runtime_journal_route_trace_decision_type_text(
                rel(path),
                path.read_text(encoding="utf-8"),
            )
        )
    return findings


def scan_runtime_journal_route_trace_decision_type_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    for match in re.finditer(
        r"fn\s+route_trace_decision_for_journal\b(?P<body>.*?)(?=\n\s*(?:pub\(crate\)\s+)?fn\s+|\n\}|\Z)",
        text,
        flags=re.DOTALL,
    ):
        body = match.group("body")
        body_start = match.start("body")
        signature = body.split("{", 1)[0]
        first_layer_in_signature = signature.find("FirstLayerDecision")
        if first_layer_in_signature >= 0:
            findings.append(
                Finding(
                    rel_path,
                    text.count("\n", 0, body_start + first_layer_in_signature) + 1,
                    "runtime_journal_route_trace_first_layer_return_type",
                    "route_trace_decision_for_journal must return a route-trace token enum",
                )
            )
        first_layer_variant = body.find("FirstLayerDecision::")
        if first_layer_variant >= 0:
            findings.append(
                Finding(
                    rel_path,
                    text.count("\n", 0, body_start + first_layer_variant) + 1,
                    "runtime_journal_route_trace_first_layer_variant",
                    "route_trace_decision_for_journal must not construct FirstLayerDecision variants",
                )
            )
    return findings


def scan_first_layer_decision_test_only_boundary() -> list[Finding]:
    findings: list[Finding] = []
    for path in (RUNTIME_TYPES_FILE, MAIN_FILE, INTENT_ROUTER_FILE):
        findings.extend(
            scan_first_layer_decision_test_only_boundary_text(
                rel(path),
                path.read_text(encoding="utf-8"),
            )
        )
    return findings


def preceding_lines_have_cfg_test(text: str, offset: int, line_window: int = 3) -> bool:
    prefix = text[:offset].splitlines()
    recent = prefix[-line_window:]
    return any("#[cfg(test)]" in line for line in recent)


def scan_first_layer_decision_test_only_boundary_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    checks: tuple[tuple[str, str, str], ...] = (
        (
            r"\benum\s+FirstLayerDecision\b",
            "first_layer_decision_enum_not_test_only",
            "FirstLayerDecision enum must remain test-only",
        ),
        (
            r"\bpub\(crate\)\s+use\s+runtime::types::FirstLayerDecision\b",
            "first_layer_decision_crate_reexport_not_test_only",
            "crate-root FirstLayerDecision re-export must remain test-only",
        ),
        (
            r"\buse\s+crate::FirstLayerDecision\b",
            "first_layer_decision_import_not_test_only",
            "intent-router FirstLayerDecision import must remain test-only",
        ),
    )
    for pattern, kind, message in checks:
        for match in re.finditer(pattern, text):
            if preceding_lines_have_cfg_test(text, match.start()):
                continue
            findings.append(
                Finding(
                    rel_path,
                    text.count("\n", 0, match.start()) + 1,
                    kind,
                    message,
                )
            )
    return findings


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


def scan_contract_repair_loop_observation_boundary() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_CONTRACT_REPAIR_FILE)
    return scan_contract_repair_loop_observation_boundary_text(
        rel_path,
        ASK_PIPELINE_CONTRACT_REPAIR_FILE.read_text(encoding="utf-8"),
    )


def scan_contract_repair_loop_observation_boundary_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in CONTRACT_REPAIR_LOOP_OBSERVATION_FORBIDDEN_PATTERNS:
            if pattern.search(line):
                findings.append(Finding(rel_path, line_no, kind, line.strip()))
    return findings


def scan_post_route_boundary_candidate_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_POST_ROUTE_REFINEMENT_FILE)
    return scan_post_route_boundary_candidate_typing_text(
        rel_path,
        ASK_PIPELINE_POST_ROUTE_REFINEMENT_FILE.read_text(encoding="utf-8"),
    )


def scan_post_route_boundary_candidate_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum BoundaryClarifyCandidate" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "post_route_boundary_candidate_enum_missing",
                "BoundaryClarifyCandidate enum is required for post-route boundary candidates",
            )
        )
    if "enum BoundaryContractDeferral" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "post_route_boundary_deferral_enum_missing",
                "BoundaryContractDeferral enum is required for post-route boundary deferrals",
            )
        )
    if "enum PostRouteBoundaryReady" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "post_route_boundary_ready_enum_missing",
                "PostRouteBoundaryReady enum is required for post-route boundary-ready records",
            )
        )
    for line_no, line in enumerate(text.splitlines(), start=1):
        for kind, pattern in POST_ROUTE_BOUNDARY_CANDIDATE_FORBIDDEN_PATTERNS:
            if pattern.search(line):
                findings.append(Finding(rel_path, line_no, kind, line.strip()))
    for kind, pattern in POST_ROUTE_BOUNDARY_DEFERRAL_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    for kind, pattern in POST_ROUTE_BOUNDARY_READY_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_boundary_preflight_deferral_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_BOUNDARY_PREFLIGHT_FILE)
    return scan_boundary_preflight_deferral_typing_text(
        rel_path,
        ASK_PIPELINE_BOUNDARY_PREFLIGHT_FILE.read_text(encoding="utf-8"),
    )


def scan_boundary_preflight_deferral_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum BoundaryPreflightDeferral" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_preflight_deferral_enum_missing",
                "BoundaryPreflightDeferral enum is required for boundary preflight deferrals",
            )
        )
    for kind, pattern in BOUNDARY_PREFLIGHT_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_worker_loop_boundary_deferral_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_FILE)
    return scan_worker_loop_boundary_deferral_typing_text(
        rel_path,
        ASK_PIPELINE_FILE.read_text(encoding="utf-8"),
    )


def scan_worker_loop_boundary_deferral_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum WorkerLoopBoundaryDeferral" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "worker_loop_boundary_deferral_enum_missing",
                "WorkerLoopBoundaryDeferral enum is required for main worker boundary deferrals",
            )
        )
    for kind, pattern in WORKER_LOOP_BOUNDARY_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_worker_route_marker_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_FILE)
    return scan_worker_route_marker_typing_text(
        rel_path,
        ASK_PIPELINE_FILE.read_text(encoding="utf-8"),
    )


def scan_worker_route_marker_typing_text(rel_path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    if "enum WorkerRouteMarker" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "worker_route_marker_enum_missing",
                "WorkerRouteMarker enum is required for main worker route markers",
            )
        )
    for kind, pattern in WORKER_ROUTE_MARKER_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_background_locator_loop_recovery_marker_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_BACKGROUND_LOCATOR_GUARD_FILE)
    return scan_background_locator_loop_recovery_marker_typing_text(
        rel_path,
        ASK_PIPELINE_BACKGROUND_LOCATOR_GUARD_FILE.read_text(encoding="utf-8"),
    )


def scan_background_locator_loop_recovery_marker_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum BackgroundLocatorLoopRecoveryMarker" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "background_locator_recovery_marker_enum_missing",
                "BackgroundLocatorLoopRecoveryMarker enum is required for background locator recovery markers",
            )
        )
    for kind, pattern in BACKGROUND_LOCATOR_LOOP_RECOVERY_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_structured_anchor_evidence_marker_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_STRUCTURED_ANCHOR_GUARD_FILE)
    return scan_structured_anchor_evidence_marker_typing_text(
        rel_path,
        ASK_PIPELINE_STRUCTURED_ANCHOR_GUARD_FILE.read_text(encoding="utf-8"),
    )


def scan_structured_anchor_evidence_marker_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum StructuredAnchorEvidenceMarker" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "structured_anchor_evidence_marker_enum_missing",
                "StructuredAnchorEvidenceMarker enum is required for structured anchor evidence markers",
            )
        )
    for kind, pattern in STRUCTURED_ANCHOR_EVIDENCE_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_subagent_boundary_deferral_helper() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_FILE)
    return scan_subagent_boundary_deferral_helper_text(
        rel_path,
        ASK_PIPELINE_FILE.read_text(encoding="utf-8"),
    )


def scan_subagent_boundary_deferral_helper_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    helper = rust_private_or_pub_function_block(
        text, "defer_subagent_boundary_clarify_to_agent_loop"
    )
    if helper is None:
        findings.append(
            Finding(
                rel_path,
                1,
                "subagent_boundary_deferral_helper_missing",
                "defer_subagent_boundary_clarify_to_agent_loop helper is required",
            )
        )
        helper_start = helper_end = -1
    else:
        helper_start, helper_text = helper
        helper_end = helper_start + helper_text.count("\n")
    for line_no, line in enumerate(text.splitlines(), start=1):
        if helper_start <= line_no <= helper_end:
            continue
        for token in SUBAGENT_BOUNDARY_TOKENS:
            if f'"{token}"' in line:
                findings.append(
                    Finding(
                        rel_path,
                        line_no,
                        "subagent_boundary_deferral_token_outside_helper",
                        line.strip(),
                    )
                )
    return findings


def scan_file_delivery_boundary_deferral_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_FILE_DELIVERY_FILE)
    return scan_file_delivery_boundary_deferral_typing_text(
        rel_path,
        ASK_PIPELINE_FILE_DELIVERY_FILE.read_text(encoding="utf-8"),
    )


def scan_file_delivery_boundary_deferral_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum FileDeliveryBoundaryDeferral" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "file_delivery_boundary_deferral_enum_missing",
                "FileDeliveryBoundaryDeferral enum is required for file-delivery boundary deferrals",
            )
        )
    for kind, pattern in FILE_DELIVERY_BOUNDARY_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_default_config_contract_deferral_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_DEFAULT_CONFIG_FILE)
    return scan_default_config_contract_deferral_typing_text(
        rel_path,
        ASK_PIPELINE_DEFAULT_CONFIG_FILE.read_text(encoding="utf-8"),
    )


def scan_default_config_contract_deferral_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum DefaultConfigContractDeferral" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "default_config_contract_deferral_enum_missing",
                "DefaultConfigContractDeferral enum is required for default config contract deferrals",
            )
        )
    for kind, pattern in DEFAULT_CONFIG_CONTRACT_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_execution_context_sanitization_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_EXECUTION_CONTEXT_FILE)
    return scan_execution_context_sanitization_typing_text(
        rel_path,
        ASK_PIPELINE_EXECUTION_CONTEXT_FILE.read_text(encoding="utf-8"),
    )


def scan_execution_context_sanitization_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum ExecutionContextSanitization" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "execution_context_sanitization_enum_missing",
                "ExecutionContextSanitization enum is required for execution-context sanitization markers",
            )
        )
    for kind, pattern in EXECUTION_CONTEXT_SANITIZATION_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
    return findings


def scan_auto_locator_binding_marker_typing() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_AUTO_LOCATOR_BINDING_FILE)
    return scan_auto_locator_binding_marker_typing_text(
        rel_path,
        ASK_PIPELINE_AUTO_LOCATOR_BINDING_FILE.read_text(encoding="utf-8"),
    )


def scan_auto_locator_binding_marker_typing_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    if "enum AutoLocatorBindingMarker" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "auto_locator_binding_marker_enum_missing",
                "AutoLocatorBindingMarker enum is required for auto-locator binding markers",
            )
        )
    for kind, pattern in AUTO_LOCATOR_BINDING_FORBIDDEN_BLOCK_PATTERNS:
        for match in pattern.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(Finding(rel_path, line_no, kind, match.group(0).strip()))
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
        if not path.exists():
            continue
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


def scan_answer_verifier_output_contract_prompt_marker() -> list[Finding]:
    rel_path = rel(ANSWER_VERIFIER_RUNTIME_FILE)
    text = ANSWER_VERIFIER_RUNTIME_FILE.read_text(encoding="utf-8")
    fn_start = text.find("pub(super) fn output_contract_prompt_block(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "answer_verifier_output_contract_prompt_missing",
                "missing output_contract_prompt_block",
            )
        ]
    fn_end = text.find("\nfn verifier_contract_matrix_prompt_trace", fn_start)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if '"contract_marker": route_result.effective_output_contract_semantic_kind().as_str()' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "answer_verifier_contract_marker_missing",
                "answer verifier output contract prompt should expose contract_marker",
            )
        )
    if '"semantic_kind": route_result.effective_output_contract_semantic_kind().as_str()' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "answer_verifier_semantic_kind_emission",
                "answer verifier output contract prompt must not expose legacy semantic_kind",
            )
        )
    return findings


def scan_verifier_contract_missing_detail_marker() -> list[Finding]:
    rel_path = rel(VERIFIER_FILE)
    text = VERIFIER_FILE.read_text(encoding="utf-8")
    findings: list[Finding] = []
    if "error_code=evidence_policy_entry_missing contract_marker=" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "verifier_contract_marker_detail_missing",
                "evidence-policy missing verifier detail should emit machine fields with contract_marker",
            )
        )
    if "no contract matrix entry matched semantic kind" in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "verifier_semantic_kind_detail",
                "contract-missing verifier detail must not name legacy semantic_kind",
            )
        )
    return findings


def scan_route_guard_record_contract_marker() -> list[Finding]:
    rel_path = rel(ASK_PIPELINE_FILE)
    text = ASK_PIPELINE_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn log_route_guard_record(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "route_guard_record_missing",
                "missing log_route_guard_record",
            )
        ]
    fn_end = text.find("\nfn ", fn_start + 1)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if "contract_marker={}" not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "route_guard_record_contract_marker_missing",
                "route guard record should log contract_marker, not legacy semantic_kind",
            )
        )
    if "semantic_kind={}" in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "route_guard_record_semantic_kind_field",
                "route guard record must not expose legacy semantic_kind field name",
            )
        )
    return findings


def scan_loop_control_output_contract_marker_key() -> list[Finding]:
    rel_path = rel(LOOP_CONTROL_FILE)
    text = LOOP_CONTROL_FILE.read_text(encoding="utf-8")
    findings: list[Finding] = []
    if '"agent_loop.effective_output_contract_marker"' not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "loop_control_contract_marker_key_missing",
                "loop output vars should expose effective_output_contract_marker",
            )
        )
    if '"agent_loop.effective_output_contract_semantic_kind"' in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "loop_control_semantic_kind_key",
                "loop output vars must not expose legacy effective_output_contract_semantic_kind key",
            )
        )
    return findings


def scan_loop_recovery_contract_marker_fields() -> list[Finding]:
    findings: list[Finding] = []
    loop_text = LOOP_CONTROL_FILE.read_text(encoding="utf-8")
    if 'object.contains_key("contract_marker")' not in loop_text:
        findings.append(
            Finding(
                rel(LOOP_CONTROL_FILE),
                1,
                "loop_control_contract_marker_reader_missing",
                "loop control machine JSON detection should read contract_marker",
            )
        )
    if 'object.contains_key("semantic_kind")' in loop_text:
        findings.append(
            Finding(
                rel(LOOP_CONTROL_FILE),
                1,
                "loop_control_semantic_kind_reader",
                "loop control machine JSON detection must not read legacy semantic_kind",
            )
        )
    fs_text = LOOP_CONTROL_FILESYSTEM_MUTATION_RECOVERY_FILE.read_text(encoding="utf-8")
    if '.get("contract_marker")' not in fs_text:
        findings.append(
            Finding(
                rel(LOOP_CONTROL_FILESYSTEM_MUTATION_RECOVERY_FILE),
                1,
                "filesystem_mutation_recovery_contract_marker_missing",
                "filesystem mutation recovery should read contract_marker",
            )
        )
    if '.get("semantic_kind")' in fs_text:
        findings.append(
            Finding(
                rel(LOOP_CONTROL_FILESYSTEM_MUTATION_RECOVERY_FILE),
                1,
                "filesystem_mutation_recovery_semantic_kind_reader",
                "filesystem mutation recovery must not read legacy semantic_kind",
            )
        )
    return findings


def scan_dry_run_contract_plan_marker_payloads() -> list[Finding]:
    if not DRY_RUN_CONTRACT_PLAN_FILE.exists():
        return []
    rel_path = rel(DRY_RUN_CONTRACT_PLAN_FILE)
    text = DRY_RUN_CONTRACT_PLAN_FILE.read_text(encoding="utf-8")
    findings: list[Finding] = []
    required_markers = [
        '"contract_marker": "answer_verifier_contract_dry_run"',
        '"contract_marker": "task_control_cancel_dry_run"',
        '"contract_marker": "observed_output_projection_dry_run"',
        '"contract_marker": "local_process_cancel_dry_run"',
        '"contract_marker": "async_job_poll_contract_dry_run"',
        '"contract_marker": "finalizer_language_policy_dry_run"',
    ]
    for marker in required_markers:
        if marker in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "dry_run_contract_marker_missing",
                f"missing required dry-run payload marker: {marker}",
            )
        )
    if '"semantic_kind":' in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "dry_run_contract_semantic_kind_payload",
                "dry-run response payloads must use contract_marker, not semantic_kind",
            )
        )
    return findings


def scan_observed_output_contract_marker_payload() -> list[Finding]:
    rel_path = rel(OBSERVED_OUTPUT_FILE)
    text = OBSERVED_OUTPUT_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn observed_contract_json(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "observed_contract_json_missing",
                "missing observed_contract_json",
            )
        ]
    fn_end = text.find("\nfn observed_answer_fallback_prompt_logical_path", fn_start)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if '"contract_marker": route.effective_output_contract_semantic_kind().as_str()' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "observed_contract_marker_missing",
                "observed fallback contract JSON should expose contract_marker",
            )
        )
    if '"semantic_kind": route.effective_output_contract_semantic_kind().as_str()' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "observed_contract_semantic_kind_payload",
                "observed fallback contract JSON must not expose legacy semantic_kind",
            )
        )
    return findings


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


def scan_execution_recipe_contract_marker_outputs() -> list[Finding]:
    path = SRC_ROOT / "intent_router_execution_recipe_contract.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    findings: list[Finding] = []
    if '"contract_marker".to_string()' not in text or '.get("contract_marker")' not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "execution_recipe_contract_marker_missing",
                "execution recipe contract repair should read/write contract_marker",
            )
        )
    forbidden_tokens = [
        '"semantic_kind".to_string()',
        '.get("semantic_kind")',
        "force_output_contract_semantic_kind",
    ]
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in forbidden_tokens:
            if token not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "execution_recipe_contract_semantic_kind_field",
                    line.strip(),
                )
            )
    return findings


def scan_schema_report_contract_marker_fields() -> list[Finding]:
    path = SRC_ROOT / "intent_router_schema_report.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    findings: list[Finding] = []
    required_tokens = [
        '"contract_marker"',
        '"output_contract_marker_normalized"',
        '.get("contract_marker")',
    ]
    for token in required_tokens:
        if token in text:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "schema_report_contract_marker_missing",
                f"missing required schema report token: {token}",
            )
        )
    forbidden_tokens = [
        '.get("semantic_kind")',
        '"output_contract_semantic_kind_normalized"',
    ]
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in forbidden_tokens:
            if token not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "schema_report_semantic_kind_field",
                    line.strip(),
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


def scan_contract_matrix_trace_contract_marker() -> list[Finding]:
    rel_path = rel(CONTRACT_MATRIX_RUNTIME_FILE)
    text = CONTRACT_MATRIX_RUNTIME_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn trace_snapshot_for_output_contract_with_route_reason(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "contract_matrix_trace_snapshot_missing",
                "missing trace_snapshot_for_output_contract_with_route_reason",
            )
        ]
    fn_end = text.find("\nfn ", fn_start + 1)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if '"contract_marker": output_contract.semantic_kind.as_str()' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_matrix_trace_contract_marker_missing",
                "contract matrix trace snapshot should expose contract_marker",
            )
        )
    if '"semantic_kind": output_contract.semantic_kind.as_str()' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_matrix_trace_semantic_kind_field",
                "contract matrix trace snapshot must not expose legacy semantic_kind",
            )
        )
    return findings


def scan_task_journal_step_contract_marker() -> list[Finding]:
    rel_path = rel(TASK_JOURNAL_FILE)
    text = TASK_JOURNAL_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn step_contract_trace_json(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "task_journal_step_contract_trace_missing",
                "missing step_contract_trace_json",
            )
        ]
    fn_end = text.find("\nfn ", fn_start + 1)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if '"contract_marker": contract.get("contract_marker").and_then(Value::as_str)' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "task_journal_step_contract_marker_missing",
                "task journal step contract trace should expose contract_marker",
            )
        )
    if '"semantic_kind": contract.get("semantic_kind").and_then(Value::as_str)' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "task_journal_step_semantic_kind_field",
                "task journal step contract trace must not expose legacy semantic_kind",
            )
        )
    return findings


def scan_schedule_preview_contract_marker() -> list[Finding]:
    rel_path = rel(SCHEDULE_SERVICE_FILE)
    text = SCHEDULE_SERVICE_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn schedule_compile_only_response(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "schedule_compile_only_response_missing",
                "missing schedule_compile_only_response",
            )
        ]
    fn_end = text.find("\npub(crate) async fn try_handle_schedule_request", fn_start)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if '"contract_marker": "schedule_intent_preview"' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "schedule_preview_contract_marker_missing",
                "schedule preview response should expose contract_marker",
            )
        )
    if '"semantic_kind": "schedule_intent_preview"' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "schedule_preview_semantic_kind_field",
                "schedule preview response must not expose legacy semantic_kind",
            )
        )
    return findings


def scan_current_workspace_scope_legacy_semantic_marker_removed() -> list[Finding]:
    rel_path = rel(AGENT_ENGINE_FILE)
    text = AGENT_ENGINE_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn current_workspace_scope_marks_scalar_count(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "current_workspace_scope_marker_reader_missing",
                "missing current_workspace_scope_marks_scalar_count",
            )
        ]
    fn_end = text.find("\nfn ", fn_start + 1)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    if '".chain(["semantic_kind"])' in body or '"semantic_kind"' in body:
        return [
            Finding(
                rel_path,
                1,
                "current_workspace_scope_legacy_semantic_marker",
                "current workspace scope marker reader must not accept legacy semantic_kind",
            )
        ]
    return []


def scan_runtime_status_recipe_contract_marker() -> list[Finding]:
    rel_path = rel(INTENT_ROUTER_RUNTIME_STATUS_RECIPE_FILE)
    text = INTENT_ROUTER_RUNTIME_STATUS_RECIPE_FILE.read_text(encoding="utf-8")
    fn_start = text.find("fn output_contract_declares_scalar_locatorless_observation(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "runtime_status_recipe_contract_reader_missing",
                "missing output_contract_declares_scalar_locatorless_observation",
            )
        ]
    fn_end = text.find("\nfn ", fn_start + 1)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if '.get("contract_marker")' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "runtime_status_recipe_contract_marker_missing",
                "runtime status recipe should read output_contract.contract_marker",
            )
        )
    if '.get("semantic_kind")' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "runtime_status_recipe_semantic_kind_reader",
                "runtime status recipe must not read legacy output_contract.semantic_kind",
            )
        )
    return findings


def scan_prompt_utils_contract_repair_judge_marker_only() -> list[Finding]:
    rel_path = rel(PROMPT_UTILS_CONTRACT_REPAIR_JUDGE_FILE)
    text = PROMPT_UTILS_CONTRACT_REPAIR_JUDGE_FILE.read_text(encoding="utf-8")
    findings: list[Finding] = []
    if '.get("contract_marker")' not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_repair_judge_contract_marker_missing",
                "contract repair judge should read output_contract.contract_marker",
            )
        )
    if 'contract.get("semantic_kind")' in text or '.or_else(|| contract.get("semantic_kind"))' in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_repair_judge_semantic_kind_fallback",
                "contract repair judge must not fall back to legacy output_contract.semantic_kind",
            )
        )
    return findings


def scan_prompt_utils_output_contract_marker_only() -> list[Finding]:
    rel_path = rel(PROMPT_UTILS_OUTPUT_CONTRACT_FILE)
    text = PROMPT_UTILS_OUTPUT_CONTRACT_FILE.read_text(encoding="utf-8")
    fn_start = text.find("pub(super) fn canonicalize_output_contract(")
    if fn_start < 0:
        return [
            Finding(
                rel_path,
                1,
                "prompt_utils_output_contract_canonicalizer_missing",
                "missing canonicalize_output_contract",
            )
        ]
    fn_end = text.find("\nfn ", fn_start + 1)
    body = text[fn_start : fn_end if fn_end >= 0 else len(text)]
    findings: list[Finding] = []
    if '"contract_marker"' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "prompt_utils_output_contract_marker_missing",
                "output contract canonicalizer should preserve contract_marker",
            )
        )
    for line_no, line in enumerate(body.splitlines(), start=1):
        if '"semantic_kind"' not in line:
            continue
        findings.append(
            Finding(
                rel_path,
                line_no,
                "prompt_utils_output_contract_semantic_kind_field",
                line.strip(),
            )
        )
    return findings


def scan_intent_router_output_contract_schema_marker_only() -> list[Finding]:
    path = SRC_ROOT / "intent_router_output_contract_schema.rs"
    rel_path = rel(path)
    text = path.read_text(encoding="utf-8")
    findings: list[Finding] = []
    if '"contract_marker"' not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "intent_router_output_contract_schema_marker_missing",
                "normalizer output contract schema should read/write contract_marker",
            )
        )
    forbidden_tokens = [
        '"semantic_kind"',
        '"semantic_kind".to_string()',
        '.get("semantic_kind")',
        'contains_key("semantic_kind")',
    ]
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in forbidden_tokens:
            if token not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "intent_router_output_contract_schema_semantic_kind_field",
                    line.strip(),
                )
            )
    return findings


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
    if not RUNTIME_SURFACE_PLAN_FILE.exists():
        return []
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
        "struct IntentNormalizerOutput {\n    boundary_envelope: BoundaryEnvelope,\n}\n"
        "struct BoundaryEnvelope {\n    raw_chars: usize,\n}\n",
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
        and blocked_boundary_envelope_raw_text[0].kind
        == "boundary_envelope_forbidden_field"
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
    blocked_boundary_envelope_rust_raw_text = scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "pub(crate) struct BoundaryEnvelope {\n    pub(crate) raw_user_request: String,\n}",
    )
    assert (
        blocked_boundary_envelope_rust_raw_text
        and blocked_boundary_envelope_rust_raw_text[0].kind
        == "boundary_envelope_rust_raw_user_request_field"
    )
    blocked_boundary_envelope_rust_missing_raw_chars = scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "pub(crate) struct BoundaryEnvelope {\n    pub(crate) session_binding: Option<String>,\n}",
    )
    assert any(
        item.kind == "boundary_envelope_rust_raw_chars_missing"
        for item in blocked_boundary_envelope_rust_missing_raw_chars
    )
    assert not scan_boundary_envelope_rust_type_text(
        "crates/clawd/src/intent_router_output_types.rs",
        "pub(crate) struct BoundaryEnvelope {\n    pub(crate) raw_chars: usize,\n}",
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
