#!/usr/bin/env python3
"""Guard parser-backed repository indexing and structured context retrieval."""

from __future__ import annotations

import argparse
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS: dict[str, tuple[str, ...]] = {
    "Cargo.toml": (
        'proc-macro2 = { version = "1", features = ["span-locations"] }',
        'syn = { version = "2", features = ["full", "visit"] }',
    ),
    "crates/clawd/src/skills/builtin_code_index.rs": (
        "INDEX_RELATIVE_PATH",
        "refresh_index",
        "RustSymbolCollector",
        "syn::parse_file",
        "record_token_stream_references",
        '"find_definitions"',
        '"find_references"',
        '"changed_impact"',
        '"retrieve_context"',
        '"read_capability": "filesystem.read_text_range"',
        "normalize_relative_path",
    ),
    "crates/clawd/src/skills/builtin_code_index_tests.rs": (
        "refresh_is_incremental_and_indexes_rust_symbols_references_and_tests",
        "definitions_and_references_return_machine_range_handles",
        "retrieve_context_uses_structured_symbols_and_bounded_source_ranges",
        "changed_impact_connects_changed_definitions_to_dependent_test_files",
        "workspace_traversal_is_rejected_as_machine_error",
    ),
    "crates/clawd/src/skills/builtin_tests.rs": (
        "code_index_builtin_dispatch_returns_structured_definition_ranges",
    ),
    "crates/clawd/src/runtime/policy.rs": (
        '"skill:code_index"',
    ),
    "configs/skills_registry.toml": (
        'name = "code_index"',
        '{ name = "code.find_definitions"',
        '{ name = "code.find_references"',
        '{ name = "code.changed_impact"',
        '{ name = "code.retrieve_context"',
    ),
    "docker/config/skills_registry.toml": (
        'name = "code_index"',
        '{ name = "code.retrieve_context"',
    ),
    "prompts/layers/generated/skills/code_index.md": (
        "incremental repository intelligence",
        "structured symbol/path relevance",
        "Never put the whole natural-language task",
        "## Multilingual Reinforcement",
    ),
}

FORBIDDEN_TOKENS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/skills/builtin_code_index.rs": (
        "request_text",
        "user_text",
        "Regex::",
        "to_ascii_lowercase().contains(",
    ),
}


def scan_texts(texts: dict[str, str | None]) -> list[str]:
    findings: list[str] = []
    for rel_path, tokens in REQUIRED_TOKENS.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token not in text:
                findings.append(f"missing_token:{rel_path}:{token}")
    for rel_path, tokens in FORBIDDEN_TOKENS.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token in text:
                findings.append(f"forbidden_token:{rel_path}:{token}")
    return findings


def read_repo_texts() -> dict[str, str | None]:
    paths = set(REQUIRED_TOKENS) | set(FORBIDDEN_TOKENS)
    out: dict[str, str | None] = {}
    for rel_path in paths:
        try:
            out[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            out[rel_path] = None
    return out


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens)
        for rel_path, tokens in REQUIRED_TOKENS.items()
    }
    for rel_path in FORBIDDEN_TOKENS:
        texts.setdefault(rel_path, "")
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    assert not scan_texts(good)

    missing = dict(good)
    missing["crates/clawd/src/skills/builtin_code_index.rs"] = "syn::parse_file"
    assert any("retrieve_context" in item for item in scan_texts(missing))

    regressed = dict(good)
    regressed["crates/clawd/src/skills/builtin_code_index.rs"] += "\nrequest_text"
    assert any("forbidden_token" in item for item in scan_texts(regressed))
    print("CODE_INDEX_CONTRACT_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CODE_INDEX_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print("CODE_INDEX_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
