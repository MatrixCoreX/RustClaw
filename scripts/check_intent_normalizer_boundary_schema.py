#!/usr/bin/env python3
"""Guard the live intent normalizer schema from regaining route authority.

The normalizer may expose boundary extraction fields, but ordinary semantic
decisions such as direct-answer vs execute and user-visible answer candidates
belong to the planner / agent loop. This guard checks schema structure, not
prompt prose, so prohibitive documentation such as "do not emit decision" does
not create false positives.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SCHEMA = ROOT / "prompts" / "schemas" / "intent_normalizer.schema.json"

FORBIDDEN_TOP_LEVEL_FIELDS: frozenset[str] = frozenset(
    {
        "decision",
        "answer_candidate",
        "direct_answer",
        "direct_answer_candidate",
        "planner_execute",
        "route_authority",
        "semantic_route_authority",
    }
)

FORBIDDEN_TOP_LEVEL_REQUIRED_FIELDS: frozenset[str] = frozenset(
    {
        "output_contract",
        "execution_recipe",
        "resume_behavior",
        "schedule_kind",
        "wants_file_delivery",
        "attachment_processing_required",
    }
)

FORBIDDEN_OUTPUT_CONTRACT_FIELDS: frozenset[str] = frozenset(
    {
        "semantic_kind",
        "semantic",
        "semantic_type",
        "semantic_route",
        "semantic_route_kind",
        "semantic_kind_hint",
        "answer_kind",
        "route_kind",
    }
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    location: str
    kind: str
    field: str


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def load_schema(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"{rel(path)} must contain a JSON object")
    return data


def object_keys(value: Any) -> set[str]:
    if isinstance(value, dict):
        return set(value)
    return set()


def list_values(value: Any) -> set[str]:
    if isinstance(value, list):
        return {item for item in value if isinstance(item, str)}
    return set()


def scan_schema(schema: dict[str, Any], path: Path) -> list[Finding]:
    rel_path = rel(path)
    findings: list[Finding] = []

    if schema.get("additionalProperties") is not False:
        findings.append(
            Finding(
                rel_path,
                "additionalProperties",
                "additional_properties_not_false",
                "false",
            )
        )

    top_properties = object_keys(schema.get("properties"))
    top_required = list_values(schema.get("required"))
    if "boundary_envelope" not in top_required:
        findings.append(
            Finding(
                rel_path,
                "required",
                "boundary_envelope_not_required",
                "boundary_envelope",
            )
        )
    for field in sorted(top_properties & FORBIDDEN_TOP_LEVEL_FIELDS):
        findings.append(Finding(rel_path, "properties", "top_level_field", field))
    for field in sorted(top_required & FORBIDDEN_TOP_LEVEL_FIELDS):
        findings.append(Finding(rel_path, "required", "top_level_required", field))
    for field in sorted(top_required & FORBIDDEN_TOP_LEVEL_REQUIRED_FIELDS):
        findings.append(
            Finding(rel_path, "required", "compat_field_required", field)
        )

    output_contract = schema.get("properties", {}).get("output_contract", {})
    output_properties = object_keys(output_contract.get("properties"))
    output_required = list_values(output_contract.get("required"))
    for field in sorted(output_properties & FORBIDDEN_OUTPUT_CONTRACT_FIELDS):
        findings.append(
            Finding(rel_path, "properties.output_contract.properties", "output_contract_field", field)
        )
    for field in sorted(output_required & FORBIDDEN_OUTPUT_CONTRACT_FIELDS):
        findings.append(
            Finding(rel_path, "properties.output_contract.required", "output_contract_required", field)
        )

    return findings


def scan_file(path: Path) -> list[Finding]:
    return scan_schema(load_schema(path), path)


def print_report(findings: list[Finding]) -> int:
    print(f"INTENT_NORMALIZER_BOUNDARY_SCHEMA_CHECK findings={len(findings)}")
    for finding in findings:
        print(
            "  - "
            f"{finding.path} {finding.location} "
            f"[{finding.kind}] {finding.field}"
        )
    return 1 if findings else 0


def run_self_test() -> int:
    good_schema = {
        "type": "object",
        "additionalProperties": False,
        "required": ["boundary_envelope", "resolved_user_intent"],
        "properties": {
            "boundary_envelope": {"type": "object"},
            "resolved_user_intent": {"type": "string"},
            "needs_clarify": {"type": "boolean"},
            "output_contract": {
                "type": ["object", "null"],
                "properties": {"contract_marker": {"type": "string"}},
            },
        },
    }
    dummy_path = ROOT / "prompts" / "schemas" / "intent_normalizer.schema.json"
    assert scan_schema(good_schema, dummy_path) == []

    bad_top_property = {
        "type": "object",
        "additionalProperties": False,
        "required": ["boundary_envelope"],
        "properties": {"decision": {"type": "string"}},
    }
    assert scan_schema(bad_top_property, dummy_path)[0].kind == "top_level_field"

    bad_top_required = {
        "type": "object",
        "additionalProperties": False,
        "required": ["boundary_envelope", "answer_candidate"],
        "properties": {"boundary_envelope": {"type": "object"}},
    }
    assert scan_schema(bad_top_required, dummy_path)[0].kind == "top_level_required"

    bad_missing_boundary_required = {
        "type": "object",
        "additionalProperties": False,
        "required": ["resolved_user_intent"],
        "properties": {"boundary_envelope": {"type": "object"}},
    }
    assert (
        scan_schema(bad_missing_boundary_required, dummy_path)[0].kind
        == "boundary_envelope_not_required"
    )

    bad_required_compat = {
        "type": "object",
        "additionalProperties": False,
        "required": ["boundary_envelope", "output_contract"],
        "properties": {"boundary_envelope": {"type": "object"}},
    }
    assert scan_schema(bad_required_compat, dummy_path)[0].kind == "compat_field_required"

    bad_contract_property = {
        "type": "object",
        "additionalProperties": False,
        "required": ["boundary_envelope"],
        "properties": {
            "boundary_envelope": {"type": "object"},
            "output_contract": {
                "type": ["object", "null"],
                "properties": {"semantic_kind": {"type": "string"}},
            }
        },
    }
    assert scan_schema(bad_contract_property, dummy_path)[0].kind == "output_contract_field"

    bad_contract_required = {
        "type": "object",
        "additionalProperties": False,
        "required": ["boundary_envelope"],
        "properties": {
            "boundary_envelope": {"type": "object"},
            "output_contract": {
                "type": ["object", "null"],
                "required": ["answer_kind"],
                "properties": {},
            }
        },
    }
    assert scan_schema(bad_contract_required, dummy_path)[0].kind == "output_contract_required"

    bad_open_schema = {
        "type": "object",
        "additionalProperties": True,
        "required": ["boundary_envelope"],
        "properties": {"boundary_envelope": {"type": "object"}},
    }
    assert scan_schema(bad_open_schema, dummy_path)[0].kind == "additional_properties_not_false"

    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    return print_report(scan_file(args.schema))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
