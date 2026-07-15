#!/usr/bin/env python3
"""Validate clawcli models readiness machine contracts."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

READINESS_FIELDS = (
    "model_readiness_summary",
    "schema_version",
    "selected_provider",
    "selected_model",
    "selected_entry_status",
    "entry_count",
    "matched_entry_count",
    "credential_state",
    "ready",
    "text_generation",
    "image_input",
    "image_understanding",
    "image_generation",
    "audio_input",
    "audio_transcription",
    "audio_generation",
    "video_input",
    "video_generation",
    "music_generation",
    "async_required",
    "dry_run",
)

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawcli/src/commands/models.rs": (
        "pub(crate) fn run_models_readiness",
        "get_v1_json(base_url, key, \"/models/catalog\", \"models_catalog\")",
        "pub(super) fn model_readiness_json",
        "pub(super) fn model_readiness_text_lines",
        "selected_entry_status",
        "matched_entry_count",
        "credential_state",
        "ready",
        "supports_text",
        "supports_image_input",
        "supports_image_understanding",
        "supports_image_generation",
        "supports_audio_input",
        "supports_audio_transcription",
        "supports_audio_generation",
        "supports_video_input",
        "supports_video_generation",
        "supports_music_generation",
        "async_required",
        "dry_run_supported",
        *READINESS_FIELDS,
    ),
    "crates/clawcli/src/commands.rs": (
        "run_models_readiness",
        "model_readiness_json",
        "model_readiness_text_lines",
    ),
    "crates/clawcli/src/commands/llm_trace.rs": (
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
        "text_generation",
        "image_understanding",
        "video_generation",
        "music_generation",
        "dry_run",
    ),
    "crates/clawcli/src/main.rs": (
        "ModelsCommand::Readiness",
        "commands::run_models_readiness",
        "models",
        "readiness",
    ),
    "crates/clawcli/src/commands_models_tests.rs": (
        "models_readiness_text_and_json_use_selected_catalog_entry",
        "models_readiness_marks_missing_selected_entry_not_ready",
        "model_readiness_summary",
        "selected_entry_status=found",
        "selected_entry_status=missing",
        "matched_entry_count=1",
        "matched_entry_count=0",
        "credential_state=configured_inline",
        "credential_state=null",
        "ready=1",
        "ready=0",
        "image_understanding=1",
        "audio_transcription=1",
        "video_generation=1",
        "music_generation=1",
        "dry_run=1",
    ),
    "crates/clawcli/src/commands_llm_trace_tests.rs": (
        "llm_trace_text_lines_project_missing_model_readiness",
        "llm_trace_model_readiness:",
        "trace_ref=model_catalog_trace.readiness",
        "selected_entry_status=found",
        "selected_entry_status=missing",
        "credential_state=configured_env",
        "credential_state=null",
        "ready=true",
        "ready=false",
    ),
    "crates/clawd/src/http/ui_routes/task_debug_trace.rs": (
        "build_model_catalog_trace_for_debug",
        "build_model_readiness_trace_for_debug",
        '"readiness": build_model_readiness_trace_for_debug(&catalog)',
        "task_debug_model_catalog_trace_projects_secret_free_capabilities",
        "task_debug_model_catalog_trace_marks_missing_selected_model_not_ready",
        "selected_entry_status",
        "matched_entry_count",
        "credential_state",
        "ready",
        "text_generation",
        "image_input",
        "image_understanding",
        "image_generation",
        "audio_input",
        "audio_transcription",
        "audio_generation",
        "video_input",
        "video_generation",
        "music_generation",
        "async_required",
        "dry_run",
    ),
    "UI/src/lib/task-llm-trace.ts": (
        "modelCatalogTraceMachineTokens",
        "MODEL_CATALOG_READINESS_SCALAR_FIELDS",
        "MODEL_CATALOG_READINESS_BOOL_FIELDS",
        "modelCatalogReadinessTokens",
        "readiness.${field}",
        "selected_entry_status",
        "matched_entry_count",
        "credential_state",
        "ready",
        "text_generation",
        "image_understanding",
        "music_generation",
        "dry_run",
    ),
    "UI/src/lib/task-llm-trace.test.ts": (
        "builds model catalog trace machine tokens",
        "readiness",
        "readiness.selected_entry_status=found",
        "readiness.matched_entry_count=1",
        "readiness.credential_state=configured_env",
        "readiness.ready=true",
        "readiness.image_understanding=true",
        "readiness.video_generation=true",
        "readiness.music_generation=true",
        "readiness.dry_run=true",
    ),
    "README.md": (
        "clawcli models readiness",
        "clawcli llm-trace",
        "model_readiness_summary",
        "model_catalog_trace.readiness",
        "clawcli_models_readiness_contracts.txt",
        "clawcli_models_readiness_contracts=1",
        "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0",
    ),
    "README.zh-CN.md": (
        "clawcli models readiness",
        "clawcli llm-trace",
        "model_readiness_summary",
        "model_catalog_trace.readiness",
        "clawcli_models_readiness_contracts.txt",
        "clawcli_models_readiness_contracts=1",
        "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/README.md": (
        "clawcli_models_readiness_contracts.txt",
        "clawcli_models_readiness_contracts=1",
        "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0",
        "model_readiness_summary",
        "model_catalog_trace.readiness",
        "clawcli llm-trace",
        "selected_entry_status",
    ),
    "AGENTS.md": (
        "scripts/check_clawcli_models_readiness_contracts.py",
        "clawcli_models_readiness_contracts.txt",
        "clawcli_models_readiness_contracts=1",
        "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0",
        "model_readiness_summary",
        "model_catalog_trace.readiness",
        "clawcli llm-trace",
    ),
    "scripts/nl_tests/run_agent_parity_gate.sh": (
        "AGENT_PARITY_GATE_STEP clawcli_models_readiness_contracts",
        "check_clawcli_models_readiness_contracts.py",
        "clawcli_models_readiness_contracts.txt",
        "clawcli_models_readiness_contracts=1",
    ),
    "scripts/nl_tests/check_suite_artifact_contract.py": (
        "agent_parity_gate/clawcli_models_readiness_contracts.txt",
        '"clawcli_models_readiness_contracts": "1"',
        "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0",
    ),
    "scripts/chinese_model_catalog_gate_checks.py": (
        "AGENT_PARITY_GATE_STEP clawcli_models_readiness_contracts",
        "check_clawcli_models_readiness_contracts.py",
        "clawcli_models_readiness_contracts.txt",
        "clawcli_models_readiness_contracts=1",
        "CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0",
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

    models_text = texts.get("crates/clawcli/src/commands/models.rs") or ""
    for token in READINESS_FIELDS:
        if token not in models_text:
            findings.append(f"models_readiness_contract_token_missing:{token}")
    if "matches!(credential_state.as_str(), \"missing\" | \"null\" | \"\")" not in models_text:
        findings.append("models_readiness_ready_gate_missing_credential_state")
    if models_text.count("bool_value(entry") < 12:
        findings.append("models_readiness_capability_projection_too_weak")

    llm_trace = texts.get("crates/clawcli/src/commands/llm_trace.rs") or ""
    for token in (
        "llm_trace_model_readiness_line",
        "llm_trace_model_readiness:",
        "trace_ref=model_catalog_trace.readiness",
        "selected_entry_status",
        "matched_entry_count",
        "credential_state",
        "ready",
        "dry_run",
    ):
        if token not in llm_trace:
            findings.append(f"missing_llm_trace_model_readiness_token:{token}")

    tests = texts.get("crates/clawcli/src/commands_models_tests.rs") or ""
    for token in (
        "model_readiness_summary",
        "ready=1",
        "ready=0",
        "selected_entry_status=missing",
        "credential_state=null",
        "music_generation=1",
    ):
        if token not in tests:
            findings.append(f"missing_models_readiness_test_token:{token}")

    llm_trace_tests = texts.get("crates/clawcli/src/commands_llm_trace_tests.rs") or ""
    for token in (
        "llm_trace_text_lines_project_missing_model_readiness",
        "llm_trace_model_readiness:",
        "selected_entry_status=missing",
        "credential_state=null",
        "ready=false",
    ):
        if token not in llm_trace_tests:
            findings.append(f"missing_llm_trace_model_readiness_test_token:{token}")

    task_debug = texts.get("crates/clawd/src/http/ui_routes/task_debug_trace.rs") or ""
    for token in (
        '"readiness": build_model_readiness_trace_for_debug(&catalog)',
        "task_debug_model_catalog_trace_marks_missing_selected_model_not_ready",
        "selected_entry_status",
        "matched_entry_count",
        "credential_state",
        "ready",
        "music_generation",
    ):
        if token not in task_debug:
            findings.append(f"missing_task_debug_model_readiness_token:{token}")

    ui_trace = texts.get("UI/src/lib/task-llm-trace.ts") or ""
    for token in (
        "modelCatalogReadinessTokens",
        "MODEL_CATALOG_READINESS_SCALAR_FIELDS",
        "MODEL_CATALOG_READINESS_BOOL_FIELDS",
        "readiness.${field}",
    ):
        if token not in ui_trace:
            findings.append(f"missing_ui_model_readiness_token:{token}")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    texts["crates/clawcli/src/commands/models.rs"] += "\n" + "\n".join(
        [
            *READINESS_FIELDS,
            *["bool_value(entry" for _ in range(12)],
            'matches!(credential_state.as_str(), "missing" | "null" | "")',
        ]
    )
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    good_findings = scan_texts(good)
    assert not good_findings, good_findings

    missing_summary = dict(good)
    missing_summary["crates/clawcli/src/commands/models.rs"] = (
        missing_summary["crates/clawcli/src/commands/models.rs"] or ""
    ).replace("model_readiness_summary", "")
    findings = scan_texts(missing_summary)
    assert any("model_readiness_summary" in item for item in findings), findings

    missing_ready_gate = dict(good)
    missing_ready_gate["crates/clawcli/src/commands/models.rs"] = (
        missing_ready_gate["crates/clawcli/src/commands/models.rs"] or ""
    ).replace('matches!(credential_state.as_str(), "missing" | "null" | "")', "")
    findings = scan_texts(missing_ready_gate)
    assert any("ready_gate" in item for item in findings), findings

    missing_llm_trace = dict(good)
    missing_llm_trace["crates/clawcli/src/commands/llm_trace.rs"] = (
        missing_llm_trace["crates/clawcli/src/commands/llm_trace.rs"] or ""
    ).replace("llm_trace_model_readiness:", "")
    findings = scan_texts(missing_llm_trace)
    assert any("missing_llm_trace_model_readiness_token" in item for item in findings), findings

    missing_gate = dict(good)
    missing_gate["scripts/nl_tests/run_agent_parity_gate.sh"] = "agent parity"
    findings = scan_texts(missing_gate)
    assert any("clawcli_models_readiness_contracts" in item for item in findings), findings

    print("CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
