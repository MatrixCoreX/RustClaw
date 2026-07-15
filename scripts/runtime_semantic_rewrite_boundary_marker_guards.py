#!/usr/bin/env python3
"""Boundary marker typing guards for runtime semantic rewrite checks."""

from __future__ import annotations

import dataclasses
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
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


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def rel(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def quoted_token_alternation(tokens: tuple[str, ...]) -> str:
    return "|".join(re.escape(token) for token in tokens)


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
