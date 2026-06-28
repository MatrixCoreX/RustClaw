#!/usr/bin/env python3
"""Guard answer-verifier modules against semantic routing expansion.

The answer verifier may call the verifier prompt, check structured contracts,
compare candidate answers with observed machine evidence, and preserve a small
set of historical compatibility reads. It must not grow into a skill/action
router, runtime localized reply renderer, or skill text/error_text protocol
parser.
"""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates/clawd/src"


@dataclasses.dataclass(frozen=True)
class BoundaryRule:
    name: str
    pattern: re.Pattern[str]
    allowed_files: frozenset[str]


RULES: tuple[BoundaryRule, ...] = (
    BoundaryRule(
        name="verifier_llm_entry",
        pattern=re.compile(r"\brun_with_fallback_with_prompt_source\s*\("),
        allowed_files=frozenset({"crates/clawd/src/answer_verifier_runtime.rs"}),
    ),
    BoundaryRule(
        name="language_policy",
        pattern=re.compile(r"\bcrate::language_policy::"),
        allowed_files=frozenset({"crates/clawd/src/answer_verifier_runtime.rs"}),
    ),
    BoundaryRule(
        name="route_reason_marker_parse",
        pattern=re.compile(r"\broute_result\.route_reason\b"),
        allowed_files=frozenset(
            {
                "crates/clawd/src/answer_verifier.rs",
                "crates/clawd/src/answer_verifier_runtime.rs",
            }
        ),
    ),
    BoundaryRule(
        name="observed_text_wrapper_compat",
        pattern=re.compile(r"\.get\(\"text\"\)"),
        allowed_files=frozenset(
            {
                "crates/clawd/src/answer_verifier_matrix.rs",
                "crates/clawd/src/answer_verifier_scalar.rs",
            }
        ),
    ),
    BoundaryRule(
        name="skill_visible_error_text_protocol",
        pattern=re.compile(r"\berror_text\b"),
        allowed_files=frozenset(),
    ),
    BoundaryRule(
        name="skill_visible_text_normalization",
        pattern=re.compile(r"\bnormalize_skill_error_for_user\b"),
        allowed_files=frozenset(),
    ),
    BoundaryRule(
        name="localized_reply_rendering",
        pattern=re.compile(
            r"\b(crate::i18n::|observed_t(?:_with_vars)?\s*\(|t_with_vars\s*\()"
        ),
        allowed_files=frozenset(),
    ),
    BoundaryRule(
        name="planner_action_emission",
        pattern=re.compile(
            r"\b(call_capability|call_skill|call_tool|CallCapability|CallSkill|CallTool)\b"
        ),
        allowed_files=frozenset(),
    ),
    BoundaryRule(
        name="runtime_dispatch_from_verifier",
        pattern=re.compile(
            r"\b(run_skill_with_runner|execution_adapters::run_skill|CapabilityResolver|PlanVerifier)\b"
        ),
        allowed_files=frozenset(),
    ),
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    return rel_path.endswith(("_tests.rs", "tests.rs")) or any(
        part == "tests" or part.endswith("_tests") for part in Path(rel_path).parts
    )


def answer_verifier_files() -> list[Path]:
    return sorted(
        path
        for path in SRC_ROOT.glob("answer_verifier*.rs")
        if path.is_file() and not is_test_path(path)
    )


def scan_repo() -> list[str]:
    findings: list[str] = []
    for path in answer_verifier_files():
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
    print(f"answer_verifier_files={len(answer_verifier_files())}")
    for rule in RULES:
        print(f"{rule.name} allowed_files={len(rule.allowed_files)}")


def run_self_test() -> int:
    llm_rule = next(rule for rule in RULES if rule.name == "verifier_llm_entry")
    assert llm_rule.pattern.search("run_with_fallback_with_prompt_source(")
    assert "crates/clawd/src/answer_verifier_runtime.rs" in llm_rule.allowed_files
    action_rule = next(rule for rule in RULES if rule.name == "planner_action_emission")
    assert action_rule.pattern.search('"call_capability"')
    assert "crates/clawd/src/answer_verifier_runtime.rs" not in action_rule.allowed_files
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
        print(f"ANSWER_VERIFIER_BOUNDARY_CHECK findings={len(findings)}")
        for finding in findings:
            print(f"  - {finding}")
        return 1
    print(
        "ANSWER_VERIFIER_BOUNDARY_CHECK ok "
        f"files={len(answer_verifier_files())} rules={len(RULES)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
