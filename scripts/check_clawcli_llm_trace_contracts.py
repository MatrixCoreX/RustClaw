#!/usr/bin/env python3
"""Validate clawcli LLM trace machine contracts."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawcli/src/commands/llm_trace.rs": (
        "pub(crate) fn run_llm_trace",
        "fn fetch_task_llm_trace",
        "pub(super) fn llm_trace_text_lines",
        "fn llm_call_summary_line",
        "fn push_raw_field_line",
        "fn debug_calls",
        "debug/tasks",
        "llm_trace_task_id",
        "llm_trace_goal_id",
        "llm_trace_session_id",
        "llm_trace_call_count",
        "flow_summary",
        "llm_trace_flow_stage_count",
        "llm_trace_retry_count",
        "llm_trace_verifier_call_count",
        "llm_trace_finalizer_call_count",
        "llm_trace_provider_error_count",
        "llm_trace_model_readiness_line",
        "llm_trace_model_readiness:",
        "trace_ref=model_catalog_trace.readiness",
        "MODEL_READINESS_SCALAR_FIELDS",
        "MODEL_READINESS_BOOL_FIELDS",
        "model_catalog_trace/readiness",
        "selected_entry_status",
        "matched_entry_count",
        "credential_state",
        "ready",
        "image_understanding",
        "video_generation",
        "music_generation",
        "dry_run",
        "llm_call_ref=LLM#",
        "index={call_index}",
        "status",
        "vendor",
        "provider",
        "provider_type",
        "model",
        "model_kind",
        "prompt_label",
        "flow_stage",
        "flow_node",
        "code_module",
        "code_entrypoint",
        "trigger_kind",
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "request_payload",
        "response",
        "clean_response",
        "raw_response",
        "error",
    ),
    "crates/clawcli/src/main.rs": (
        "Command::LlmTrace",
        "commands::run_llm_trace",
        "raw",
        "limit",
    ),
    "crates/clawcli/src/main_tests.rs": (
        "llm-trace",
    ),
    "crates/clawcli/src/commands_llm_trace_tests.rs": (
        "llm_trace_text_lines_number_calls_and_flow_tokens",
        "llm_trace_text_lines_project_missing_model_readiness",
        "llm_trace_model_readiness:",
        "trace_ref=model_catalog_trace.readiness",
        "selected_entry_status=found",
        "selected_entry_status=missing",
        "credential_state=configured_env",
        "credential_state=null",
        "ready=true",
        "ready=false",
        "llm_trace_text_lines_limit_and_raw_fields",
        "llm_call_ref=LLM#1",
        "llm_call_ref=LLM#2",
        "TRACE_INPUT_TOKEN",
        "TRACE_RESPONSE_TOKEN",
        "TRACE_RAW_TOKEN",
        "SHOULD_BE_LIMITED_OUT",
        "request_payload",
        "raw_response",
        "prompt_label=answer_verifier",
        "flow_stage=agent_loop.answer_verifier",
    ),
    "UI/src/lib/task-llm-trace.ts": (
        "flowSummaryMachineTokens",
        "agentFlowTimelineRows",
        "modelCatalogTraceMachineTokens",
        "resumeTraceMachineTokens",
        "flow_stage",
        "codeModules",
        "codeEntrypoints",
        "provider_error_count",
    ),
    "UI/src/lib/task-llm-debug-display.ts": (
        "taskLlmDebugCallMetaTokens",
        "taskLlmDebugRawFields",
        "taskLlmDebugRequestData",
        "taskLlmDebugResponseData",
        "request_payload",
        "raw_response",
        "clean_response",
        "response",
        "error",
    ),
    "UI/src/lib/task-llm-trace.test.ts": (
        "flowSummaryMachineTokens",
        "agentFlowTimelineRows",
        "modelCatalogTraceMachineTokens",
        "resumeTraceMachineTokens",
    ),
    "UI/src/lib/task-llm-debug-display.test.ts": (
        "reads teaching trace payloads",
        "keeps compatibility with flat teaching trace calls",
    ),
    "README.md": (
        "clawcli llm-trace",
        "LLM#1",
        "llm_call_ref=LLM#",
        "raw request/response",
        "clawcli_llm_trace_contracts.txt",
        "clawcli_llm_trace_contracts=1",
        "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok",
        "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0",
    ),
    "README.zh-CN.md": (
        "clawcli llm-trace",
        "LLM#1",
        "llm_call_ref=LLM#",
        "raw request/response",
        "clawcli_llm_trace_contracts.txt",
        "clawcli_llm_trace_contracts=1",
        "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok",
        "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/README.md": (
        "clawcli_llm_trace_contracts.txt",
        "clawcli_llm_trace_contracts=1",
        "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok",
        "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0",
        "LLM#1",
        "raw request/response",
    ),
    "AGENTS.md": (
        "scripts/check_clawcli_llm_trace_contracts.py",
        "clawcli_llm_trace_contracts.txt",
        "clawcli_llm_trace_contracts=1",
        "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok",
        "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/run_agent_parity_gate.sh": (
        "AGENT_PARITY_GATE_STEP clawcli_llm_trace_contracts",
        "check_clawcli_llm_trace_contracts.py",
        "clawcli_llm_trace_contracts.txt",
        "clawcli_llm_trace_contracts=1",
    ),
    "scripts/nl_tests/check_suite_artifact_contract.py": (
        "agent_parity_gate/clawcli_llm_trace_contracts.txt",
        '"clawcli_llm_trace_contracts": "1"',
        "CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok",
        "CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0",
    ),
}


def read_repo_texts() -> dict[str, str | None]:
    texts: dict[str, str | None] = {}
    for rel_path in REQUIRED_TOKENS_BY_PATH:
        try:
            texts[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            texts[rel_path] = None
    return texts


def scan_texts(texts: dict[str, str | None]) -> list[str]:
    findings: list[str] = []
    for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token not in text:
                findings.append(f"missing_token:{rel_path}:{token}")

    llm_trace = texts.get("crates/clawcli/src/commands/llm_trace.rs") or ""
    for token in (
        "llm_call_ref=LLM#",
        "llm_trace_model_readiness:",
        "trace_ref=model_catalog_trace.readiness",
        "selected_entry_status",
        "credential_state",
        "ready",
        "request_payload",
        "raw_response",
        "clean_response",
        "flow_stage",
        "code_module",
        "code_entrypoint",
    ):
        if token not in llm_trace:
            findings.append(f"llm_trace_contract_token_missing:{token}")
    if llm_trace.count("push_token") < 10:
        findings.append("llm_trace_machine_projection_too_weak")

    tests = texts.get("crates/clawcli/src/commands_llm_trace_tests.rs") or ""
    for token in (
        "LLM#1",
        "LLM#2",
        "TRACE_INPUT_TOKEN",
        "TRACE_RAW_TOKEN",
        "llm_trace_model_readiness:",
        "selected_entry_status=missing",
        "ready=false",
    ):
        if token not in tests:
            findings.append(f"missing_llm_trace_test_token:{token}")

    ui_debug = texts.get("UI/src/lib/task-llm-debug-display.ts") or ""
    for token in ("request_payload", "raw_response", "clean_response", "response", "error"):
        if token not in ui_debug:
            findings.append(f"ui_llm_debug_raw_field_missing:{token}")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    texts["crates/clawcli/src/commands/llm_trace.rs"] += "\n" + "\n".join(
        [
            *["push_token" for _ in range(10)],
            "llm_call_ref=LLM#",
            "llm_trace_model_readiness:",
            "trace_ref=model_catalog_trace.readiness",
            "selected_entry_status",
            "credential_state",
            "ready",
            "request_payload",
            "raw_response",
            "clean_response",
            "flow_stage",
            "code_module",
            "code_entrypoint",
        ]
    )
    texts["crates/clawcli/src/commands_llm_trace_tests.rs"] += (
        "\nLLM#1\nLLM#2\nTRACE_INPUT_TOKEN\nTRACE_RAW_TOKEN\n"
        "llm_trace_model_readiness:\nselected_entry_status=missing\nready=false\n"
    )
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    good_findings = scan_texts(good)
    assert not good_findings, good_findings

    missing_ref = dict(good)
    missing_ref["crates/clawcli/src/commands/llm_trace.rs"] = (
        missing_ref["crates/clawcli/src/commands/llm_trace.rs"] or ""
    ).replace("llm_call_ref=LLM#", "")
    findings = scan_texts(missing_ref)
    assert any("llm_call_ref=LLM#" in item for item in findings), findings

    missing_raw = dict(good)
    missing_raw["crates/clawcli/src/commands/llm_trace.rs"] = (
        missing_raw["crates/clawcli/src/commands/llm_trace.rs"] or ""
    ).replace("raw_response", "")
    findings = scan_texts(missing_raw)
    assert any("raw_response" in item for item in findings), findings

    missing_flow = dict(good)
    missing_flow["crates/clawcli/src/commands/llm_trace.rs"] = (
        missing_flow["crates/clawcli/src/commands/llm_trace.rs"] or ""
    ).replace("code_entrypoint", "")
    findings = scan_texts(missing_flow)
    assert any("code_entrypoint" in item for item in findings), findings

    missing_gate = dict(good)
    missing_gate["scripts/nl_tests/run_agent_parity_gate.sh"] = "agent parity"
    findings = scan_texts(missing_gate)
    assert any("clawcli_llm_trace_contracts" in item for item in findings), findings

    print("CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
