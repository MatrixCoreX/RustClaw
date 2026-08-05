#!/usr/bin/env python3
"""Validate the behavior-neutral memory/context WP0 evidence bundle."""
from __future__ import annotations

import argparse
import json
import tempfile
import tomllib
from pathlib import Path
from typing import Any


REQUIRED_COVERAGE = {
    "attachment",
    "case_sensitive",
    "chat",
    "code_symbol",
    "conflict",
    "cross_scope",
    "deleted",
    "emoji",
    "english",
    "expired",
    "group_channel",
    "hash",
    "japanese",
    "korean",
    "mcp",
    "ocr",
    "path",
    "principal",
    "project",
    "prompt_injection",
    "relative_time",
    "scheduled_task",
    "secret_filter",
    "sensitive_text",
    "short_query",
    "simplified_chinese",
    "stt",
    "subagent",
    "timezone",
    "tool_evidence",
    "traditional_chinese",
    "typo",
    "url",
    "user_correction",
    "web",
}

REMOVED_RUNTIME_MARKERS = {
    "crates/clawd/src/finalize/task_memory.rs": ["tokio::spawn"],
    "crates/clawd/src/memory/api.rs": ["std::fs::write(&config_path"],
    "crates/clawd/src/memory.rs": ["utf8_safe_prefix(&normalized, keep)"],
    "crates/clawd/src/task_context_builder/compaction.rs": [
        "PROVIDER_CONTEXT_COMPACTION_PERCENT: usize = 75",
    ],
}

# These are deliberate compatibility/fallback seams, not unresolved WP0
# defects. Keep them inventoried so a future removal has to shrink the ratchet
# explicitly instead of silently changing upgrade behavior.
REQUIRED_COMPATIBILITY_MARKERS = {
    "crates/clawd/src/memory/embedding.rs": [
        "Box::new(LocalHashEmbeddingProvider)",
    ],
    "crates/clawd/src/memory/indexing.rs": [
        "vector_json       TEXT",
        "LIMIT -1 OFFSET ?1",
    ],
    "crates/clawd/src/memory/service.rs": ["long_term_summary_max_chars"],
    "crates/clawd/src/worker/runtime_support/background_workers.rs": [
        "LIMIT -1 OFFSET ?1",
    ],
}

ADR_DECISIONS = [f"### D{index}:" for index in range(1, 9)]
SECRET_LIKE_MARKERS = ("-----BEGIN PRIVATE KEY-----", "sk-proj-", "xoxb-")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_payloads(
    fixture: dict[str, Any],
    baseline: dict[str, Any],
    inventory: dict[str, Any],
    raw_fixture: str,
) -> list[str]:
    findings: list[str] = []
    if fixture.get("schema_version") != 1 or baseline.get("schema_version") != 1:
        findings.append("unsupported_schema_version")
    if fixture.get("synthetic_only") is not True:
        findings.append("fixture_not_marked_synthetic")
    if fixture.get("contains_production_data") is not False:
        findings.append("fixture_production_data_flag_not_false")
    if baseline.get("fixture_id") != fixture.get("fixture_id"):
        findings.append("baseline_fixture_id_mismatch")
    if any(marker in raw_fixture for marker in SECRET_LIKE_MARKERS):
        findings.append("fixture_contains_secret_like_marker")

    covered = {
        category
        for case in fixture.get("coverage_cases", [])
        for category in case.get("categories", [])
    }
    for category in sorted(REQUIRED_COVERAGE - covered):
        findings.append(f"missing_coverage:{category}")

    retrieval = baseline.get("retrieval", {})
    current_limits = fixture.get("thresholds", {}).get("baseline_non_regression", {})
    minimums = {
        "recall_at_k": "recall_at_3_min",
        "mrr": "mrr_min",
        "ndcg_at_k": "ndcg_at_3_min",
    }
    for metric, limit_name in minimums.items():
        if retrieval.get(metric, -1) < current_limits.get(limit_name, 1):
            findings.append(f"baseline_below_floor:{metric}")
    if retrieval.get("cross_principal_leakage_rate", 1) > current_limits.get(
        "cross_principal_leakage_max", 0
    ):
        findings.append("cross_principal_leakage_regression")
    if retrieval.get("cross_project_leakage_rate", 1) > current_limits.get(
        "cross_project_leakage_max", 0
    ):
        findings.append("cross_project_leakage_regression")
    if retrieval.get("false_positive_rate", 1) > current_limits.get(
        "false_positive_rate_max", 0
    ):
        findings.append("false_positive_rate_regression")
    if retrieval.get("expired_deleted_residual_count", 1) > current_limits.get(
        "expired_deleted_residual_max", 0
    ):
        findings.append("expired_deleted_residual_regression")
    if baseline.get("provider_calls", {}).get("no_memory_extra", 1) > current_limits.get(
        "extra_provider_calls_max", 0
    ):
        findings.append("no_memory_outbound_regression")

    final = fixture.get("thresholds", {}).get("final_acceptance", {})
    if final.get("scope_leakage_rate_max") != 0.0:
        findings.append("final_scope_leakage_target_not_zero")
    if final.get("compaction_machine_ref_retention_min") != 1.0:
        findings.append("final_compaction_ref_target_not_complete")
    if final.get("cross_principal_evictions_max") != 0:
        findings.append("final_cross_principal_eviction_target_not_zero")
    if final.get("canonical_truncations_max") != 0:
        findings.append("final_canonical_truncation_target_not_zero")

    expected_loss_probes = set(fixture.get("data_loss_probes", []))
    recorded_loss_probes = set(inventory.get("data_loss_probes", {}))
    for probe in sorted(expected_loss_probes - recorded_loss_probes):
        findings.append(f"missing_data_loss_inventory:{probe}")
    return findings


