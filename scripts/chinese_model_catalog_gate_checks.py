#!/usr/bin/env python3
"""Agent parity gate assertions used by the Chinese model catalog guard."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
README = ROOT / "README.md"
README_ZH_CN = ROOT / "README.zh-CN.md"
NL_TESTS_README = ROOT / "scripts/nl_tests/README.md"
CHINESE_PROVIDER_SMOKE_RUNNER = ROOT / "scripts/nl_tests/run_chinese_provider_smoke_matrix.sh"
CHINESE_PROVIDER_SMOKE_MATRIX_CHECKER = ROOT / "scripts/nl_tests/check_chinese_provider_smoke_matrix.py"
AGENT_PARITY_GATE_RUNNER = ROOT / "scripts/nl_tests/run_agent_parity_gate.sh"
SUITE_WRAPPER_CONTRACT_CHECKER = ROOT / "scripts/nl_tests/check_suite_wrapper_contract.py"
SUITE_ARTIFACT_CONTRACT_CHECKER = ROOT / "scripts/nl_tests/check_suite_artifact_contract.py"
ROLLOUT_METRICS_SUMMARY = ROOT / "scripts/nl_tests/summarize_rollout_metrics.py"


RequireFn = Callable[[bool, list[str], str], None]


def fail(findings: list[str], message: str) -> None:
    findings.append(message)


def require(condition: bool, findings: list[str], message: str) -> None:
    if not condition:
        fail(findings, message)


def check_agent_parity_auxiliary_gate_steps(
    *,
    parity_text: str,
    suite_artifact_contract_text: str,
    findings: list[str],
    require: RequireFn,
) -> None:
    require(
        "AGENT_PARITY_GATE_STEP llm_raw_trace_runner_contract" in parity_text,
        findings,
        "agent parity gate must run the NL raw LLM trace runner contract step",
    )
    require(
        "agent_parity_gate/secret_scan_contract_self_test.txt" in suite_artifact_contract_text
        and '"secret_scan_contract_self_test": "1"' in suite_artifact_contract_text
        and "SECRET_SCAN_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "agent_parity_gate/nl_suite_checker_self_tests.txt" in suite_artifact_contract_text
        and '"nl_suite_checker_self_tests": "1"' in suite_artifact_contract_text
        and "SUITE_WRAPPER_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "RUNNER_PATH_REF_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "COMPACT_COVERAGE_SELF_TEST ok" in suite_artifact_contract_text
        and "LLM_RAW_TRACE_RUNNER_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text,
        findings,
        "suite artifact contract checker must include recent checker self-test gate tokens",
    )
    require(
        "AGENT_PARITY_GATE_STEP suite_artifact_contract_self_test" in parity_text
        and "check_suite_artifact_contract.py\" --self-test" in parity_text
        and "suite_artifact_contract_self_test.txt" in parity_text,
        findings,
        "agent parity gate must run the suite artifact contract checker self-test",
    )
    require(
        "suite_artifact_contract_self_test=1" in parity_text,
        findings,
        "agent parity gate summary must record the suite artifact contract self-test state",
    )
    require(
        "print_llm_raw_trace.py\" --self-test" in parity_text
        and "check_llm_raw_trace_runner_contract.py\" --self-test" in parity_text
        and "check_llm_raw_trace_runner_contract.py" in parity_text
        and "llm_raw_trace_runner_contract.txt" in parity_text,
        findings,
        "agent parity gate must write the NL raw LLM trace runner contract artifact",
    )
    require(
        "check_secret_scan_contract.py\" --self-test" in parity_text
        and "secret_scan_contract_self_test.txt" in parity_text
        and "secret_scan_contract_self_test=1" in parity_text
        and "nl_suite_checker_self_tests.txt" in parity_text
        and "nl_suite_checker_self_tests=1" in parity_text,
        findings,
        "agent parity gate must write recent checker self-test artifacts and flags",
    )
    require(
        "llm_raw_trace_runner_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the NL raw LLM trace runner contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP rollout_metrics_contract" in parity_text
        and "summarize_rollout_metrics.py\" --self-test" in parity_text
        and "rollout_metrics_contract.txt" in parity_text
        and "rollout_metrics_contract=1" in parity_text,
        findings,
        "agent parity gate must run and record the rollout metrics path contract self-test",
    )
    require(
        "LIVE_METRICS_RAN=1" in parity_text
        and "live_metrics=${LIVE_METRICS_RAN}" in parity_text,
        findings,
        "agent parity gate summary must record whether live run metrics actually executed",
    )


def check_chinese_provider_smoke_live_scope(findings: list[str]) -> None:
    require(
        CHINESE_PROVIDER_SMOKE_RUNNER.exists(),
        findings,
        f"missing {CHINESE_PROVIDER_SMOKE_RUNNER.relative_to(ROOT)}",
    )
    require(
        AGENT_PARITY_GATE_RUNNER.exists(),
        findings,
        f"missing {AGENT_PARITY_GATE_RUNNER.relative_to(ROOT)}",
    )
    require(
        SUITE_WRAPPER_CONTRACT_CHECKER.exists(),
        findings,
        f"missing {SUITE_WRAPPER_CONTRACT_CHECKER.relative_to(ROOT)}",
    )
    require(
        SUITE_ARTIFACT_CONTRACT_CHECKER.exists(),
        findings,
        f"missing {SUITE_ARTIFACT_CONTRACT_CHECKER.relative_to(ROOT)}",
    )
    require(
        README.exists(),
        findings,
        f"missing {README.relative_to(ROOT)}",
    )
    require(
        README_ZH_CN.exists(),
        findings,
        f"missing {README_ZH_CN.relative_to(ROOT)}",
    )
    require(
        NL_TESTS_README.exists(),
        findings,
        f"missing {NL_TESTS_README.relative_to(ROOT)}",
    )
    if not CHINESE_PROVIDER_SMOKE_RUNNER.exists() or not AGENT_PARITY_GATE_RUNNER.exists():
        return

    smoke_text = CHINESE_PROVIDER_SMOKE_RUNNER.read_text(encoding="utf-8")
    smoke_matrix_text = (
        CHINESE_PROVIDER_SMOKE_MATRIX_CHECKER.read_text(encoding="utf-8")
        if CHINESE_PROVIDER_SMOKE_MATRIX_CHECKER.exists()
        else ""
    )
    parity_text = AGENT_PARITY_GATE_RUNNER.read_text(encoding="utf-8")
    readme_text = README.read_text(encoding="utf-8") if README.exists() else ""
    readme_zh_text = README_ZH_CN.read_text(encoding="utf-8") if README_ZH_CN.exists() else ""
    nl_tests_readme_text = (
        NL_TESTS_README.read_text(encoding="utf-8") if NL_TESTS_README.exists() else ""
    )
    suite_wrapper_text = (
        SUITE_WRAPPER_CONTRACT_CHECKER.read_text(encoding="utf-8")
        if SUITE_WRAPPER_CONTRACT_CHECKER.exists()
        else ""
    )
    suite_artifact_contract_text = (
        SUITE_ARTIFACT_CONTRACT_CHECKER.read_text(encoding="utf-8")
        if SUITE_ARTIFACT_CONTRACT_CHECKER.exists()
        else ""
    )
    rollout_metrics_text = (
        ROLLOUT_METRICS_SUMMARY.read_text(encoding="utf-8")
        if ROLLOUT_METRICS_SUMMARY.exists()
        else ""
    )
    require(
        'DEFAULT_LIVE_PROVIDERS="${CHINESE_PROVIDER_LIVE_PROVIDERS:-minimax}"'
        in smoke_text,
        findings,
        "Chinese provider smoke runner must default live scope to minimax",
    )
    require(
        'if [[ "$LIVE_SCOPE_SET" -eq 0 ]]' in smoke_text
        and 'add_csv_live_providers "$DEFAULT_LIVE_PROVIDERS"' in smoke_text,
        findings,
        "Chinese provider smoke runner must apply the default live scope when no override is passed",
    )
    require(
        'if [[ "$item" == "all" ]]' in smoke_text and "LIVE_SCOPE_ALL=1" in smoke_text,
        findings,
        "Chinese provider smoke runner must keep explicit all-provider opt-in",
    )
    require(
        '"provider_not_in_live_scope"' in smoke_text,
        findings,
        "Chinese provider smoke runner must preserve provider_not_in_live_scope attribution",
    )
    require(
        "live_scope_providers=$(live_scope_csv)" in smoke_text,
        findings,
        "Chinese provider smoke runner must report the effective live scope as a machine token",
    )
    require(
        "path_ref()" in smoke_text
        and '"case_file": path_ref(case_file)' in smoke_text
        and '"output_file": path_ref(output_file)' in smoke_text
        and '"run_dir": path_ref(run_dir)' in smoke_text
        and 'CHINESE_PROVIDER_SMOKE_MATRIX out_dir_ref=$(path_ref "$OUT_DIR")' in smoke_text,
        findings,
        "Chinese provider smoke runner must write portable path refs instead of host paths",
    )
    require(
        "path_ref(case_file)" in smoke_matrix_text
        and "CHINESE_PROVIDER_SMOKE_MATRIX_SELF_TEST ok" in smoke_matrix_text
        and "external_path" in smoke_matrix_text,
        findings,
        "Chinese provider smoke case coverage must write portable case_file refs and self-test them",
    )
    require(
        "portable_path_ref" in rollout_metrics_text
        and "rollout_metrics_ok_line" in rollout_metrics_text
        and "ROLLOUT_METRICS_SELF_TEST ok" in rollout_metrics_text,
        findings,
        "rollout metrics summary must write portable source/output refs and self-test them",
    )
    require(
        'CHINESE_PROVIDER_LIVE_PROVIDERS="${CHINESE_PROVIDER_LIVE_PROVIDERS:-minimax}"'
        in parity_text,
        findings,
        "agent parity gate must default Chinese-provider live scope to minimax",
    )
    require(
        '--live-providers "$CHINESE_PROVIDER_LIVE_PROVIDERS"' in parity_text,
        findings,
        "agent parity gate must pass the configured Chinese-provider live scope to the smoke runner",
    )
    require(
        'CHINESE_PROVIDER_ENV_FILE="${CHINESE_PROVIDER_ENV_FILE:-${ROOT_DIR}/../runtime_env_filled.sh}"'
        in parity_text,
        findings,
        "agent parity gate must default Chinese-provider preflight env file to ../runtime_env_filled.sh",
    )
    require(
        "--chinese-env-file)" in parity_text and "--no-chinese-env-file)" in parity_text,
        findings,
        "agent parity gate must expose explicit Chinese-provider env-file override and disable options",
    )
    require(
        'chinese_provider_env_file_args+=(--env-file "$CHINESE_PROVIDER_ENV_FILE")'
        in parity_text,
        findings,
        "agent parity gate must build reusable Chinese-provider env-file args",
    )
    require(
        '"${chinese_provider_env_file_args[@]}"' in parity_text,
        findings,
        "agent parity gate must pass Chinese-provider env-file args to catalog and smoke checks",
    )
    require(
        "AGENT_PARITY_GATE_STEP chinese_model_catalog_self_test" in parity_text
        and "check_chinese_model_catalog.py" in parity_text
        and "--self-test" in parity_text
        and "chinese_model_catalog_self_test.txt" in parity_text,
        findings,
        "agent parity gate must run the Chinese model catalog self-test artifact",
    )
    require(
        "chinese_provider_live_providers=${CHINESE_PROVIDER_LIVE_PROVIDERS}" in parity_text,
        findings,
        "agent parity gate summary must record the Chinese-provider live scope",
    )
    require(
        "chinese_provider_env_file_state=${CHINESE_PROVIDER_ENV_FILE_STATE}" in parity_text,
        findings,
        "agent parity gate summary must record the Chinese-provider env-file state",
    )
    require(
        "chinese_provider_env_file_source=${CHINESE_PROVIDER_ENV_FILE_SOURCE}" in parity_text,
        findings,
        "agent parity gate summary must record the Chinese-provider env-file source token",
    )
    require(
        "chinese_provider_env_file=${CHINESE_PROVIDER_ENV_FILE}" not in parity_text,
        findings,
        "agent parity gate summary must not record the Chinese-provider env-file path",
    )
    require(
        "AGENT_PARITY_GATE_STEP runtime_hard_reply_baseline" in parity_text,
        findings,
        "agent parity gate must run the runtime hard-reply baseline guard step",
    )
    require(
        "check_no_runtime_hard_reply.py" in parity_text
        and 'check_no_runtime_hard_reply.py" --self-test' in parity_text
        and "runtime_hard_reply_baseline.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the runtime hard-reply baseline artifact",
    )
    require(
        "runtime_hard_reply_baseline=1" in parity_text,
        findings,
        "agent parity gate summary must record the runtime hard-reply baseline guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP policy_boundary_hard_reply" in parity_text,
        findings,
        "agent parity gate must run the policy-boundary hard-reply guard step",
    )
    require(
        "check_no_policy_boundary_hard_reply.py" in parity_text
        and 'check_no_policy_boundary_hard_reply.py" --self-test' in parity_text
        and "policy_boundary_hard_reply.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the policy-boundary hard-reply artifact",
    )
    require(
        "policy_boundary_hard_reply=1" in parity_text,
        findings,
        "agent parity gate summary must record the policy-boundary hard-reply guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP repair_no_user_text_fields" in parity_text,
        findings,
        "agent parity gate must run the repair no-user-text guard step",
    )
    require(
        "check_repair_no_user_text_fields.py" in parity_text
        and 'check_repair_no_user_text_fields.py" --self-test' in parity_text
        and "repair_no_user_text_fields.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the repair no-user-text artifact",
    )
    require(
        "repair_no_user_text_fields=1" in parity_text,
        findings,
        "agent parity gate summary must record the repair no-user-text guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP policy_decision_tokens" in parity_text,
        findings,
        "agent parity gate must run the policy decision token guard step",
    )
    require(
        "check_policy_decision_tokens.py" in parity_text
        and 'check_policy_decision_tokens.py" --self-test' in parity_text
        and "policy_decision_tokens.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the policy decision token artifact",
    )
    require(
        "policy_decision_tokens=1" in parity_text,
        findings,
        "agent parity gate summary must record the policy decision token guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP agent_loop_guard_final_scope" in parity_text,
        findings,
        "agent parity gate must run the final agent-loop guard scope step",
    )
    require(
        "check_agent_loop_guard_final_scope.py" in parity_text
        and 'check_agent_loop_guard_final_scope.py" --self-test' in parity_text
        and "agent_loop_guard_final_scope.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the final guard scope artifact",
    )
    require(
        "agent_loop_guard_final_scope=1" in parity_text,
        findings,
        "agent parity gate summary must record the final guard scope state",
    )
    require(
        "AGENT_PARITY_GATE_STEP registry_policy_contracts" in parity_text,
        findings,
        "agent parity gate must run the registry policy contract guard step",
    )
    require(
        "check_registry_policy_contracts.py" in parity_text
        and 'check_registry_policy_contracts.py" --self-test' in parity_text
        and "registry_policy_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the registry policy contract artifact",
    )
    require(
        "registry_policy_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the registry policy contract guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP skill_registry_aliases" in parity_text,
        findings,
        "agent parity gate must run the skill registry aliases guard step",
    )
    require(
        "check_skill_registry_aliases.py" in parity_text
        and 'check_skill_registry_aliases.py" --self-test' in parity_text
        and "skill_registry_aliases.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the skill registry aliases artifact",
    )
    require(
        "skill_registry_aliases=1" in parity_text,
        findings,
        "agent parity gate summary must record the skill registry aliases guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP long_tail_skill_contracts" in parity_text,
        findings,
        "agent parity gate must run the long-tail skill contract guard step",
    )
    require(
        "check_long_tail_skill_contracts.py" in parity_text
        and 'check_long_tail_skill_contracts.py" --self-test' in parity_text
        and "long_tail_skill_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the long-tail skill contract artifact",
    )
    require(
        "long_tail_skill_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the long-tail skill contract guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP task_lifecycle_contracts" in parity_text,
        findings,
        "agent parity gate must run the task lifecycle contract guard step",
    )
    require(
        "check_task_lifecycle_contracts.py" in parity_text
        and 'check_task_lifecycle_contracts.py" --self-test' in parity_text
        and "task_lifecycle_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the task lifecycle contract artifact",
    )
    require(
        "task_lifecycle_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the task lifecycle contract guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP task_event_context_team_contracts" in parity_text,
        findings,
        "agent parity gate must run the task event/context/team contract guard step",
    )
    require(
        "check_task_event_context_team_contracts.py" in parity_text
        and 'check_task_event_context_team_contracts.py" --self-test' in parity_text
        and "task_event_context_team_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the task event/context/team artifact",
    )
    require(
        "task_event_context_team_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the task event/context/team contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP clawcli_exec_replay_contracts" in parity_text,
        findings,
        "agent parity gate must run the clawcli exec/replay contract guard step",
    )
    require(
        "check_clawcli_exec_replay_contracts.py" in parity_text
        and 'check_clawcli_exec_replay_contracts.py" --self-test' in parity_text
        and "clawcli_exec_replay_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the clawcli exec/replay artifact",
    )
    require(
        "clawcli_exec_replay_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the clawcli exec/replay contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP clawcli_session_tui_contracts" in parity_text,
        findings,
        "agent parity gate must run the clawcli session/TUI contract guard step",
    )
    require(
        "check_clawcli_session_tui_contracts.py" in parity_text
        and 'check_clawcli_session_tui_contracts.py" --self-test' in parity_text
        and "clawcli_session_tui_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the clawcli session/TUI artifact",
    )
    require(
        "clawcli_session_tui_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the clawcli session/TUI contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP clawcli_goal_contracts" in parity_text,
        findings,
        "agent parity gate must run the clawcli goal contract guard step",
    )
    require(
        "check_clawcli_goal_contracts.py" in parity_text
        and 'check_clawcli_goal_contracts.py" --self-test' in parity_text
        and "clawcli_goal_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the clawcli goal artifact",
    )
    require(
        "clawcli_goal_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the clawcli goal contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP clawcli_llm_trace_contracts" in parity_text,
        findings,
        "agent parity gate must run the clawcli LLM trace contract guard step",
    )
    require(
        "check_clawcli_llm_trace_contracts.py" in parity_text
        and 'check_clawcli_llm_trace_contracts.py" --self-test' in parity_text
        and "clawcli_llm_trace_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the clawcli LLM trace artifact",
    )
    require(
        "clawcli_llm_trace_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the clawcli LLM trace contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP clawcli_models_catalog_contracts" in parity_text,
        findings,
        "agent parity gate must run the clawcli models catalog contract guard step",
    )
    require(
        "check_clawcli_models_catalog_contracts.py" in parity_text
        and 'check_clawcli_models_catalog_contracts.py" --self-test' in parity_text
        and "clawcli_models_catalog_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the clawcli models catalog artifact",
    )
    require(
        "clawcli_models_catalog_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the clawcli models catalog contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP clawcli_models_readiness_contracts" in parity_text,
        findings,
        "agent parity gate must run the clawcli models readiness contract guard step",
    )
    require(
        "check_clawcli_models_readiness_contracts.py" in parity_text
        and 'check_clawcli_models_readiness_contracts.py" --self-test' in parity_text
        and "clawcli_models_readiness_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the clawcli models readiness artifact",
    )
    require(
        "clawcli_models_readiness_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the clawcli models readiness contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP no_agent_mode_payload" in parity_text,
        findings,
        "agent parity gate must run the no-agent-mode payload guard step",
    )
    require(
        "check_no_agent_mode_payload.py" in parity_text
        and 'check_no_agent_mode_payload.py" --self-test' in parity_text
        and "no_agent_mode_payload.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the no-agent-mode payload guard artifact",
    )
    require(
        "no_agent_mode_payload=1" in parity_text,
        findings,
        "agent parity gate summary must record the no-agent-mode payload guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP agent_loop_static_contracts" in parity_text,
        findings,
        "agent parity gate must run the agent-loop static contracts step",
    )
    require(
        "check_route_authority_legacy_keys.py" in parity_text
        and "check_legacy_route_boundary.py" in parity_text
        and "check_pre_planner_exit_inventory.py" in parity_text
        and "check_frontdoor_boundary_dispatch.py" in parity_text
        and "check_no_nl_hardmatch.py" in parity_text
        and "check_historical_hardcoded_language.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_route_authority_legacy_keys.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_legacy_route_boundary.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_pre_planner_exit_inventory.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_frontdoor_boundary_dispatch.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_no_nl_hardmatch.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_historical_hardcoded_language.py" in parity_text
        and "agent_loop_static_contracts.txt" in parity_text,
        findings,
        "agent parity gate must write the agent-loop static contracts artifact",
    )
    require(
        "agent_loop_static_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the agent-loop static contracts state",
    )
    require(
        "AGENT_PARITY_GATE_STEP semantic_boundary_contracts" in parity_text
        and "check_runtime_semantic_rewrite_boundary.py" in parity_text
        and 'check_runtime_semantic_rewrite_boundary.py" --self-test' in parity_text
        and "check_contract_repair_loop_observation_boundary.py" in parity_text
        and 'check_contract_repair_loop_observation_boundary.py" --self-test' in parity_text
        and "check_route_reason_marker_facade.py" in parity_text
        and 'check_route_reason_marker_facade.py" --self-test' in parity_text
        and "check_output_semantic_kind_write_boundary.py" in parity_text
        and 'check_output_semantic_kind_write_boundary.py" --self-test' in parity_text
        and "semantic_boundary_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the semantic boundary contracts artifact",
    )
    require(
        "semantic_boundary_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the semantic boundary contracts state",
    )
    require(
        "AGENT_PARITY_GATE_STEP evidence_extractor_contracts" in parity_text
        and "check_evidence_extractor_contracts.py" in parity_text
        and 'check_evidence_extractor_contracts.py" --self-test' in parity_text
        and "evidence_extractor_contracts.txt" in parity_text,
        findings,
        "agent parity gate must write the evidence extractor contract artifact",
    )
    require(
        "evidence_extractor_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the evidence extractor contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP secret_scan_contract" in parity_text,
        findings,
        "agent parity gate must run the shared secret scan contract step",
    )
    require(
        "check_secret_scan_contract.py" in parity_text
        and "secret_scan_contract.json" in parity_text,
        findings,
        "agent parity gate must write the shared secret scan contract artifact",
    )
    require(
        "secret_scan_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the shared secret scan contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP suite_wrapper_contract" in parity_text,
        findings,
        "agent parity gate must run the wrapped suite contract step",
    )
    require(
        "check_suite_wrapper_contract.py" in parity_text
        and "suite_wrapper_contract.json" in parity_text,
        findings,
        "agent parity gate must write the wrapped suite contract artifact",
    )
    require(
        "suite_wrapper_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the wrapped suite contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP runner_path_ref_contract" in parity_text,
        findings,
        "agent parity gate must run the runner path-ref contract step",
    )
    require(
        "check_runner_path_ref_contract.py" in parity_text
        and "runner_path_ref_contract.json" in parity_text,
        findings,
        "agent parity gate must write the runner path-ref contract artifact",
    )
    require(
        "runner_path_ref_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the runner path-ref contract state",
    )
    require(
        "SUITE_ARTIFACT_CONTRACT" in suite_wrapper_text
        and "AGENT_PARITY_GATE_REQUIRED_ARTIFACTS" in suite_wrapper_text
        and "AGENT_PARITY_GATE_REQUIRED_FLAGS" in suite_wrapper_text
        and "AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS" in suite_wrapper_text
        and "runner_path_ref_contract" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_live_provider_scope" in suite_wrapper_text
        and "agent_parity_gate_summary_missing_live_provider_scope" in suite_wrapper_text
        and "validate_chinese_provider_env_file_summary" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_env_file_state" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_env_file_source" in suite_wrapper_text
        and "validate_gate_summary_no_host_paths" in suite_wrapper_text
        and "agent_parity_gate_summary_host_path" in suite_wrapper_text
        and "agent_parity_gate_summary_legacy_out_dir" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_out_dir_ref" in suite_wrapper_text
        and "gate-summary-host-path" in suite_wrapper_text
        and "out_dir_ref" in suite_wrapper_text
        and "RUN_SUITE_FORBIDDEN_SNIPPETS" in suite_wrapper_text
        and "RUN_MULTI_TURN_SUITE" in suite_wrapper_text
        and "RUN_MULTI_TURN_FORBIDDEN_SNIPPETS" in suite_wrapper_text
        and "multi_turn_run_dir_ref" in suite_wrapper_text
        and "multi_turn_run_log_ref" in suite_wrapper_text
        and "run_dir_ref" in suite_wrapper_text
        and "run_log_ref" in suite_wrapper_text
        and "suite_artifact_contract_ref" in suite_wrapper_text
        and "clarify_run_dir_ref" in suite_wrapper_text
        and "clarify_run_log_ref" in suite_wrapper_text
        and "clarify_summary_jsonl_ref" in suite_wrapper_text
        and "context_run_dir_ref" in suite_wrapper_text
        and "context_run_log_ref" in suite_wrapper_text
        and "context_summary_jsonl_ref" in suite_wrapper_text
        and "agent-parity-run-log-host-path" in suite_wrapper_text
        and "validate_provider_smoke_path_refs" in suite_wrapper_text
        and "agent_parity_gate_provider_smoke_bad_path_ref" in suite_wrapper_text
        and "agent_parity_gate_provider_smoke_case_coverage_bad_case_file" in suite_wrapper_text
        and "rollout_metrics_contract" in suite_wrapper_text
        and "agent_parity_gate_metrics_host_path" in suite_wrapper_text
        and "--validate-contract-report-content" in suite_wrapper_text
        and "--require-contract-report-content-checked" in suite_wrapper_text
        and "validate_existing_contract_report" in suite_wrapper_text
        and "SUITE_ARTIFACT_CONTRACT_FORBIDDEN_SNIPPETS" in suite_wrapper_text
        and "check_forbidden_snippets" in suite_wrapper_text
        and "forbidden_snippet" in suite_wrapper_text
        and "agent_parity_gate_contract" in suite_wrapper_text,
        findings,
        "wrapped suite contract guard must statically protect agent parity nested artifact checks",
    )
    require(
        "AGENT_PARITY_GATE_REQUIRED_ARTIFACTS" in suite_artifact_contract_text
        and "agent_parity_gate/runtime_hard_reply_baseline.txt" in suite_artifact_contract_text
        and "agent_parity_gate/policy_boundary_hard_reply.txt" in suite_artifact_contract_text
        and "agent_parity_gate/repair_no_user_text_fields.txt" in suite_artifact_contract_text
        and "agent_parity_gate/policy_decision_tokens.txt" in suite_artifact_contract_text
        and "agent_parity_gate/agent_loop_guard_final_scope.txt" in suite_artifact_contract_text
        and "agent_parity_gate/registry_policy_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/skill_registry_aliases.txt" in suite_artifact_contract_text
        and "agent_parity_gate/long_tail_skill_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/task_lifecycle_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/task_event_context_team_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/clawcli_exec_replay_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/clawcli_session_tui_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/clawcli_goal_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/clawcli_llm_trace_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/clawcli_models_catalog_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/clawcli_models_readiness_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/agent_loop_static_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/evidence_extractor_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/suite_wrapper_contract.json" in suite_artifact_contract_text
        and "agent_parity_gate/runner_path_ref_contract.json" in suite_artifact_contract_text
        and "agent_parity_gate/suite_artifact_contract_self_test.txt" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_REQUIRED_FLAGS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_TEXT_CONTENT_TOKENS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_JSON_OK_ARTIFACTS" in suite_artifact_contract_text
        and '"runtime_hard_reply_baseline": "1"' in suite_artifact_contract_text
        and '"policy_boundary_hard_reply": "1"' in suite_artifact_contract_text
        and '"repair_no_user_text_fields": "1"' in suite_artifact_contract_text
        and '"policy_decision_tokens": "1"' in suite_artifact_contract_text
        and '"agent_loop_guard_final_scope": "1"' in suite_artifact_contract_text
        and '"registry_policy_contracts": "1"' in suite_artifact_contract_text
        and '"skill_registry_aliases": "1"' in suite_artifact_contract_text
        and '"long_tail_skill_contracts": "1"' in suite_artifact_contract_text
        and '"task_lifecycle_contracts": "1"' in suite_artifact_contract_text
        and '"task_event_context_team_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_exec_replay_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_session_tui_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_goal_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_llm_trace_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_models_catalog_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_models_readiness_contracts": "1"' in suite_artifact_contract_text
        and "RUNTIME_HARD_REPLY_ALL_SCAN" in suite_artifact_contract_text
        and "new=0" in suite_artifact_contract_text
        and "POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok" in suite_artifact_contract_text
        and "POLICY_BOUNDARY_HARD_REPLY_CHECK ok" in suite_artifact_contract_text
        and "REPAIR_USER_TEXT_FIELD_CHECK ok" in suite_artifact_contract_text
        and "POLICY_DECISION_TOKEN_SELF_TEST ok" in suite_artifact_contract_text
        and "POLICY_DECISION_TOKEN_CHECK ok" in suite_artifact_contract_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok" in suite_artifact_contract_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0" in suite_artifact_contract_text
        and "REGISTRY_POLICY_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "REGISTRY_POLICY_CONTRACT_CHECK ok" in suite_artifact_contract_text
        and "SKILL_REGISTRY_ALIAS_SELF_TEST ok" in suite_artifact_contract_text
        and "SKILL_REGISTRY_ALIAS_CHECK ok" in suite_artifact_contract_text
        and "LONG_TAIL_SKILL_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "LONG_TAIL_SKILL_CONTRACT_CHECK ok" in suite_artifact_contract_text
        and "TASK_LIFECYCLE_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "TASK_LIFECYCLE_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "CLAWCLI_GOAL_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_route_authority_legacy_keys.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_legacy_route_boundary.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_pre_planner_exit_inventory.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_frontdoor_boundary_dispatch.py" in suite_artifact_contract_text
        and "FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_no_nl_hardmatch.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_historical_hardcoded_language.py" in suite_artifact_contract_text
        and "agent_parity_gate/semantic_boundary_contracts.txt" in suite_artifact_contract_text
        and '"semantic_boundary_contracts": "1"' in suite_artifact_contract_text
        and "RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0" in suite_artifact_contract_text
        and "CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0" in suite_artifact_contract_text
        and "ROUTE_REASON_MARKER_FACADE_SELF_TEST ok" in suite_artifact_contract_text
        and "ROUTE_REASON_MARKER_FACADE_CHECK findings=0" in suite_artifact_contract_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok" in suite_artifact_contract_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_CHECK findings=0" in suite_artifact_contract_text
        and '"evidence_extractor_contracts": "1"' in suite_artifact_contract_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and '"runner_path_ref_contract": "1"' in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_OPTIONAL_ARTIFACTS_BY_FLAG" in suite_artifact_contract_text
        and "AGENT_PARITY_CHINESE_MODEL_PROVIDERS" in suite_artifact_contract_text
        and "validate_text_artifact_tokens" in suite_artifact_contract_text
        and "agent_parity_gate_artifact_decode_failed" in suite_artifact_contract_text
        and "text-artifact-decode-failed" in suite_artifact_contract_text
        and "validate_json_artifact_ok" in suite_artifact_contract_text
        and "agent_parity_gate_artifact_bad_shape" in suite_artifact_contract_text
        and "json-ok-artifact-bad-shape" in suite_artifact_contract_text
        and "validate_compact_coverage_artifact" in suite_artifact_contract_text
        and "validate_chinese_model_catalog_artifact" in suite_artifact_contract_text
        and "chinese_model_catalog_self_test.txt" in suite_artifact_contract_text
        and "CHINESE_MODEL_CATALOG_SELF_TEST ok" in suite_artifact_contract_text
        and (
            "agent_parity_gate_chinese_model_catalog_bad_catalog_shape"
            in suite_artifact_contract_text
        )
        and "chinese-model-catalog-bad-catalog-shape" in suite_artifact_contract_text
        and "validate_provider_smoke_artifacts" in suite_artifact_contract_text
        and (
            "agent_parity_gate_provider_smoke_bad_providers_shape"
            in suite_artifact_contract_text
        )
        and "provider-smoke-bad-providers-shape" in suite_artifact_contract_text
        and "validate_provider_smoke_case_coverage" in suite_artifact_contract_text
        and (
            "agent_parity_gate_provider_smoke_case_coverage_bad_provider_tags"
            in suite_artifact_contract_text
        )
        and "provider-case-coverage-bad-provider-tags" in suite_artifact_contract_text
        and "agent_parity_gate_provider_smoke_case_coverage_bad_case_file" in suite_artifact_contract_text
        and "provider-case-coverage-bad-case-file" in suite_artifact_contract_text
        and "parse_provider_summary_jsonl" in suite_artifact_contract_text
        and "agent_parity_gate_provider_summary_bad_json_line" in suite_artifact_contract_text
        and "agent_parity_gate_provider_summary_bad_row" in suite_artifact_contract_text
        and "provider-summary-jsonl-row-errors" in suite_artifact_contract_text
        and "validate_provider_smoke_path_refs" in suite_artifact_contract_text
        and "agent_parity_gate_provider_smoke_bad_path_ref" in suite_artifact_contract_text
        and "provider_path_ref_errors" in suite_artifact_contract_text
        and "parse_live_provider_scope" in suite_artifact_contract_text
        and "validate_live_provider_scope" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_live_provider_scope" in suite_artifact_contract_text
        and "live_provider_scope" in suite_artifact_contract_text
        and "validate_chinese_provider_env_file_summary" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_env_file_state" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_env_file_source" in suite_artifact_contract_text
        and "env_file_summary" in suite_artifact_contract_text
        and "validate_gate_summary_no_host_paths" in suite_artifact_contract_text
        and "agent_parity_gate_summary_host_path" in suite_artifact_contract_text
        and "agent_parity_gate_summary_legacy_out_dir" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_out_dir_ref" in suite_artifact_contract_text
        and "gate-summary-host-path" in suite_artifact_contract_text
        and "out_dir_ref" in suite_artifact_contract_text
        and 'validate_text_artifact_no_host_paths(run_dir, "run.log")'
        in suite_artifact_contract_text
        and "agent_parity_gate_artifact_host_path:run.log" in suite_artifact_contract_text
        and "agent-parity-run-log-host-path" in suite_artifact_contract_text
        and "expected_live_scope_providers" in suite_artifact_contract_text
        and "provider_not_in_live_scope" in suite_artifact_contract_text
        and "validate_rollout_metrics_artifact" in suite_artifact_contract_text
        and "rollout_metrics_contract.txt" in suite_artifact_contract_text
        and "ROLLOUT_METRICS_SELF_TEST ok" in suite_artifact_contract_text
        and "agent_parity_gate_metrics_host_path" in suite_artifact_contract_text
        and "agent_parity_gate_metrics_bad_source_run_dir" in suite_artifact_contract_text
        and "rollout_metrics_text_host_path" in suite_artifact_contract_text
        and "load_json_artifact" in suite_artifact_contract_text
        and "load-json-artifact-bad-shape" in suite_artifact_contract_text
        and "load-json-artifact-decode-failed" in suite_artifact_contract_text
        and "summary_decode_failed" in suite_artifact_contract_text
        and "artifact_index_decode_failed" in suite_artifact_contract_text
        and "summary-decode-failed" in suite_artifact_contract_text
        and "artifact-index-decode-failed" in suite_artifact_contract_text
        and "validate_enabled_agent_parity_optional_artifacts" in suite_artifact_contract_text
        and "agent_parity_gate_summary_missing" in suite_artifact_contract_text
        and "agent-parity-missing-gate-summary" in suite_artifact_contract_text
        and "return findings, content_checks" in suite_artifact_contract_text
        and "validate_existing_contract_report" in suite_artifact_contract_text
        and "--validate-contract-report-content" in suite_artifact_contract_text
        and "--require-contract-report-content-checked" in suite_artifact_contract_text
        and '"contract_report_content_checked"' in suite_artifact_contract_text
        and "stored_agent_contract" in suite_artifact_contract_text
        and "stored_report_override" in suite_artifact_contract_text
        and "contract_report_missing" in suite_artifact_contract_text
        and "contract_report_read_failed" in suite_artifact_contract_text
        and "contract_report_decode_failed" in suite_artifact_contract_text
        and "contract-report-decode-failed" in suite_artifact_contract_text
        and "contract_report_bad_json" in suite_artifact_contract_text
        and "contract_report_bad_shape" in suite_artifact_contract_text
        and "contract_report_not_ok" in suite_artifact_contract_text
        and "contract_report_bad_run_dir" in suite_artifact_contract_text
        and "contract_report_bad_require_contract_report" in suite_artifact_contract_text
        and "contract_report_findings_not_empty" in suite_artifact_contract_text
        and "contract_report_content_checked_not_true" in suite_artifact_contract_text
        and "contract_report_summary_mismatch" in suite_artifact_contract_text
        and "contract_report_agent_parity_contract_mismatch" in suite_artifact_contract_text
        and "contract_report_unexpected_agent_parity_contract" in suite_artifact_contract_text
        and "unexpected_agent_contract" in suite_artifact_contract_text
        and "missing-contract-report" in suite_artifact_contract_text
        and "read-failed" in suite_artifact_contract_text
        and "bad-json" in suite_artifact_contract_text
        and "bad-shape" in suite_artifact_contract_text
        and '"agent_loop_static_contracts": "1"' in suite_artifact_contract_text
        and '"semantic_boundary_contracts": "1"' in suite_artifact_contract_text
        and '"agent_loop_guard_final_scope": "1"' in suite_artifact_contract_text
        and '"registry_policy_contracts": "1"' in suite_artifact_contract_text
        and '"skill_registry_aliases": "1"' in suite_artifact_contract_text
        and '"long_tail_skill_contracts": "1"' in suite_artifact_contract_text
        and '"task_lifecycle_contracts": "1"' in suite_artifact_contract_text
        and '"task_event_context_team_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_exec_replay_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_session_tui_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_goal_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_llm_trace_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_models_catalog_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_models_readiness_contracts": "1"' in suite_artifact_contract_text
        and '"evidence_extractor_contracts": "1"' in suite_artifact_contract_text
        and '"suite_wrapper_contract": "1"' in suite_artifact_contract_text
        and '"suite_artifact_contract_self_test": "1"' in suite_artifact_contract_text
        and "SUITE_ARTIFACT_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and '"live_metrics": {"0", "1"}' in suite_artifact_contract_text
        and (
            'live_metrics_enabled = gate_summary.get("live_metrics") == "1"'
            in suite_artifact_contract_text
        )
        and "agent_parity_gate_summary_bad_machine_field" in suite_artifact_contract_text
        and '"required_machine_field_count"' in suite_artifact_contract_text
        and "len(AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS)" in suite_artifact_contract_text
        and '"content_check_count"' in suite_artifact_contract_text
        and 'summary.get("suite") == "agent_parity_gate"' in suite_artifact_contract_text
        and '"agent_parity_gate_contract"' in suite_artifact_contract_text,
        findings,
        "suite artifact contract checker must verify wrapped agent parity nested artifacts, flags, and success content",
    )
    check_agent_parity_auxiliary_gate_steps(
        parity_text=parity_text,
        suite_artifact_contract_text=suite_artifact_contract_text,
        findings=findings,
        require=require,
    )
    require(
        'if [[ -n "${NL_SUITE_RUN_DIR:-}" ]]' in parity_text
        and 'OUT_DIR="${NL_SUITE_RUN_DIR}/agent_parity_gate"' in parity_text,
        findings,
        "agent parity gate must co-locate artifacts under NL_SUITE_RUN_DIR when wrapped by run_suite",
    )
    require(
        "path_ref()" in parity_text
        and 'AGENT_PARITY_GATE out_dir_ref=$(path_ref "$OUT_DIR")' in parity_text
        and 'echo "out_dir_ref=$(path_ref "$OUT_DIR")"' in parity_text
        and 'AGENT_PARITY_GATE_OK out_dir_ref=$(path_ref "$OUT_DIR")' in parity_text
        and 'out_dir=${OUT_DIR}' not in parity_text,
        findings,
        "agent parity gate must report portable out_dir refs instead of host paths",
    )
    for label, readme_body in (("README.md", readme_text), ("README.zh-CN.md", readme_zh_text)):
        require(
            "agent_loop_static_contracts.txt" in readme_body
            and "runtime_hard_reply_baseline.txt" in readme_body
            and "policy_boundary_hard_reply.txt" in readme_body
            and "repair_no_user_text_fields.txt" in readme_body
            and "policy_decision_tokens.txt" in readme_body
            and "agent_loop_guard_final_scope.txt" in readme_body
            and "registry_policy_contracts.txt" in readme_body
            and "skill_registry_aliases.txt" in readme_body
            and "long_tail_skill_contracts.txt" in readme_body
            and "task_lifecycle_contracts.txt" in readme_body
            and "task_event_context_team_contracts.txt" in readme_body
            and "clawcli_exec_replay_contracts.txt" in readme_body
            and "clawcli_session_tui_contracts.txt" in readme_body
            and "clawcli_goal_contracts.txt" in readme_body
            and "clawcli_llm_trace_contracts.txt" in readme_body
            and "clawcli_models_catalog_contracts.txt" in readme_body
            and "clawcli_models_readiness_contracts.txt" in readme_body
            and "no_agent_mode_payload.txt" in readme_body
            and "semantic_boundary_contracts.txt" in readme_body
            and "evidence_extractor_contracts.txt" in readme_body
            and "self-test" in readme_body
            and "check_evidence_extractor_contracts.py --self-test" in readme_body
            and "suite_artifact_contract.json" in readme_body
            and "suite_artifact_contract_self_test.txt" in readme_body
            and "chinese_model_catalog_self_test.txt" in readme_body
            and "agent_parity_gate_contract.checked=true" in readme_body
            and "--validate-contract-report-content" in readme_body
            and "--require-contract-report-content-checked" in readme_body
            and "contract_report_content_checked=true" in readme_body
            and "live_metrics=0|1" in readme_body
            and "metrics=1" in readme_body
            and "live_metrics=1" in readme_body
            and "out_dir_ref" in readme_body
            and "run_dir_ref" in readme_body
            and "run_log_ref" in readme_body
            and "runner_path_ref_contract.json" in readme_body
            and "llm_raw_trace_runner_contract.txt" in readme_body,
            findings,
            f"{label} must document agent parity nested/static/raw-trace gate artifacts",
        )
        require(
            "check_frontdoor_boundary_dispatch.py" in readme_body
            and "FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0" in readme_body,
            findings,
            f"{label} must document the frontdoor boundary static guard",
        )
        require(
            "semantic_boundary_contracts.txt" in readme_body
            and "RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0" in readme_body
            and "CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0" in readme_body
            and "ROUTE_REASON_MARKER_FACADE_SELF_TEST ok" in readme_body
            and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok" in readme_body,
            findings,
            f"{label} must document the semantic boundary contracts artifact",
        )
    require(
        "runtime_hard_reply_baseline.txt" in nl_tests_readme_text
        and "runtime_hard_reply_baseline=1" in nl_tests_readme_text
        and "RUNTIME_HARD_REPLY_ALL_SCAN" in nl_tests_readme_text
        and "new=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document runtime hard-reply baseline artifact content",
    )
    require(
        "policy_boundary_hard_reply.txt" in nl_tests_readme_text
        and "policy_boundary_hard_reply=1" in nl_tests_readme_text
        and "POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok" in nl_tests_readme_text
        and "POLICY_BOUNDARY_HARD_REPLY_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document policy-boundary hard-reply artifact content",
    )
    require(
        "repair_no_user_text_fields.txt" in nl_tests_readme_text
        and "repair_no_user_text_fields=1" in nl_tests_readme_text
        and "REPAIR_USER_TEXT_FIELD_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document repair no-user-text artifact content",
    )
    require(
        "policy_decision_tokens.txt" in nl_tests_readme_text
        and "policy_decision_tokens=1" in nl_tests_readme_text
        and "POLICY_DECISION_TOKEN_SELF_TEST ok" in nl_tests_readme_text
        and "POLICY_DECISION_TOKEN_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document policy decision token artifact content",
    )
    require(
        "agent_loop_guard_final_scope.txt" in nl_tests_readme_text
        and "agent_loop_guard_final_scope=1" in nl_tests_readme_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok" in nl_tests_readme_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document final agent-loop guard scope artifact content",
    )
    require(
        "registry_policy_contracts.txt" in nl_tests_readme_text
        and "registry_policy_contracts=1" in nl_tests_readme_text
        and "REGISTRY_POLICY_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "REGISTRY_POLICY_CONTRACT_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document registry policy contract artifact content",
    )
    require(
        "skill_registry_aliases.txt" in nl_tests_readme_text
        and "skill_registry_aliases=1" in nl_tests_readme_text
        and "SKILL_REGISTRY_ALIAS_SELF_TEST ok" in nl_tests_readme_text
        and "SKILL_REGISTRY_ALIAS_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document skill registry aliases artifact content",
    )
    require(
            "long_tail_skill_contracts.txt" in nl_tests_readme_text
            and "long_tail_skill_contracts=1" in nl_tests_readme_text
            and "LONG_TAIL_SKILL_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
            and "LONG_TAIL_SKILL_CONTRACT_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document long-tail skill contract artifact content",
    )
    require(
        "task_lifecycle_contracts.txt" in nl_tests_readme_text
        and "task_lifecycle_contracts=1" in nl_tests_readme_text
        and "TASK_LIFECYCLE_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "TASK_LIFECYCLE_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document task lifecycle contract artifact content",
    )
    require(
        "task_event_context_team_contracts.txt" in nl_tests_readme_text
        and "task_event_context_team_contracts=1" in nl_tests_readme_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document task event/context/team artifact content",
    )
    require(
        "clawcli_exec_replay_contracts.txt" in nl_tests_readme_text
        and "clawcli_exec_replay_contracts=1" in nl_tests_readme_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document clawcli exec/replay artifact content",
    )
    require(
        "clawcli_session_tui_contracts.txt" in nl_tests_readme_text
        and "clawcli_session_tui_contracts=1" in nl_tests_readme_text
        and "CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document clawcli session/TUI artifact content",
    )
    require(
        "clawcli_goal_contracts.txt" in nl_tests_readme_text
        and "clawcli_goal_contracts=1" in nl_tests_readme_text
        and "CLAWCLI_GOAL_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "CLAWCLI_GOAL_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document clawcli goal artifact content",
    )
    require(
        "clawcli_llm_trace_contracts.txt" in nl_tests_readme_text
        and "clawcli_llm_trace_contracts=1" in nl_tests_readme_text
        and "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document clawcli LLM trace artifact content",
    )
    require(
        "clawcli_models_catalog_contracts.txt" in nl_tests_readme_text
        and "clawcli_models_catalog_contracts=1" in nl_tests_readme_text
        and "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document clawcli models catalog artifact content",
    )
    require(
        "clawcli_models_readiness_contracts.txt" in nl_tests_readme_text
        and "clawcli_models_readiness_contracts=1" in nl_tests_readme_text
        and "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document clawcli models readiness artifact content",
    )
    require(
        "evidence_extractor_contracts.txt" in nl_tests_readme_text
        and "AGENT_LOOP_STATIC_SELF_TEST" in nl_tests_readme_text
        and "check_frontdoor_boundary_dispatch.py" in nl_tests_readme_text
        and "FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0" in nl_tests_readme_text
        and "evidence_extractor_contracts=1" in nl_tests_readme_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings=0" in nl_tests_readme_text
        and "agent_parity_gate/evidence_extractor_contracts.txt" in nl_tests_readme_text,
        findings,
        "scripts/nl_tests/README.md must document the evidence extractor gate artifact",
    )
    require(
        "semantic_boundary_contracts.txt" in nl_tests_readme_text
        and "semantic_boundary_contracts=1" in nl_tests_readme_text
        and "RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0" in nl_tests_readme_text
        and "CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0" in nl_tests_readme_text
        and "ROUTE_REASON_MARKER_FACADE_SELF_TEST ok" in nl_tests_readme_text
        and "ROUTE_REASON_MARKER_FACADE_CHECK findings=0" in nl_tests_readme_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok" in nl_tests_readme_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document semantic boundary contract artifact content",
    )
