#!/usr/bin/env python3
"""Validate clawcli models catalog machine contracts."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS_BY_PATH: dict[str, tuple[str, ...]] = {
    "crates/clawcli/src/commands/models.rs": (
        "pub(crate) fn run_models_catalog",
        "get_v1_json(base_url, key, \"/models/catalog\", \"models_catalog\")",
        "pub(super) fn filter_catalog_response",
        "pub(super) fn model_catalog_text_lines",
        "fn model_catalog_summary_line",
        "model_catalog_summary",
        "schema_version",
        "selected_provider",
        "selected_model",
        "entry_count",
        "model_catalog_entry",
        "provider",
        "model",
        "active_text_provider",
        "api_style",
        "base_url_kind",
        "credential_state",
        "context_window_tokens",
        "input_modalities",
        "output_modalities",
        "supports_text",
        "supports_image_input",
        "supports_video_input",
        "supports_audio_input",
        "supports_image_understanding",
        "supports_audio_transcription",
        "supports_image_generation",
        "supports_audio_generation",
        "supports_video_generation",
        "supports_music_generation",
        "async_required",
        "dry_run_supported",
    ),
    "crates/clawcli/src/main.rs": (
        "Command::Models",
        "commands::run_models_catalog",
        "ModelsCommand::Catalog",
        "models",
        "catalog",
        "provider",
    ),
    "crates/clawcli/src/commands_models_tests.rs": (
        "models_catalog_filter_and_text_lines_use_machine_tokens",
        "model_catalog_summary",
        "schema_version=1",
        "selected_provider=minimax",
        "selected_model=MiniMax-M3",
        "entry_count=1",
        "model_catalog_entry provider=minimax model=MiniMax-M3",
        "credential_state=configured_inline",
        "context_window_tokens=1000000",
        "input_modalities=text,image,video",
        "output_modalities=text",
        "video_generation=1",
        "music_generation=1",
        "async_required=1",
        "dry_run=1",
    ),
    "UI/src/lib/model-config.ts": (
        "credential_state",
        "active_text_provider",
        "input_modalities",
        "output_modalities",
        "supports_image_generation",
        "supports_video_generation",
        "supports_music_generation",
        "context_window_tokens",
    ),
    "UI/src/lib/model-config.test.ts": (
        "credential_state",
        "active_text_provider",
        "MiniMax-M3",
    ),
    "README.md": (
        "clawcli models catalog",
        "model_catalog_summary",
        "model_catalog_entry",
        "credential_state",
        "clawcli_models_catalog_contracts.txt",
        "clawcli_models_catalog_contracts=1",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0",
    ),
    "README.zh-CN.md": (
        "clawcli models catalog",
        "model_catalog_summary",
        "model_catalog_entry",
        "credential_state",
        "clawcli_models_catalog_contracts.txt",
        "clawcli_models_catalog_contracts=1",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/README.md": (
        "clawcli_models_catalog_contracts.txt",
        "clawcli_models_catalog_contracts=1",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0",
        "model_catalog_summary",
        "credential_state",
    ),
    "AGENTS.md": (
        "scripts/check_clawcli_models_catalog_contracts.py",
        "clawcli_models_catalog_contracts.txt",
        "clawcli_models_catalog_contracts=1",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0",
    ),
    "scripts/nl_tests/run_agent_parity_gate.sh": (
        "AGENT_PARITY_GATE_STEP clawcli_models_catalog_contracts",
        "check_clawcli_models_catalog_contracts.py",
        "clawcli_models_catalog_contracts.txt",
        "clawcli_models_catalog_contracts=1",
    ),
    "scripts/nl_tests/check_suite_artifact_contract.py": (
        "agent_parity_gate/clawcli_models_catalog_contracts.txt",
        '"clawcli_models_catalog_contracts": "1"',
        "CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok",
        "CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0",
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
    for token in (
        "model_catalog_summary",
        "schema_version",
        "selected_provider",
        "selected_model",
        "entry_count",
        "credential_state",
        "input_modalities",
        "output_modalities",
        "dry_run_supported",
    ):
        if token not in models_text:
            findings.append(f"models_catalog_contract_token_missing:{token}")
    if models_text.count("bool_token") < 12:
        findings.append("models_catalog_capability_projection_too_weak")

    tests = texts.get("crates/clawcli/src/commands_models_tests.rs") or ""
    for token in (
        "model_catalog_summary",
        "entry_count=1",
        "credential_state=configured_inline",
        "video_generation=1",
        "dry_run=1",
    ):
        if token not in tests:
            findings.append(f"missing_models_catalog_test_token:{token}")

    return findings


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens) for rel_path, tokens in REQUIRED_TOKENS_BY_PATH.items()
    }
    texts["crates/clawcli/src/commands/models.rs"] += "\n" + "\n".join(
        [
            *["bool_token" for _ in range(12)],
            "model_catalog_summary",
            "schema_version",
            "selected_provider",
            "selected_model",
            "entry_count",
            "credential_state",
            "input_modalities",
            "output_modalities",
            "dry_run_supported",
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
    ).replace("model_catalog_summary", "")
    findings = scan_texts(missing_summary)
    assert any("model_catalog_summary" in item for item in findings), findings

    missing_credential = dict(good)
    missing_credential["crates/clawcli/src/commands/models.rs"] = (
        missing_credential["crates/clawcli/src/commands/models.rs"] or ""
    ).replace("credential_state", "")
    findings = scan_texts(missing_credential)
    assert any("credential_state" in item for item in findings), findings

    missing_capability = dict(good)
    missing_capability["crates/clawcli/src/commands/models.rs"] = (
        missing_capability["crates/clawcli/src/commands/models.rs"] or ""
    ).replace("supports_video_generation", "")
    findings = scan_texts(missing_capability)
    assert any("supports_video_generation" in item for item in findings), findings

    missing_gate = dict(good)
    missing_gate["scripts/nl_tests/run_agent_parity_gate.sh"] = "agent parity"
    findings = scan_texts(missing_gate)
    assert any("clawcli_models_catalog_contracts" in item for item in findings), findings

    print("CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings={len(findings)}")
        for item in findings:
            print(item)
        return 1
    print("CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