def validate_repository(root: Path) -> list[str]:
    fixture_path = root / "scripts/fixtures/memory_context/wp0_baseline_v1.json"
    baseline_path = root / "scripts/baselines/memory_context_wp0_baseline.json"
    inventory_path = root / "scripts/inventories/memory_context_wp0_inventory.json"
    adr_path = root / "docs/memory_context_architecture_adr.md"
    required_paths = [fixture_path, baseline_path, inventory_path, adr_path]
    findings = [
        f"missing_file:{path.relative_to(root)}" for path in required_paths if not path.is_file()
    ]
    if findings:
        return findings

    raw_fixture = fixture_path.read_text(encoding="utf-8")
    fixture = load_json(fixture_path)
    baseline = load_json(baseline_path)
    inventory = load_json(inventory_path)
    findings.extend(validate_payloads(fixture, baseline, inventory, raw_fixture))

    memory_toml = tomllib.loads((root / "configs/memory.toml").read_text(encoding="utf-8"))
    expected_key_count = inventory.get("configuration", {}).get("tracked_key_count")
    if len(memory_toml) != expected_key_count:
        findings.append(
            f"memory_config_inventory_drift:expected={expected_key_count}:actual={len(memory_toml)}"
        )

    for relative, markers in REMOVED_RUNTIME_MARKERS.items():
        path = root / relative
        if not path.is_file():
            findings.append(f"missing_runtime_inventory_file:{relative}")
            continue
        text = path.read_text(encoding="utf-8")
        for marker in markers:
            if marker in text:
                findings.append(f"resolved_runtime_regression:{relative}:{marker}")

    for relative, markers in REQUIRED_COMPATIBILITY_MARKERS.items():
        path = root / relative
        if not path.is_file():
            findings.append(f"missing_runtime_inventory_file:{relative}")
            continue
        text = path.read_text(encoding="utf-8")
        for marker in markers:
            if marker not in text:
                findings.append(f"compatibility_inventory_drift:{relative}:{marker}")

    for entry in inventory.get("schema_writers", []):
        relative = entry.get("path", "")
        if not relative or not (root / relative).is_file():
            findings.append(f"missing_schema_writer:{relative}")

    adr = adr_path.read_text(encoding="utf-8")
    for heading in ADR_DECISIONS:
        if heading not in adr:
            findings.append(f"missing_adr_decision:{heading}")

    retrieval_tests = (
        root / "crates/clawd/src/memory/memory_wp0_baseline_tests.rs"
    ).read_text(encoding="utf-8")
    compaction_tests = (
        root / "crates/clawd/src/task_context_builder_compaction_tests.rs"
    ).read_text(encoding="utf-8")
    if "wp0_fixture_measures_current_retrieval_and_lifecycle_baseline" not in retrieval_tests:
        findings.append("missing_retrieval_behavior_baseline")
    if "wp0_diagnostic_reproduces_legacy_data_loss_risks" not in retrieval_tests:
        findings.append("missing_data_loss_diagnostic")
    if "wp0_disabled_memory_generation_has_zero_extra_provider_calls" not in retrieval_tests:
        findings.append("missing_no_memory_outbound_baseline")
    if "wp0_compaction_fixture_preserves_machine_refs_across_repeated_compaction" not in compaction_tests:
        findings.append("missing_compaction_behavior_baseline")
    return findings


def run_self_test() -> int:
    fixture = {
        "schema_version": 1,
        "fixture_id": "fixture",
        "synthetic_only": True,
        "contains_production_data": False,
        "coverage_cases": [{"categories": sorted(REQUIRED_COVERAGE)}],
        "data_loss_probes": ["probe"],
        "thresholds": {
            "baseline_non_regression": {
                "recall_at_3_min": 0.5,
                "mrr_min": 0.5,
                "ndcg_at_3_min": 0.5,
                "false_positive_rate_max": 0.5,
                "cross_principal_leakage_max": 0.0,
                "cross_project_leakage_max": 0.0,
                "expired_deleted_residual_max": 0.0,
                "extra_provider_calls_max": 0,
            },
            "final_acceptance": {
                "scope_leakage_rate_max": 0.0,
                "compaction_machine_ref_retention_min": 1.0,
                "cross_principal_evictions_max": 0,
                "canonical_truncations_max": 0,
            },
        },
    }
    baseline = {
        "schema_version": 1,
        "fixture_id": "fixture",
        "retrieval": {
            "recall_at_k": 0.5,
            "mrr": 0.5,
            "ndcg_at_k": 0.5,
            "false_positive_rate": 0.0,
            "cross_principal_leakage_rate": 0.0,
            "cross_project_leakage_rate": 0.0,
            "expired_deleted_residual_count": 0,
        },
        "provider_calls": {"no_memory_extra": 0},
    }
    inventory = {"data_loss_probes": {"probe": "reproduced"}}
    if validate_payloads(fixture, baseline, inventory, json.dumps(fixture)):
        print("SELF_TEST_FAIL positive fixture")
        return 1
    fixture["synthetic_only"] = False
    findings = validate_payloads(fixture, baseline, inventory, json.dumps(fixture))
    if "fixture_not_marked_synthetic" not in findings:
        print(f"SELF_TEST_FAIL negative fixture findings={findings}")
        return 1
    with tempfile.TemporaryDirectory(prefix="memory-context-wp0-") as tmp:
        if not Path(tmp).is_dir():
            print("SELF_TEST_FAIL temporary directory")
            return 1
    print("MEMORY_CONTEXT_WP0_CONTRACTS_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    findings = validate_repository(repo_root())
    if findings:
        print("MEMORY_CONTEXT_WP0_CONTRACTS_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print("MEMORY_CONTEXT_WP0_CONTRACTS_CHECK ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
