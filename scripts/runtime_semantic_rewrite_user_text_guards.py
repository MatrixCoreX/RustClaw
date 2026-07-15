#!/usr/bin/env python3
"""User-text semantic selection guards for runtime semantic rewrite checks."""

from __future__ import annotations

import dataclasses
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"
VALUE_STRING_LIST_FILE = SRC_ROOT / "agent_engine/value_string_list.rs"
RUNTIME_SURFACE_PLAN_FILE = SRC_ROOT / "agent_engine/runtime_surface_plan.rs"
READ_RANGE_ACTION_FILE = SRC_ROOT / "agent_engine/read_range_action.rs"
SINGLE_TARGET_STRUCTURED_FIELD_REWRITE_FILE = (
    SRC_ROOT / "agent_engine/single_target_structured_field_rewrite.rs"
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    text: str


def rel(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


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
