#!/usr/bin/env python3
"""Prompt, schema, and route-trace guards for runtime semantic rewrite checks."""

from __future__ import annotations

import dataclasses
import json
import re
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
MAIN_FILE = SRC_ROOT / "main.rs"
PIPELINE_TYPES_FILE = SRC_ROOT / "pipeline_types.rs"
RUNTIME_ASK_MODE_FILE = SRC_ROOT / "runtime/ask_mode.rs"
RUNTIME_TYPES_FILE = SRC_ROOT / "runtime/types.rs"
INTENT_ROUTER_FILE = SRC_ROOT / "intent_router.rs"
INTENT_ROUTER_CONTRACT_REPAIR_JUDGE_FILE = (
    SRC_ROOT / "intent_router_contract_repair_judge.rs"
)
ASK_PIPELINE_FILE = SRC_ROOT / "worker/ask_pipeline.rs"
INTENT_ROUTER_PROMPT_RENDER_FILE = SRC_ROOT / "intent_router_prompt_render.rs"
INTENT_ROUTER_OUTPUT_TYPES_FILE = SRC_ROOT / "intent_router_output_types.rs"
INTENT_ROUTER_ROUTE_TRACE_FILE = SRC_ROOT / "intent_router_route_trace.rs"
INTENT_ROUTER_NORMALIZER_RUN_FILE = SRC_ROOT / "intent_router_normalizer_run.rs"

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
FORBIDDEN_LEGACY_ROUTE_TRACE_REASON_TOKENS: tuple[str, ...] = (
    "direct_answer_trace_inferred",
    "planner_execute_trace_inferred",
)
FORBIDDEN_NORMALIZER_ROUTE_TRACE_LABELS: tuple[str, ...] = (
    "AskClarify",
    "ChatAct",
    "Chat",
    "Act",
)
FORBIDDEN_ASK_MODE_ROUTE_TRACE_LABELS: tuple[str, ...] = (
    "AskClarify",
    "ChatAct",
    "Chat",
    "Act",
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def rel(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


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
    findings: list[Finding] = []
    forbidden_tokens = [
        "Always emit boundary schema keys",
        "Always include boundary schema keys",
        "Set `output_contract.contract_marker",
        "set `output_contract.contract_marker",
        "set output_contract.contract_marker",
        "Set `output_contract.semantic_kind",
        "`delivery_intent`, `semantic_kind`, `locator_hint`",
    ]
    for path in (INTENT_NORMALIZER_PROMPT, *CHINA_MODEL_ROUTING_PATCH_FILES):
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        if "Prefer the compact `boundary_envelope`" not in text:
            findings.append(
                Finding(
                    rel_path,
                    1,
                    "intent_normalizer_boundary_envelope_not_primary",
                    "intent normalizer prompt should make boundary_envelope the primary output",
                )
            )
        if "Runtime fills missing compatibility schema slots with neutral defaults" not in text:
            findings.append(
                Finding(
                    rel_path,
                    1,
                    "intent_normalizer_compat_defaults_missing",
                    "intent normalizer prompt should rely on runtime-filled compatibility defaults",
                )
            )
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
    schema_version = schema.get("properties", {}).get("schema_version", {})
    if not isinstance(schema_version, dict) or schema_version.get("const") != 1:
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_envelope_schema_version_missing",
                "BoundaryEnvelope schema must expose schema_version const=1",
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
    if "BOUNDARY_ENVELOPE_SCHEMA_VERSION: u8 = 1" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_envelope_schema_version_const_missing",
                "BoundaryEnvelope Rust type must expose BOUNDARY_ENVELOPE_SCHEMA_VERSION = 1",
            )
        )
    if "fn schema_version(&self) -> u8" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "boundary_envelope_schema_version_method_missing",
                "BoundaryEnvelope must expose schema_version()",
            )
        )
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


def scan_normalizer_route_trace_label_tokens() -> list[Finding]:
    return scan_normalizer_route_trace_label_tokens_text(
        rel(INTENT_ROUTER_NORMALIZER_RUN_FILE),
        INTENT_ROUTER_NORMALIZER_RUN_FILE.read_text(encoding="utf-8"),
    )


