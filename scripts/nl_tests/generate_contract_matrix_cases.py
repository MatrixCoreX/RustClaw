#!/usr/bin/env python3
"""Generate deterministic contract-matrix regression cases as JSONL.

This is an offline seed generator. It does not call clawd or a model; live NL
replay can consume the emitted contract ids, expected actions, evidence fields,
and final answer shapes. With --nl --expectations it also writes evaluator
expectation rows aligned with the emitted live replay case order.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path
from typing import Any


DEFAULT_MATRIX = Path("configs/task_contract_matrix.toml")


FIXTURE_ROOT = "scripts/nl_tests/fixtures/device_local"
FIXTURE_DOC = f"{FIXTURE_ROOT}/docs/release_checklist.md"
FIXTURE_DOCS_DIR = f"{FIXTURE_ROOT}/docs"
FIXTURE_PACKAGE = f"{FIXTURE_ROOT}/package.json"
FIXTURE_CONFIG = f"{FIXTURE_ROOT}/configs/app_config.toml"
FIXTURE_DB = f"{FIXTURE_ROOT}/data/test_contract.sqlite"
FIXTURE_ARCHIVE = f"{FIXTURE_ROOT}/tmp/test_bundle.zip"


PROBE_ACTIONS = [
    "run_cmd",
    "fs_basic.list_dir",
    "fs_basic.read_text_range",
    "fs_basic.write_text",
    "fs_basic.find_entries",
    "archive_basic.pack",
    "archive_basic.read",
    "config_basic.validate",
    "docker_basic",
    "package_manager.detect",
    "db_basic",
    "health_check",
    "respond",
]


NL_PROMPTS_BY_CONTRACT: dict[str, str] = {
    "none": "不用执行任何操作，直接用一句话解释 RustClaw 是一个什么样的本地助手。",
    "raw_command_output": "执行 pwd，并简短告诉我命令输出是什么。",
    "service_status": "检查 clawd 服务当前状态，并用一句话说明来源。",
    "hidden_entries_check": "检查当前目录有没有隐藏文件，如果有就列出几个例子。",
    "file_names": f"列出 {FIXTURE_DOCS_DIR} 目录下的文件名，只输出文件名列表。",
    "directory_names": f"列出 {FIXTURE_ROOT} 下的文件夹名，只输出名称列表。",
    "directory_entry_groups": f"列出 {FIXTURE_ROOT} 下的直接子项，并按文件和文件夹分组。",
    "file_paths": f"找出 {FIXTURE_ROOT} 下的 markdown 文件路径，只输出路径列表。",
    "directory_purpose_summary": f"看一下 {FIXTURE_DOCS_DIR} 目录，然后一句话说明这些文件大概是做什么的。",
    "content_excerpt_summary": f"读取 {FIXTURE_DOC} 前 20 行，并用三句话总结。",
    "content_presence_check": f"检查 {FIXTURE_DOC} 里是否提到了 release，只回答是否有并给出依据。",
    "excerpt_kind_judgment": f"读取 {FIXTURE_DOC} 开头内容，判断它更像清单、日志还是配置。",
    "recent_artifacts_judgment": f"列出 {FIXTURE_DOCS_DIR} 最近修改的 2 个文件，再判断它们更像文档还是产物。",
    "workspace_project_summary": f"快速看一下 {FIXTURE_ROOT}，用非技术用户能听懂的话总结它是什么项目。",
    "scalar_count": f"数一下 {FIXTURE_DOCS_DIR} 目录直接子项有多少个，只输出数字。",
    "quantity_comparison": f"比较 {FIXTURE_DOC} 和 {FIXTURE_PACKAGE} 哪个文件更大，并给出依据。",
    "execution_failed_step": "执行一个会失败的只读检查命令：cat /definitely_missing_rustclaw_contract_case，然后说明失败原因。",
    "generated_file_delivery": "写一个简单文本文件到 tmp/contract_matrix_generated_note.txt，内容是 RustClaw contract matrix test，然后把文件路径发给我。",
    "scalar_path_only": f"只输出 {FIXTURE_PACKAGE} 的路径，不要解释。",
    "existence_with_path": f"检查 {FIXTURE_PACKAGE} 是否存在，只回答存在性和路径。",
    "existence_with_path_summary": f"检查 {FIXTURE_PACKAGE} 是否存在，如果存在就一句话说明它大概是什么文件。",
    "recent_scalar_equality_check": "检查当前 git 分支名是否等于 main，只回答判断结果和依据。",
    "git_commit_subject": "只告诉我最近一次 git 提交标题，不要解释。",
    "git_repository_state": "检查这个仓库当前是否有未提交改动，用一句话说明。",
    "structured_keys": f"读取 {FIXTURE_CONFIG} 的顶层键名，只输出键名列表。",
    "config_validation": f"验证 {FIXTURE_CONFIG} 是否是可读配置，并简短说明结果。",
    "config_risk_assessment": "检查 configs/config.toml 的风险配置项，简短说明是否有明显高风险设置。",
    "sqlite_table_listing": f"列出 {FIXTURE_DB} 里的表，并简短说明。",
    "sqlite_table_names_only": f"只输出 {FIXTURE_DB} 里的表名列表。",
    "sqlite_database_kind_judgment": f"判断 {FIXTURE_DB} 更像业务库还是测试库，并给出依据。",
    "sqlite_schema_version": f"读取 {FIXTURE_DB} 的 schema 版本，简短告诉我结果。",
    "package_manager_detection": "检测这台机器可用的包管理器，并说明依据。",
    "archive_list": f"列出 {FIXTURE_ARCHIVE} 里的成员列表。",
    "archive_read": f"读取 {FIXTURE_ARCHIVE} 里的 notes.txt 内容片段，并简短总结。",
    "archive_pack": f"把 {FIXTURE_DOCS_DIR} 打包成 tmp/contract_matrix_docs_bundle.zip，并告诉我生成路径。",
    "archive_unpack": f"把 {FIXTURE_ARCHIVE} 解压到 tmp/contract_matrix_unpacked，并简短说明结果。",
    "docker_ps": "列出当前 Docker 容器，如果没有也要说明观察结果。",
    "docker_images": "列出当前 Docker 镜像，如果没有也要说明观察结果。",
    "docker_logs": "查看最近一个 Docker 容器日志片段，如果没有容器就说明无法获取日志的原因。",
    "docker_container_lifecycle": "检查 Docker 是否可用，并说明是否能执行容器生命周期操作。",
}

NL_PROMPTS_BY_GENERIC_PROFILE: dict[str, str] = {
    "generic_path_content": f"看一下 {FIXTURE_DOC}，然后用一句适合新手的话说明它主要讲什么。",
    "generic_delivery": "生成一个 tmp/contract_matrix_generic_delivery.txt 文件，内容是 generic delivery case，然后把文件发给我。",
}


def normalize_token(value: str) -> str:
    return value.strip().lower()


def parse_action(raw: str) -> tuple[str, str | None]:
    raw = normalize_token(raw).replace("-", "_")
    if "." not in raw:
        return raw, None
    skill, action = raw.split(".", 1)
    return skill, action or None


def action_matches(action: str, policies: list[str]) -> bool:
    action_skill, action_name = parse_action(action)
    for policy in policies:
        policy_skill, policy_name = parse_action(policy)
        if action_skill != policy_skill:
            continue
        if policy_name is None or action_name == policy_name:
            return True
    return False


def action_policy(action: str, contract: dict[str, Any]) -> str:
    if action_matches(action, contract.get("forbidden_actions", [])):
        return "rejected_forbidden"
    allowed = contract.get("allowed_actions", [])
    if not allowed:
        return "allowed" if contract.get("none_passthrough") else "rejected_no_actions_allowed"
    if action_matches(action, allowed):
        return "allowed"
    return "rejected_not_allowed"


def normalized_evidence(contract: dict[str, Any]) -> list[str]:
    return sorted({normalize_token(item) for item in contract.get("required_evidence", []) if item})


def normalized_list(values: list[Any]) -> list[str]:
    return sorted({normalize_token(item) for item in values if isinstance(item, str) and item})


def evidence_expression(contract: dict[str, Any]) -> dict[str, list[str]]:
    raw = contract.get("evidence_expression") or {}
    expression = {
        "all_of": normalized_list(raw.get("all_of", [])),
        "one_of": normalized_list(raw.get("one_of", [])),
        "any_of": normalized_list(raw.get("any_of", [])),
        "negative_evidence": normalized_list(raw.get("negative_evidence", [])),
    }
    if not any(expression.values()):
        expression["all_of"] = normalized_evidence(contract)
    return expression


def evidence_expression_key(contract: dict[str, Any]) -> str:
    expression = evidence_expression(contract)
    return (
        f"all_of={','.join(expression['all_of'])}|"
        f"one_of={','.join(expression['one_of'])}|"
        f"any_of={','.join(expression['any_of'])}|"
        f"negative={','.join(expression['negative_evidence'])}"
    )


def trace_policy_key(matrix: dict[str, Any]) -> str:
    policy = matrix.get("trace_policy", {})
    return (
        f"storage={normalize_token(str(policy.get('evidence_storage', 'redacted_excerpt_hash')))}|"
        f"provider={normalize_token(str(policy.get('provider_evidence_view', 'provider_safe_redacted')))}|"
        f"raw={normalize_token(str(policy.get('raw_excerpt_policy', 'no_full_raw_excerpt')))}|"
        f"max_items={int(policy.get('max_items', 24))}|"
        f"max_excerpt_chars={int(policy.get('max_excerpt_chars', 240))}"
    )


def matrix_hash(matrix: dict[str, Any]) -> str:
    contracts = matrix.get("contracts", {})
    profiles = matrix.get("generic_profiles", [])
    parts = [
        str(matrix.get("schema_version", 1)),
        str(matrix.get("matrix_version", "")),
        str(len(contracts)),
        str(len(profiles)),
        trace_policy_key(matrix),
    ]
    for key in sorted(contracts):
        contract = contracts[key]
        parts.append(
            ":".join(
                [
                    key,
                    ",".join(normalized_evidence(contract)),
                    str(contract.get("final_answer_shape", "")),
                    ",".join(normalized_list(contract.get("allowed_actions", []))),
                    ",".join(normalized_list(contract.get("preferred_actions", []))),
                    ",".join(normalized_list(contract.get("forbidden_actions", []))),
                    evidence_expression_key(contract),
                ]
            )
        )
    for profile in profiles:
        parts.append(
            ":".join(
                [
                    "generic",
                    str(profile.get("name", "")),
                    ",".join(normalized_evidence(profile)),
                    str(profile.get("final_answer_shape", "")),
                    ",".join(normalized_list(profile.get("allowed_actions", []))),
                    ",".join(normalized_list(profile.get("preferred_actions", []))),
                    ",".join(normalized_list(profile.get("forbidden_actions", []))),
                    evidence_expression_key(profile),
                ]
            )
        )
    text = "|".join(parts)
    h = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        h ^= byte
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def base_case(
    matrix: dict[str, Any],
    contract_type: str,
    contract_id: str,
    contract: dict[str, Any],
    phase: str,
    action_ref: str | None,
    expected_decision: str | None,
) -> dict[str, Any]:
    return {
        "case_id": ".".join(
            item
            for item in [
                contract_type,
                contract_id,
                phase,
                normalize_token(action_ref).replace(".", "_") if action_ref else None,
            ]
            if item
        ),
        "source": "task_contract_matrix",
        "matrix_version": matrix.get("matrix_version"),
        "matrix_hash": matrix_hash(matrix),
        "contract_type": contract_type,
        "contract_id": contract_id,
        "semantic_kind": contract.get("semantic_kind"),
        "phase": phase,
        "action_ref": action_ref,
        "expected_policy_decision": expected_decision,
        "required_evidence": normalized_evidence(contract),
        "evidence_expression": evidence_expression(contract),
        "final_answer_shape": contract.get("final_answer_shape", ""),
        "allowed_actions": normalized_list(contract.get("allowed_actions", [])),
        "forbidden_actions": normalized_list(contract.get("forbidden_actions", [])),
        "none_passthrough": bool(contract.get("none_passthrough", False)),
        "failure_policy": contract.get("failure_policy", ""),
    }


def contract_test_hint_lines(case: dict[str, Any]) -> list[str]:
    lines = [
        f"contract_type={case.get('contract_type') or ''}",
        f"contract_id={case.get('contract_id') or ''}",
        f"semantic_kind={case.get('semantic_kind') or ''}",
        f"phase={case.get('phase') or ''}",
        f"final_answer_shape={case.get('final_answer_shape') or ''}",
        "required_evidence_json="
        + json.dumps(case.get("required_evidence") or [], ensure_ascii=False, sort_keys=True),
        "evidence_expression_json="
        + json.dumps(case.get("evidence_expression") or {}, ensure_ascii=False, sort_keys=True),
        "allowed_actions_json="
        + json.dumps(case.get("allowed_actions") or [], ensure_ascii=False, sort_keys=True),
        "forbidden_actions_json="
        + json.dumps(case.get("forbidden_actions") or [], ensure_ascii=False, sort_keys=True),
        f"none_passthrough={str(bool(case.get('none_passthrough'))).lower()}",
    ]
    action_ref = case.get("action_ref")
    decision = case.get("expected_policy_decision")
    if isinstance(action_ref, str) and action_ref:
        if case.get("phase") == "allowed_action" and live_nl_action_preference_applicable(case):
            lines.append(f"preferred_action_ref={action_ref}")
            lines.append("policy_expectation=use_allowed_action_with_required_evidence")
        elif case.get("phase") == "negative_action" and decision != "allowed":
            lines.append(f"candidate_wrong_action_ref={action_ref}")
            lines.append("policy_expectation=runtime_must_reject_or_replace_disallowed_action")
        else:
            lines.append(f"action_ref={action_ref}")
    if decision:
        lines.append(f"expected_policy_decision={decision}")
    contract_id = str(case.get("contract_id") or "")
    if contract_id == "file_names":
        lines.append("selector_target_kind=file")
    elif contract_id == "directory_entry_groups":
        lines.append("selector_target_kind=any")
    elif contract_id == "content_presence_check":
        lines.extend(
            [
                "selector_query=release",
                "selector_case_insensitive=true",
            ]
        )
    elif contract_id == "directory_names":
        lines.append("selector_target_kind=dir")
    elif contract_id == "file_paths":
        lines.extend(
            [
                "selector_extension=md",
                "selector_target_kind=file",
            ]
        )
    elif contract_id == "recent_artifacts_judgment":
        lines.extend(
            [
                "selector_limit=2",
                "selector_sort_by=mtime_desc",
                "selector_target_kind=file",
            ]
        )
    elif contract_id == "quantity_comparison":
        lines.append("selector_answer_style=larger_with_sizes")
    elif contract_id == "sqlite_database_kind_judgment":
        lines.append("selector_database_kind=test")
    return lines


def append_contract_test_hint(prompt: str, case: dict[str, Any]) -> str:
    return "\n".join(
        [
            prompt,
            "[CONTRACT_TEST_HINT]",
            *contract_test_hint_lines(case),
            "[/CONTRACT_TEST_HINT]",
        ]
    )


def generated_prompt_for_case(case: dict[str, Any]) -> str:
    contract_id = str(case.get("contract_id") or "")
    if case.get("contract_type") == "generic":
        prompt = NL_PROMPTS_BY_GENERIC_PROFILE.get(contract_id)
    else:
        prompt = NL_PROMPTS_BY_CONTRACT.get(contract_id)
    if not prompt:
        prompt = (
            f"按 RustClaw 结构化任务 {contract_id} 做一次只读检查，"
            "需要先观察证据，再按要求给出简短结果。"
        )
    return append_contract_test_hint(prompt, case)


def as_nl_case(case: dict[str, Any]) -> dict[str, Any]:
    contract_id = str(case.get("contract_id") or "unknown")
    phase = str(case.get("phase") or "case")
    name = f"contract_matrix_{contract_id}_{phase}"
    if case.get("action_ref"):
        name = f"{name}_{str(case['action_ref']).replace('.', '_')}"
    tags = [
        "contract_matrix",
        "generated",
        "live_nl",
        str(case.get("contract_type") or "contract"),
        contract_id,
        phase,
    ]
    if case.get("expected_policy_decision"):
        tags.append(str(case["expected_policy_decision"]))
    row = {
        "suite": "contract_matrix",
        "name": name,
        "tags": tags,
        "prompt": generated_prompt_for_case(case),
        "expect": "",
    }
    row.update(case)
    return row


def action_skill(action_ref: str) -> str:
    return parse_action(action_ref)[0]


def live_nl_action_preference_applicable(case: dict[str, Any]) -> bool:
    """Return whether a live NL prompt can safely force this allowed action.

    Contract actions may be conditionally valid for a subtype of the target
    object. For example, `archive_basic.read` is a valid way to provide a
    content excerpt when the target is an archive member, but the default live
    prompt for `content_excerpt_summary` targets a plain markdown file. For
    live replay we keep the contract/evidence coverage but avoid forcing an
    action whose argument contract cannot be satisfied by that prompt.
    """

    action_ref = case.get("action_ref")
    if not isinstance(action_ref, str):
        return True
    contract_id = str(case.get("contract_id") or "")
    action = normalize_token(action_ref).replace("-", "_")
    archive_action_contracts = {
        "archive_basic.list": {"archive_list"},
        "archive_basic.read": {"archive_read"},
        "archive_basic.pack": {"archive_pack"},
        "archive_basic.unpack": {"archive_unpack"},
    }
    allowed_contracts = archive_action_contracts.get(action)
    if allowed_contracts is not None:
        return contract_id in allowed_contracts
    prompt_surface_action_contracts = {
        "file_paths": {"fs_basic.find_entries"},
        "recent_scalar_equality_check": {"git_basic", "run_cmd"},
        "scalar_count": {"fs_basic.count_entries", "run_cmd"},
    }
    allowed_actions = prompt_surface_action_contracts.get(contract_id)
    if allowed_actions is not None:
        return action in allowed_actions
    return True


def allowed_action_refs(case: dict[str, Any]) -> list[str]:
    return sorted(
        {
            normalize_token(action)
            for action in case.get("allowed_actions", [])
            if isinstance(action, str) and normalize_token(action)
        }
    )


def allowed_execution_skills(case: dict[str, Any]) -> list[str]:
    ignored = {"respond", "synthesize_answer", "think"}
    return sorted(
        {
            action_skill(action)
            for action in allowed_action_refs(case)
            if action_skill(action) not in ignored
        }
    )


def planned_action_equivalents(case: dict[str, Any]) -> list[str]:
    action_ref = str(case.get("action_ref") or "")
    if not action_ref:
        return []
    contract_id = str(case.get("contract_id") or "")
    action = normalize_token(action_ref).replace("-", "_")
    equivalents: dict[tuple[str, str], list[str]] = {
        ("config_risk_assessment", "config_guard"): [
            "config_guard",
            "config_basic.guard_rustclaw_config",
            "config_edit.guard_config",
        ],
        ("config_validation", "config_guard"): [
            "config_guard",
            "config_basic.guard_rustclaw_config",
            "config_edit.guard_config",
            "config_basic.validate",
            "config_edit.validate_config",
        ],
        ("execution_failed_step", "log_analyze"): ["log_analyze", "run_cmd"],
        (
            "existence_with_path_summary",
            "fs_basic.find_entries",
        ): ["fs_basic.find_entries", "fs_basic.stat_paths", "fs_basic.read_text_range"],
        ("generated_file_delivery", "transform"): ["transform", "fs_basic.write_text"],
        ("git_repository_state", "run_cmd"): ["run_cmd", "git_basic"],
        ("git_commit_subject", "run_cmd"): ["run_cmd", "git_basic"],
        ("recent_scalar_equality_check", "run_cmd"): ["run_cmd", "git_basic"],
    }
    return equivalents.get((contract_id, action), [action])


def expectation_for_case(case: dict[str, Any], case_index: int) -> dict[str, Any]:
    row: dict[str, Any] = {
        "case": case_index,
    }
    contract_id = str(case.get("contract_id") or "")
    if case.get("contract_type") == "generic":
        if contract_id == "generic_delivery":
            row["contract_match_any"] = ["generic_delivery", "generated_file_delivery"]
            row["contract_final_answer_shape_any"] = [
                "delivery_token_or_path",
                case.get("final_answer_shape", ""),
            ]
        elif contract_id == "generic_path_content":
            row["contract_match_any"] = ["generic_path_content", "content_excerpt_summary"]
            row["contract_final_answer_shape_any"] = [
                "summary_with_evidence",
                "summary_grounded_in_excerpt",
            ]
        else:
            row["contract_match"] = case["contract_id"]
            row["contract_final_answer_shape"] = case.get("final_answer_shape", "")
    else:
        row["contract_match"] = case["contract_id"]
        row["contract_final_answer_shape"] = case.get("final_answer_shape", "")
    semantic_kind = case.get("semantic_kind")
    if case.get("contract_type") == "semantic" and semantic_kind:
        row["contract_semantic_kind"] = semantic_kind

    required_evidence = case.get("required_evidence") or []
    if case.get("contract_type") == "generic" and contract_id == "generic_path_content":
        required_evidence = ["content_excerpt"]
    if required_evidence:
        row["required_evidence_all"] = required_evidence
        row["missing_evidence_empty"] = True

    actions = allowed_action_refs(case)
    if actions and not case.get("none_passthrough"):
        skills = allowed_execution_skills(case)
        if skills:
            row["executed_any"] = skills
    if (
        case.get("phase") == "allowed_action"
        and case.get("action_ref")
        and live_nl_action_preference_applicable(case)
    ):
        row["planned_action_any"] = planned_action_equivalents(case)

    if case.get("phase") == "negative_action":
        action_ref = str(case.get("action_ref") or "")
        forbidden_skills = {action_skill(action) for action in case.get("forbidden_actions", [])}
        allowed_skills = set(allowed_execution_skills(case))
        if action_ref:
            skill = action_skill(action_ref)
            if skill in forbidden_skills and skill not in allowed_skills:
                row["executed_none_of"] = [skill]
    if contract_id == "file_names":
        row["final_contains"] = ["release_checklist.md", "service_notes.md"]
        row["final_not_contains"] = ["archive"]
    elif contract_id == "directory_entry_groups":
        row["final_contains"] = ["configs", "data", "docs", "logs", "tmp", "README.md", "package.json"]
    elif contract_id == "directory_names":
        row["final_contains"] = ["configs", "data", "docs", "logs", "tmp"]
        row["final_not_contains"] = ["README.md", "package.json"]
    elif contract_id == "file_paths":
        row["final_contains"] = ["release_checklist.md", "service_notes.md"]
        row["final_not_contains"] = ["package.json"]
    elif contract_id == "recent_artifacts_judgment":
        row["final_contains"] = ["release_checklist.md", "service_notes.md"]
    elif contract_id == "content_presence_check":
        row["final_contains"] = ["release", "# Release Checklist"]
        row["final_not_contains"] = ["不包含", "Does not contain"]
    elif contract_id == "quantity_comparison":
        row["final_contains"] = ["package.json", "246", "release_checklist.md", "153"]
        row["final_not_contains"] = ["package.json：93 字节", "package.json: 93 bytes"]

    return row


def write_expectations(path: Path, cases: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "".join(
            json.dumps(expectation_for_case(case, idx), ensure_ascii=False, sort_keys=True) + "\n"
            for idx, case in enumerate(cases, 1)
        ),
        encoding="utf-8",
    )


def generate_all_cases(matrix: dict[str, Any]) -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    contracts = matrix.get("contracts", {})
    for contract_id in sorted(contracts):
        contract = contracts[contract_id]
        cases.extend(generate_contract_cases(matrix, "semantic", contract_id, contract))
    for profile in matrix.get("generic_profiles", []):
        contract_id = profile.get("name", "unnamed_generic")
        cases.extend(generate_contract_cases(matrix, "generic", contract_id, profile))
    return unique_cases(cases)


def generate_contract_cases(
    matrix: dict[str, Any],
    contract_type: str,
    contract_id: str,
    contract: dict[str, Any],
) -> list[dict[str, Any]]:
    cases = [
        base_case(matrix, contract_type, contract_id, contract, "evidence_shape", None, None)
    ]
    for action in sorted({normalize_token(item) for item in contract.get("allowed_actions", [])}):
        cases.append(
            base_case(
                matrix,
                contract_type,
                contract_id,
                contract,
                "allowed_action",
                action,
                action_policy(action, contract),
            )
        )
    for action in sorted({normalize_token(item) for item in contract.get("forbidden_actions", [])}):
        cases.append(
            base_case(
                matrix,
                contract_type,
                contract_id,
                contract,
                "negative_action",
                action,
                action_policy(action, contract),
            )
        )
    for action in PROBE_ACTIONS:
        decision = action_policy(action, contract)
        if decision != "allowed":
            cases.append(
                base_case(
                    matrix,
                    contract_type,
                    contract_id,
                    contract,
                    "negative_action",
                    action,
                    decision,
                )
            )
    return cases


def unique_cases(cases: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: set[str] = set()
    out: list[dict[str, Any]] = []
    for case in cases:
        case_id = case["case_id"]
        if case_id in seen:
            continue
        seen.add(case_id)
        out.append(case)
    return out


def choose_first_case(
    cases: list[dict[str, Any]],
    seen_case_ids: set[str],
    predicate: Any,
) -> dict[str, Any] | None:
    unseen = [case for case in cases if case["case_id"] not in seen_case_ids and predicate(case)]
    if unseen:
        return unseen[0]
    for case in cases:
        if predicate(case):
            return case
    return None


def coverage_anchor_cases(cases: list[dict[str, Any]], seen_case_ids: set[str]) -> list[dict[str, Any]]:
    anchors: list[dict[str, Any]] = []
    semantic_ids = sorted(
        {
            case["contract_id"]
            for case in cases
            if case["contract_type"] == "semantic" and case.get("contract_id")
        }
    )
    generic_ids = sorted(
        {
            case["contract_id"]
            for case in cases
            if case["contract_type"] == "generic" and case.get("contract_id")
        }
    )
    phases = sorted({case["phase"] for case in cases if case.get("phase")})
    decisions = sorted(
        {
            case["expected_policy_decision"]
            for case in cases
            if case.get("expected_policy_decision")
        }
    )
    final_shapes = sorted(
        {case["final_answer_shape"] for case in cases if case.get("final_answer_shape")}
    )

    for contract_id in semantic_ids:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, contract_id=contract_id: item["contract_type"] == "semantic"
            and item["contract_id"] == contract_id,
        )
        if case:
            anchors.append(case)
    for contract_id in generic_ids:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, contract_id=contract_id: item["contract_type"] == "generic"
            and item["contract_id"] == contract_id,
        )
        if case:
            anchors.append(case)
    for phase in phases:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, phase=phase: item.get("phase") == phase,
        )
        if case:
            anchors.append(case)
    for decision in decisions:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, decision=decision: item.get("expected_policy_decision") == decision,
        )
        if case:
            anchors.append(case)
    for shape in final_shapes:
        case = choose_first_case(
            cases,
            seen_case_ids,
            lambda item, shape=shape: item.get("final_answer_shape") == shape,
        )
        if case:
            anchors.append(case)
    return unique_cases(anchors)


def select_cases(
    cases: list[dict[str, Any]],
    count: int,
    batch: int,
    seen_case_ids: set[str] | None = None,
) -> list[dict[str, Any]]:
    seen_case_ids = seen_case_ids or set()
    if count <= 0 or count >= len(cases):
        return cases
    mandatory = coverage_anchor_cases(cases, seen_case_ids)
    if len(mandatory) >= count:
        return mandatory[:count]

    mandatory_ids = {case["case_id"] for case in mandatory}
    unseen_extras = [
        case
        for case in cases
        if case["case_id"] not in mandatory_ids and case["case_id"] not in seen_case_ids
    ]
    seen_extras = [
        case
        for case in cases
        if case["case_id"] not in mandatory_ids and case["case_id"] in seen_case_ids
    ]
    extras = unseen_extras or seen_extras
    offset = (batch * max(1, count - len(mandatory))) % len(extras) if extras else 0
    rotated = extras[offset:] + extras[:offset]
    selected = unique_cases(mandatory + rotated)
    if len(selected) < count and unseen_extras:
        selected = unique_cases(selected + seen_extras)
    return selected[:count]


def coverage_report(cases: list[dict[str, Any]]) -> dict[str, Any]:
    semantics = sorted(
        {
            case["semantic_kind"]
            for case in cases
            if case["contract_type"] == "semantic" and case.get("semantic_kind")
        }
    )
    generic_profiles = sorted(
        {case["contract_id"] for case in cases if case["contract_type"] == "generic"}
    )
    decisions = sorted(
        {
            case["expected_policy_decision"]
            for case in cases
            if case.get("expected_policy_decision")
        }
    )
    phases = sorted({case["phase"] for case in cases})
    final_shapes = sorted(
        {case["final_answer_shape"] for case in cases if case.get("final_answer_shape")}
    )
    return {
        "case_count": len(cases),
        "contract_count": len(
            {
                (case["contract_type"], case["contract_id"])
                for case in cases
                if case.get("contract_type") and case.get("contract_id")
            }
        ),
        "semantic_count": len(semantics),
        "generic_profile_count": len(generic_profiles),
        "final_answer_shape_count": len(final_shapes),
        "phase_count": len(phases),
        "policy_decisions": decisions,
        "phases": phases,
    }


def validate_selected_cases(
    cases: list[dict[str, Any]],
    requested_count: int,
    matrix: dict[str, Any],
) -> list[str]:
    errors: list[str] = []
    if requested_count > 0 and len(cases) < requested_count:
        errors.append(f"only generated {len(cases)} cases, requested {requested_count}")
    ids = [case["case_id"] for case in cases]
    if len(ids) != len(set(ids)):
        errors.append("generated duplicate case ids")
    report = coverage_report(cases)
    if report["case_count"] >= 100 and report["semantic_count"] == 0:
        errors.append("generated cases do not cover semantic contracts")
    expected_semantics = set(matrix.get("contracts", {}))
    expected_generics = {
        profile.get("name", "unnamed_generic")
        for profile in matrix.get("generic_profiles", [])
    }
    expected_shapes = {
        contract.get("final_answer_shape", "")
        for contract in matrix.get("contracts", {}).values()
    } | {
        profile.get("final_answer_shape", "")
        for profile in matrix.get("generic_profiles", [])
    }
    selected_semantics = {
        case["contract_id"]
        for case in cases
        if case["contract_type"] == "semantic"
    }
    selected_generics = {
        case["contract_id"]
        for case in cases
        if case["contract_type"] == "generic"
    }
    selected_shapes = {
        case["final_answer_shape"]
        for case in cases
        if case.get("final_answer_shape")
    }
    if report["case_count"] >= 100:
        missing_semantics = sorted(expected_semantics - selected_semantics)
        missing_generics = sorted(expected_generics - selected_generics)
        missing_shapes = sorted(expected_shapes - selected_shapes)
        if missing_semantics:
            errors.append(f"generated cases miss semantic contracts: {missing_semantics}")
        if missing_generics:
            errors.append(f"generated cases miss generic profiles: {missing_generics}")
        if missing_shapes:
            errors.append(f"generated cases miss final answer shapes: {missing_shapes}")
    for decision in ("allowed", "rejected_forbidden", "rejected_not_allowed"):
        if decision not in report["policy_decisions"]:
            errors.append(f"generated cases do not include policy decision {decision}")
    return errors


def read_history_case_ids(path: Path | None) -> set[str]:
    if path is None or not path.exists():
        return set()
    seen: set[str] = set()
    with path.open("r", encoding="utf-8") as fh:
        for raw_line in fh:
            line = raw_line.strip()
            if not line:
                continue
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                seen.add(line)
                continue
            if isinstance(item, dict) and isinstance(item.get("case_id"), str):
                seen.add(item["case_id"])
            elif isinstance(item, str):
                seen.add(item)
    return seen


def append_history_case_ids(path: Path, cases: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as fh:
        for case in cases:
            fh.write(json.dumps({"case_id": case["case_id"]}, sort_keys=True))
            fh.write("\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--batch", type=int, default=0)
    parser.add_argument("--history", type=Path)
    parser.add_argument("--update-history", action="store_true")
    parser.add_argument("--report", action="store_true")
    parser.add_argument("--check", action="store_true")
    parser.add_argument(
        "--nl",
        action="store_true",
        help="emit client-like live NL JSONL rows with prompt/name/tags plus contract metadata",
    )
    parser.add_argument(
        "--expectations",
        type=Path,
        help="write evaluator expectations JSONL for the selected case order; intended for --nl replay",
    )
    args = parser.parse_args()

    if args.update_history and args.history is None:
        parser.error("--update-history requires --history")

    with args.matrix.open("rb") as fh:
        matrix = tomllib.load(fh)
    seen_case_ids = read_history_case_ids(args.history)
    cases = select_cases(generate_all_cases(matrix), args.count, args.batch, seen_case_ids)

    if args.check:
        errors = validate_selected_cases(cases, args.count, matrix)
        if errors:
            for error in errors:
                print(f"ERROR: {error}", file=sys.stderr)
            return 1

    output_cases = [as_nl_case(case) for case in cases] if args.nl else cases
    for case in output_cases:
        print(json.dumps(case, ensure_ascii=False, sort_keys=True))

    if args.expectations is not None:
        write_expectations(args.expectations, cases)

    if args.update_history and args.history is not None:
        append_history_case_ids(args.history, cases)

    if args.report:
        print(json.dumps(coverage_report(cases), ensure_ascii=False, sort_keys=True), file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
