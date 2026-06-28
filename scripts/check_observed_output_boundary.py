#!/usr/bin/env python3
"""Guard observed-output modules against new language/fallback expansion.

Observed-output code may project machine evidence and preserve existing
compatibility paths, but it should not keep growing new user-language rendering,
LLM fallback, route-reason parsing, or skill text/error_text protocol reads.
This guard freezes the current exception surface by file.
"""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OBSERVED_ROOT = ROOT / "crates/clawd/src/agent_engine"


@dataclasses.dataclass(frozen=True)
class BoundaryRule:
    name: str
    pattern: re.Pattern[str]
    allowed_files: frozenset[str]


RULES: tuple[BoundaryRule, ...] = (
    BoundaryRule(
        name="language_policy",
        pattern=re.compile(r"\bcrate::language_policy::"),
        allowed_files=frozenset(
            {
                "crates/clawd/src/agent_engine/observed_output.rs",
                "crates/clawd/src/agent_engine/observed_output_route_policy.rs",
            }
        ),
    ),
    BoundaryRule(
        name="llm_or_fallback",
        pattern=re.compile(r"\b(run_with_fallback_with_prompt_source|crate::fallback::)"),
        allowed_files=frozenset({"crates/clawd/src/agent_engine/observed_output.rs"}),
    ),
    BoundaryRule(
        name="localized_observed_template",
        pattern=re.compile(r"\bobserved_t(?:_with_vars)?\s*\("),
        allowed_files=frozenset(
            {
                "crates/clawd/src/agent_engine/observed_output.rs",
                "crates/clawd/src/agent_engine/observed_output_archive.rs",
                "crates/clawd/src/agent_engine/observed_output_direct_scalar.rs",
                "crates/clawd/src/agent_engine/observed_output_fs_search.rs",
                "crates/clawd/src/agent_engine/observed_output_listing.rs",
                "crates/clawd/src/agent_engine/observed_output_path_facts.rs",
                "crates/clawd/src/agent_engine/observed_output_process_service.rs",
                "crates/clawd/src/agent_engine/observed_output_read_range.rs",
                "crates/clawd/src/agent_engine/observed_output_sqlite.rs",
                "crates/clawd/src/agent_engine/observed_output_structured_fields.rs",
                "crates/clawd/src/agent_engine/observed_output_system_inventory.rs",
            }
        ),
    ),
    BoundaryRule(
        name="skill_user_text_normalization",
        pattern=re.compile(r"\bnormalize_skill_error_for_user\b"),
        allowed_files=frozenset(
            {"crates/clawd/src/agent_engine/observed_output_entries.rs"}
        ),
    ),
    BoundaryRule(
        name="skill_error_text_field",
        pattern=re.compile(r"\berror_text\b"),
        allowed_files=frozenset(
            {
                "crates/clawd/src/agent_engine/observed_output_entries.rs",
                "crates/clawd/src/agent_engine/observed_output_structured_fields.rs",
            }
        ),
    ),
    BoundaryRule(
        name="route_reason_marker_parse",
        pattern=re.compile(r"\broute_reason\b"),
        allowed_files=frozenset(
            {
                "crates/clawd/src/agent_engine/observed_output_listing.rs",
                "crates/clawd/src/agent_engine/observed_output_route_policy.rs",
            }
        ),
    ),
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    return rel_path.endswith(("_tests.rs", "tests.rs")) or any(
        part == "tests" or part.endswith("_tests") for part in Path(rel_path).parts
    )


def observed_output_files() -> list[Path]:
    return sorted(
        path
        for path in OBSERVED_ROOT.glob("observed_output*.rs")
        if path.is_file() and not is_test_path(path)
    )


def scan_repo() -> list[str]:
    findings: list[str] = []
    for path in observed_output_files():
        rel_path = rel(path)
        text = path.read_text(encoding="utf-8")
        for line_no, line in enumerate(text.splitlines(), start=1):
            for rule in RULES:
                if not rule.pattern.search(line):
                    continue
                if rel_path in rule.allowed_files:
                    continue
                findings.append(
                    f"{rel_path}:{line_no}: {rule.name}_outside_allowlist: {line.strip()}"
                )
    return findings


def print_summary() -> None:
    print(f"observed_output_files={len(observed_output_files())}")
    for rule in RULES:
        print(f"{rule.name} allowed_files={len(rule.allowed_files)}")


def run_self_test() -> int:
    blocked = "crates/clawd/src/agent_engine/observed_output_new.rs"
    allowed = "crates/clawd/src/agent_engine/observed_output.rs"
    llm_rule = next(rule for rule in RULES if rule.name == "llm_or_fallback")
    assert llm_rule.pattern.search("run_with_fallback_with_prompt_source(")
    assert blocked not in llm_rule.allowed_files
    assert allowed in llm_rule.allowed_files
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--summary", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_repo()
    if args.summary:
        print_summary()
    if findings:
        print(f"OBSERVED_OUTPUT_BOUNDARY_CHECK findings={len(findings)}")
        for finding in findings:
            print(f"  - {finding}")
        return 1
    print(
        "OBSERVED_OUTPUT_BOUNDARY_CHECK ok "
        f"files={len(observed_output_files())} rules={len(RULES)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