def scan_normalizer_route_trace_label_tokens_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    match = re.search(
        r"fn\s+route_trace_label_from_decision\b(?P<body>.*?)(?=\nfn\s+route_trace_label_from_state\b|\Z)",
        text,
        flags=re.DOTALL,
    )
    if not match:
        return [
            Finding(
                rel_path,
                1,
                "normalizer_route_trace_label_helper_missing",
                "route_trace_label_from_decision helper not found",
            )
        ]
    body = match.group("body")
    body_start = match.start("body")
    for token in FORBIDDEN_NORMALIZER_ROUTE_TRACE_LABELS:
        pattern = f'"{token}"'
        offset = body.find(pattern)
        if offset < 0:
            continue
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, body_start + offset) + 1,
                "normalizer_route_trace_legacy_label",
                pattern,
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


def scan_ask_mode_route_trace_label_tokens() -> list[Finding]:
    return scan_ask_mode_route_trace_label_tokens_text(
        rel(RUNTIME_ASK_MODE_FILE),
        RUNTIME_ASK_MODE_FILE.read_text(encoding="utf-8"),
    )


def scan_ask_mode_route_trace_label_tokens_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    match = re.search(
        r"fn\s+route_trace_label_for_log\b(?P<body>.*?)(?=\n\s*pub\(crate\)\s+fn\s+route_trace_decision_for_journal\b|\Z)",
        text,
        flags=re.DOTALL,
    )
    if not match:
        return [
            Finding(
                rel_path,
                1,
                "ask_mode_route_trace_label_helper_missing",
                "AskMode::route_trace_label_for_log helper not found",
            )
        ]
    body = match.group("body")
    body_start = match.start("body")
    for token in FORBIDDEN_ASK_MODE_ROUTE_TRACE_LABELS:
        pattern = f'"{token}"'
        offset = body.find(pattern)
        if offset < 0:
            continue
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, body_start + offset) + 1,
                "ask_mode_route_trace_legacy_label",
                pattern,
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


def scan_intent_normalizer_legacy_decision_field_deleted() -> list[Finding]:
    findings: list[Finding] = []
    findings.extend(
        scan_intent_normalizer_legacy_decision_field_deleted_text(
            rel(INTENT_ROUTER_FILE),
            INTENT_ROUTER_FILE.read_text(encoding="utf-8"),
        )
    )
    findings.extend(
        scan_intent_normalizer_legacy_decision_field_deleted_text(
            rel(INTENT_ROUTER_CONTRACT_REPAIR_JUDGE_FILE),
            INTENT_ROUTER_CONTRACT_REPAIR_JUDGE_FILE.read_text(encoding="utf-8"),
        )
    )
    return findings


def scan_intent_normalizer_legacy_decision_field_deleted_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    match = re.search(
        r"struct\s+IntentNormalizerOut\s*\{(?P<body>.*?)\n\}",
        text,
        flags=re.DOTALL,
    )
    if match:
        body = match.group("body")
        body_start = match.start("body")
        field_match = re.search(r"\bdecision\s*:\s*String\b", body)
        if field_match:
            findings.append(
                Finding(
                    rel_path,
                    text.count("\n", 0, body_start + field_match.start()) + 1,
                    "intent_normalizer_out_legacy_decision_field",
                    "IntentNormalizerOut must not keep legacy decision field",
                )
            )
    for assignment in re.finditer(r"\bout\.decision\s*=", text):
        findings.append(
            Finding(
                rel_path,
                text.count("\n", 0, assignment.start()) + 1,
                "intent_normalizer_out_legacy_decision_write",
                "repair code must not write legacy normalizer decision",
            )
        )
    return findings


def scan_legacy_route_trace_reason_tokens() -> list[Finding]:
    findings: list[Finding] = []
    for path in (INTENT_ROUTER_ROUTE_TRACE_FILE, ASK_PIPELINE_FILE):
        findings.extend(
            scan_legacy_route_trace_reason_tokens_text(
                rel(path),
                path.read_text(encoding="utf-8"),
            )
        )
    return findings


def scan_legacy_route_trace_reason_tokens_text(
    rel_path: str, text: str
) -> list[Finding]:
    findings: list[Finding] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        for token in FORBIDDEN_LEGACY_ROUTE_TRACE_REASON_TOKENS:
            if token not in line:
                continue
            findings.append(
                Finding(
                    rel_path,
                    line_no,
                    "legacy_route_trace_reason_token",
                    line.strip(),
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
