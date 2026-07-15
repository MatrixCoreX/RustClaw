#!/usr/bin/env python3
"""Agent parity gate assertions used by the Chinese model catalog guard."""

from __future__ import annotations

from collections.abc import Callable

RequireFn = Callable[[bool, list[str], str], None]


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
