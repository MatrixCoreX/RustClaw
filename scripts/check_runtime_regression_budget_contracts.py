#!/usr/bin/env python3
"""Guard token/call/wall-time regression budget wiring."""

from __future__ import annotations

import argparse
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS: dict[str, tuple[str, ...]] = {
    "scripts/nl_tests/summarize_rollout_metrics.py": (
        '"cached_input_tokens"',
        '"uncached_input_tokens"',
        '"cache_read_ratio"',
        '"avg_input_tokens_per_turn"',
        '"avg_uncached_input_tokens_per_turn"',
        '"usage_recording_status"',
        '"wall_time"',
        '"avg_tool_calls_per_turn"',
    ),
    "scripts/inventories/runtime_regression_budgets.toml": (
        "schema_version = 1",
        "[profiles.focused]",
        "[profiles.continuous_coding]",
        "max_avg_prompt_tokens",
        "max_avg_uncached_input_tokens",
        "min_cache_read_ratio",
        "max_avg_llm_calls",
        "max_avg_tool_calls",
        "max_avg_wall_time_ms",
    ),
    "scripts/nl_tests/check_runtime_regression_budget.py": (
        "RUNTIME_REGRESSION_BUDGET_SELF_TEST ok",
        '"max_avg_prompt_tokens"',
        '"max_avg_uncached_input_tokens"',
        '"min_cache_read_ratio"',
        '"max_avg_llm_calls"',
        '"max_avg_tool_calls"',
        '"max_avg_wall_time_ms"',
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
    return findings


def read_repo_texts() -> dict[str, str | None]:
    out: dict[str, str | None] = {}
    for rel_path in REQUIRED_TOKENS:
        try:
            out[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            out[rel_path] = None
    return out


def run_self_test() -> None:
    good = {
        rel_path: "\n".join(tokens)
        for rel_path, tokens in REQUIRED_TOKENS.items()
    }
    assert not scan_texts(good)
    regressed = dict(good)
    regressed["scripts/inventories/runtime_regression_budgets.toml"] = (
        "[profiles.focused]"
    )
    assert any("max_avg_prompt_tokens" in item for item in scan_texts(regressed))
    print("RUNTIME_REGRESSION_BUDGET_CONTRACT_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"RUNTIME_REGRESSION_BUDGET_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print("RUNTIME_REGRESSION_BUDGET_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
