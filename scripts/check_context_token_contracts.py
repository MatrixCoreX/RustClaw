#!/usr/bin/env python3
"""Guard provider-aware token, prompt-section, and skill-disclosure contracts."""

from __future__ import annotations

import argparse
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/token_estimator.rs": (
        "enum TokenEstimatorKind",
        "MiniMaxM2",
        "OpenAiCompatible",
        "AnthropicCompatible",
        "GenericUnicode",
        "estimate_provider_tokens",
        "provider_tokens",
        "safety_tokens",
        "byte_count",
        "char_count",
    ),
    "crates/clawd/src/providers/routing.rs": (
        "estimate_provider_tokens",
        "estimated_prompt_tokens",
        "prompt_token_estimator",
        "prompt_byte_count",
        "prompt_char_count",
        "prompt_estimate",
        ".safety_tokens",
    ),
    "crates/clawd/src/task_context_builder.rs": (
        "context_slot_metadata",
        '"token_estimate"',
        '"token_safety_estimate"',
        '"token_estimator"',
        '"cacheability"',
        '"provenance"',
        '"reason": "not_included"',
    ),
    "crates/clawd/src/prompt_budget.rs": (
        "struct PromptSection",
        "prompt_section_budget_report",
        "publish_prompt_section_budget_report",
        '"prompt_section_budget"',
        '"omission_reason"',
        '"cacheability"',
        '"provenance"',
    ),
    "crates/clawd/src/agent_engine/planning.rs": (
        "publish_prompt_section_budget_report",
        '"native_protocol_template"',
        '"tool_spec"',
        '"skill_quick_index"',
        '"selected_skill_playbooks"',
        '"turn_context"',
        '"stable_prefix"',
        '"dynamic_turn"',
    ),
    "crates/clawd/src/agent_engine/planner_skill_context.rs": (
        '"compact_index"',
        '"scoped_playbooks"',
        "candidate_skill_scope_from_loop_state",
        "MAX_SCOPED_SKILL_PLAYBOOKS",
        "quick_index_text",
        "playbook_text",
    ),
    "crates/clawd/src/token_estimator_tests.rs": (
        "minimax_estimate_uses_documented_cjk_ratio_with_conservative_admission",
        "mixed_language_estimates_are_provider_specific_and_never_zero",
        "unicode_estimator_does_not_split_or_underflow_multibyte_text",
    ),
    "crates/clawd/src/prompt_budget_tests.rs": (
        "prompt_section_report_records_tokens_cacheability_provenance_and_omission",
    ),
}

FORBIDDEN_TOKENS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/task_context_builder.rs": (
        '"token_estimate": (char_estimate / 4)',
    ),
    "crates/clawd/src/providers/routing.rs": (
        "fn estimated_prompt_tokens(prompt_bytes",
        "prompt_bytes.saturating_add(3) / 4",
    ),
    "crates/clawd/src/llm_gateway.rs": (
        "route_providers(providers, prompt.len()",
    ),
    "crates/clawd/src/llm_gateway_model_turn.rs": (
        "route_providers(task_providers, prompt.len()",
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
    missing["crates/clawd/src/token_estimator.rs"] = "enum TokenEstimatorKind"
    assert any("estimate_provider_tokens" in item for item in scan_texts(missing))

    regressed = dict(good)
    regressed["crates/clawd/src/task_context_builder.rs"] += (
        '\n"token_estimate": (char_estimate / 4)'
    )
    assert any("forbidden_token" in item for item in scan_texts(regressed))
    print("CONTEXT_TOKEN_CONTRACT_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CONTEXT_TOKEN_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print("CONTEXT_TOKEN_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
