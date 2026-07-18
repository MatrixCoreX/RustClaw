#!/usr/bin/env python3
"""Guard final agent-loop guard scopes from legacy rollout modes.

The answer-verifier required-evidence boundary and registry idempotency boundary
are final always-on machine guards. Production runtime must not reintroduce
legacy selected-route scope branches or config values that disable these guards.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SUPPORT_RS = ROOT / "crates/clawd/src/agent_engine/support.rs"
AGENT_GUARD_TOML = ROOT / "configs/agent_guard.toml"
PRODUCTION_VERIFY_FILES = (
    ROOT / "crates/claw-core/src/config.rs",
    ROOT / "crates/claw-core/src/config/defaults.rs",
    ROOT / "crates/clawd/src/agent_engine/prepare_round.rs",
    ROOT / "crates/clawd/src/bootstrap/config_loaders.rs",
    ROOT / "crates/clawd/src/runtime/state.rs",
    ROOT / "crates/clawd/src/runtime/types.rs",
)

FINAL_SCOPE_KEYS = (
    "answer_verifier_enforce_required_scope",
    "registry_idempotency_guard_scope",
)

FORBIDDEN_CONFIG_SECTIONS = (
    "agent.loop_guard.crypto",
    "agent.loop_guard.fs_search",
    "agent.loop_guard.media",
    "agent.loop_guard.dedup",
    "agent.dynamic_rules",
    "agent.messages",
    "agent.trace_messages",
)

FORBIDDEN_SUPPORT_TOKENS = (
    "SelectedAgentLoop",
    "selected_agent_loop_route",
    '"selected_agent_loop"',
    "agent_decides_eligible_migration_class(route)",
    "structured_evidence_required_for_selected_contracts",
)

FORBIDDEN_PRODUCTION_VERIFY_TOKENS = (
    "verify_enforce_enabled",
    "default_command_intent_verify_enforce_enabled",
    "VerifyMode::ObserveOnly",
)


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def support_findings(raw: str, rel_path: str) -> list[str]:
    findings: list[str] = []
    for line_no, line in enumerate(raw.splitlines(), start=1):
        for token in FORBIDDEN_SUPPORT_TOKENS:
            if token in line:
                findings.append(f"{rel_path}:{line_no}: legacy_guard_scope_runtime_token:{token}")
        if "unwrap_or(AnswerVerifierRequiredEvidenceScope::Off)" in line:
            findings.append(f"{rel_path}:{line_no}: answer_verifier_scope_fallback_off")
        if "unwrap_or(RegistryIdempotencyGuardScope::Off)" in line:
            findings.append(f"{rel_path}:{line_no}: registry_idempotency_scope_fallback_off")
    if "return AnswerVerifierRequiredEvidenceScope::All;" not in raw:
        findings.append(f"{rel_path}: missing_answer_verifier_missing_config_defaults_all")
    if "return RegistryIdempotencyGuardScope::All;" not in raw:
        findings.append(f"{rel_path}: missing_registry_idempotency_missing_config_defaults_all")
    return findings


def config_findings(raw: str, rel_path: str) -> list[str]:
    findings: list[str] = []
    try:
        tomllib.loads(raw)
    except tomllib.TOMLDecodeError as exc:
        findings.append(f"{rel_path}: invalid_toml:{exc}")
        return findings
    parsed_sections = set(re.findall(r"(?m)^\s*\[([^\]]+)\]\s*$", raw))
    for section in FORBIDDEN_CONFIG_SECTIONS:
        if section in parsed_sections:
            findings.append(f"{rel_path}: removed_legacy_section_present:{section}")
    for key in FINAL_SCOPE_KEYS:
        assignments = re.findall(rf"(?m)^\s*{re.escape(key)}\s*=\s*\"([^\"]*)\"", raw)
        if not assignments:
            findings.append(f"{rel_path}: missing_final_scope_key:{key}")
            continue
        for value in assignments:
            if value != "all":
                findings.append(f"{rel_path}: final_scope_key_not_all:{key}={value!r}")
    for line_no, line in enumerate(raw.splitlines(), start=1):
        if "selected_agent_loop" in line:
            findings.append(f"{rel_path}:{line_no}: config_mentions_legacy_selected_scope")
        if "structured_evidence_required_for_selected_contracts" in line:
            findings.append(
                f"{rel_path}:{line_no}: config_mentions_legacy_selected_contract_gate"
            )
        if "可回滚" in line or "rollback" in line.lower():
            findings.append(f"{rel_path}:{line_no}: config_mentions_guard_scope_rollback")
    return findings


def production_verify_findings(raw: str, rel_path: str) -> list[str]:
    findings: list[str] = []
    for line_no, line in enumerate(raw.splitlines(), start=1):
        for token in FORBIDDEN_PRODUCTION_VERIFY_TOKENS:
            if token in line:
                findings.append(
                    f"{rel_path}:{line_no}: legacy_production_verify_mode_token:{token}"
                )
    return findings


def scan_repo() -> list[str]:
    findings: list[str] = []
    findings.extend(support_findings(SUPPORT_RS.read_text(encoding="utf-8"), rel(SUPPORT_RS)))
    findings.extend(config_findings(AGENT_GUARD_TOML.read_text(encoding="utf-8"), rel(AGENT_GUARD_TOML)))
    for path in PRODUCTION_VERIFY_FILES:
        findings.extend(
            production_verify_findings(path.read_text(encoding="utf-8"), rel(path))
        )
    return findings


def run_self_test() -> int:
    good_support = """
