#!/usr/bin/env python3
"""Registry bridge and output-contract guards for runtime semantic rewrite checks."""

from __future__ import annotations

import dataclasses
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
AGENT_ENGINE_FILE = SRC_ROOT / "agent_engine.rs"
PREFERRED_RUN_CMD_FILE = SRC_ROOT / "agent_engine/scalar_count_deterministic_plan.rs"
PREFERRED_STRUCTURED_ACTION_FILE = SRC_ROOT / "agent_engine/preferred_structured_action.rs"
MIGRATION_CLASS_FILE = SRC_ROOT / "agent_engine/migration_class.rs"
ASK_PREPARE_FILE = SRC_ROOT / "worker/ask_prepare.rs"
ASK_PIPELINE_FILE = SRC_ROOT / "worker/ask_pipeline.rs"
TASK_JOURNAL_EVIDENCE_COVERAGE_FILE = SRC_ROOT / "task_journal_evidence_coverage.rs"
TASK_JOURNAL_FILE = SRC_ROOT / "task_journal.rs"
INTENT_ROUTER_OBSERVATION_REPAIR_FILE = SRC_ROOT / "intent_router_observation_repair.rs"
INTENT_ROUTER_CONTRACT_HINT_FILE = SRC_ROOT / "intent_router_contract_hint.rs"
INTENT_ROUTER_EXECUTION_CONTRACT_FILE = SRC_ROOT / "intent_router_execution_contract.rs"
INTENT_ROUTER_RUNTIME_STATUS_RECIPE_FILE = (
    SRC_ROOT / "intent_router_runtime_status_recipe.rs"
)
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
LOOP_CONTROL_FILE = SRC_ROOT / "agent_engine/loop_control.rs"
LOOP_CONTROL_MACHINE_STATUS_GAP_FILE = (
    SRC_ROOT / "agent_engine/loop_control_machine_status_gap.rs"
)
LOOP_CONTROL_FILESYSTEM_MUTATION_RECOVERY_FILE = (
    SRC_ROOT / "agent_engine/loop_control_filesystem_mutation_recovery.rs"
)
DRY_RUN_CONTRACT_PLAN_FILE = SRC_ROOT / "agent_engine/dry_run_contract_plan.rs"
OBSERVED_OUTPUT_FILE = SRC_ROOT / "agent_engine/observed_output.rs"
PLANNING_PROMPT_FILE = SRC_ROOT / "agent_engine/planning_prompt.rs"
FINALIZER_OBSERVED_OUTPUT_SCAN_ROOTS: tuple[Path, ...] = (
    SRC_ROOT / "finalize",
    SRC_ROOT / "agent_engine/observed_output.rs",
    SRC_ROOT / "agent_engine/observed_output_direct_answer.rs",
    SRC_ROOT / "agent_engine/observed_output_direct_scalar.rs",
    SRC_ROOT / "agent_engine/value_string_list.rs",
    SRC_ROOT / "agent_engine/direct_observed_finalize_support.rs",
    SRC_ROOT / "agent_engine/loop_control_answer_recovery.rs",
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
        '"final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str)',
        '"final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str())',
    ]
    for token in required_tokens:
        if token in body:
            continue
        findings.append(
            Finding(
                rel_path,
                1,
                "current_workspace_scope_answer_shape_missing",
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
    if "final_answer_shape={}" not in body or "final_answer_shape_class={}" not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "lightweight_tool_spec_answer_shape_missing",
                "lightweight tool spec should expose final_answer_shape/final_answer_shape_class",
            )
        )
    if "contract_marker={}" in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "lightweight_tool_spec_contract_marker_returned",
                "lightweight tool spec must not expose legacy contract_marker",
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
    if '"final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str)' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "answer_verifier_final_answer_shape_missing",
                "answer verifier output contract prompt should expose final_answer_shape",
            )
        )
    if '"contract_marker": route_result.effective_output_contract_semantic_kind().as_str()' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "answer_verifier_contract_marker_returned",
                "answer verifier output contract prompt must not expose legacy contract_marker",
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
    if "error_code=evidence_policy_entry_missing final_answer_shape=missing" not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "verifier_final_answer_shape_detail_missing",
                "evidence-policy missing verifier detail should emit final_answer_shape machine fields",
            )
        )
    if "error_code=evidence_policy_entry_missing contract_marker=" in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "verifier_contract_marker_detail_returned",
                "evidence-policy missing verifier detail must not emit legacy contract_marker",
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
    if "final_answer_shape={}" not in body or "final_answer_shape_class={}" not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "route_guard_record_final_answer_shape_missing",
                "route guard record should log final_answer_shape/final_answer_shape_class",
            )
        )
    if "contract_marker={}" in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "route_guard_record_contract_marker_field",
                "route guard record must not expose legacy contract_marker field name",
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
    if '"agent_loop.final_answer_shape"' not in text or '"agent_loop.final_answer_shape_class"' not in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "loop_control_final_answer_shape_key_missing",
                "loop output vars should expose final_answer_shape/final_answer_shape_class",
            )
        )
    if '"agent_loop.effective_output_contract_marker"' in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "loop_control_contract_marker_key_returned",
                "loop output vars must not expose legacy effective_output_contract_marker key",
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
    machine_detection_files = (
        LOOP_CONTROL_FILE,
        LOOP_CONTROL_MACHINE_STATUS_GAP_FILE,
    )
    machine_detection_texts = [
        (path, path.read_text(encoding="utf-8"))
        for path in machine_detection_files
        if path.exists()
    ]
    if not any(
        'object.contains_key("contract_marker")' in text
        for _, text in machine_detection_texts
    ):
        findings.append(
            Finding(
                rel(LOOP_CONTROL_MACHINE_STATUS_GAP_FILE),
                1,
                "loop_control_contract_marker_reader_missing",
                "loop control machine JSON detection should read contract_marker",
            )
        )
    for path, text in machine_detection_texts:
        if 'object.contains_key("semantic_kind")' in text:
            findings.append(
                Finding(
                    rel(path),
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
    if '"final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str)' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "observed_contract_final_answer_shape_missing",
                "observed fallback contract JSON should expose final_answer_shape",
            )
        )
    if '"contract_marker": route.effective_output_contract_semantic_kind().as_str()' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "observed_contract_marker_returned",
                "observed fallback contract JSON must not expose legacy contract_marker",
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
    if '"final_answer_shape": contract.get("final_answer_shape").and_then(Value::as_str)' not in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "task_journal_step_final_answer_shape_missing",
                "task journal step contract trace should expose final_answer_shape",
            )
        )
    if '"contract_marker": contract.get("contract_marker").and_then(Value::as_str)' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "task_journal_step_contract_marker_returned",
                "task journal step contract trace must not expose legacy contract_marker",
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
    exposes_json_shape = (
        '"final_answer_shape": crate::evidence_policy::FinalAnswerShape::ValidationVerdict.as_str()'
        in body
    )
    exposes_kv_shape = (
        '"final_answer_shape"' in body
        and "crate::evidence_policy::FinalAnswerShape::ValidationVerdict.as_str()" in body
    )
    if not (exposes_json_shape or exposes_kv_shape):
        findings.append(
            Finding(
                rel_path,
                1,
                "schedule_preview_final_answer_shape_missing",
                "schedule preview response should expose final_answer_shape",
            )
        )
    if '"contract_marker": "schedule_intent_preview"' in body:
        findings.append(
            Finding(
                rel_path,
                1,
                "schedule_preview_contract_marker_returned",
                "schedule preview response must not expose legacy contract_marker",
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
    return scan_prompt_utils_contract_repair_judge_marker_only_text(
        rel_path, PROMPT_UTILS_CONTRACT_REPAIR_JUDGE_FILE.read_text(encoding="utf-8")
    )


def scan_prompt_utils_contract_repair_judge_marker_only_text(
    rel_path: str, text: str
) -> list[Finding]:
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
    if 'decision == "planner_execute"' in text:
        findings.append(
            Finding(
                rel_path,
                1,
                "contract_repair_judge_planner_execute_decision_gate",
                "contract repair judge apply inference must use machine fields, not legacy decision=planner_execute",
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