fn parse_answer_verifier_required_evidence_scope() {
    return AnswerVerifierRequiredEvidenceScope::All;
}
fn parse_registry_idempotency_guard_scope() {
    return RegistryIdempotencyGuardScope::All;
}
"""
    assert not support_findings(good_support, "support.rs")

    bad_support = """
fn selected_agent_loop_route() {}
let x = AnswerVerifierRequiredEvidenceScope::SelectedAgentLoop;
let y = "selected_agent_loop";
let z = parse().unwrap_or(RegistryIdempotencyGuardScope::Off);
let old = structured_evidence_required_for_selected_contracts;
"""
    bad_support_findings = support_findings(bad_support, "support.rs")
    assert any("legacy_guard_scope_runtime_token" in item for item in bad_support_findings)
    assert any("registry_idempotency_scope_fallback_off" in item for item in bad_support_findings)
    assert any(
        "missing_answer_verifier_missing_config_defaults_all" in item
        for item in bad_support_findings
    )

    good_config = """
[agent.loop_guard]
answer_verifier_enforce_required_scope = "all"
registry_idempotency_guard_scope = "all"
"""
    assert not config_findings(good_config, "configs/agent_guard.toml")

    bad_config = """
[agent.loop_guard]
# rollback to selected_agent_loop
answer_verifier_enforce_required_scope = "off"
registry_idempotency_guard_scope = "selected_agent_loop"
structured_evidence_required_for_selected_contracts = true
[agent.dynamic_rules]
legacy = "text"
"""
    bad_config_findings = config_findings(bad_config, "configs/agent_guard.toml")
    assert any("final_scope_key_not_all" in item for item in bad_config_findings)
    assert any("config_mentions_legacy_selected_scope" in item for item in bad_config_findings)
    assert any("config_mentions_guard_scope_rollback" in item for item in bad_config_findings)
    assert any(
        "config_mentions_legacy_selected_contract_gate" in item
        for item in bad_config_findings
    )
    assert any(
        "removed_legacy_section_present:agent.dynamic_rules" in item
        for item in bad_config_findings
    )

    assert not production_verify_findings(
        "fn production_verify_mode() -> VerifyMode { VerifyMode::Enforce }",
        "prepare_round.rs",
    )
    bad_verify_findings = production_verify_findings(
        "if config.verify_enforce_enabled { VerifyMode::Enforce } else { VerifyMode::ObserveOnly }",
        "prepare_round.rs",
    )
    assert len(bad_verify_findings) == 2
    assert all(
        "legacy_production_verify_mode_token" in item for item in bad_verify_findings
    )

    print("AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_repo()
    print(f"AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings={len(findings)}")
    for finding in findings:
        print(f"  - {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
