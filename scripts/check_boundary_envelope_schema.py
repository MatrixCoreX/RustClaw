#!/usr/bin/env python3
"""Guard the target BoundaryEnvelope schema stays machine-only.

This is the target schema for shrinking the intent normalizer boundary. It must
not grow back into a pre-planner semantic router, answer generator, or raw user
text carrier.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SCHEMA = ROOT / "prompts" / "schemas" / "boundary_envelope.schema.json"
DEFAULT_INTENT_NORMALIZER_SCHEMA = (
    ROOT / "prompts" / "schemas" / "intent_normalizer.schema.json"
)

EXPECTED_FIELDS: frozenset[str] = frozenset(
    {
        "schema_version",
        "raw_chars",
        "language_hint",
        "schedule_intent",
        "attachment_refs",
        "explicit_locators",
        "active_task_reference",
        "session_binding",
        "safety_budget_hint",
    }
)

FORBIDDEN_FIELDS: frozenset[str] = frozenset(
    {
        "raw_user_request",
        "user_prompt",
        "resolved_user_intent",
        "reason",
        "decision",
        "answer_candidate",
        "direct_answer",
        "planner_execute",
        "route_authority",
        "semantic_route_authority",
        "semantic_kind",
        "output_contract",
        "capability_ref",
    }
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    location: str
    kind: str
    detail: str


def rel(path: Path) -> str:
    return path.resolve().relative_to(ROOT).as_posix()


def load_schema(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{rel(path)} must contain a JSON object")
    return value


def object_keys(value: Any) -> set[str]:
    if isinstance(value, dict):
        return set(value)
    return set()


def string_list_values(value: Any) -> set[str]:
    if isinstance(value, list):
        return {item for item in value if isinstance(item, str)}
    return set()


def field_schema(schema: dict[str, Any], field: str) -> dict[str, Any]:
    value = schema.get("properties", {}).get(field, {})
    if isinstance(value, dict):
        return value
    return {}


def schema_type_set(field: dict[str, Any]) -> set[str]:
    value = field.get("type")
    if isinstance(value, str):
        return {value}
    if isinstance(value, list):
        return {item for item in value if isinstance(item, str)}
    return set()


def prefixed(findings: list[Finding], prefix: str) -> list[Finding]:
    return [
        dataclasses.replace(finding, location=f"{prefix}.{finding.location}")
        for finding in findings
    ]


def scan_schema(schema: dict[str, Any], path: Path) -> list[Finding]:
    rel_path = rel(path)
    findings: list[Finding] = []
    properties = object_keys(schema.get("properties"))
    required = string_list_values(schema.get("required"))

    if schema.get("additionalProperties") is not False:
        findings.append(
            Finding(
                rel_path,
                "additionalProperties",
                "additional_properties_not_false",
                "BoundaryEnvelope must be closed to known machine fields",
            )
        )

    schema_version = field_schema(schema, "schema_version")
    if schema_type_set(schema_version) != {"integer"} or schema_version.get("const") != 1:
        findings.append(
            Finding(
                rel_path,
                "properties.schema_version",
                "schema_version_not_const_one",
                "schema_version must be integer const=1",
            )
        )

    for field in sorted(EXPECTED_FIELDS - properties):
        findings.append(Finding(rel_path, "properties", "missing_field", field))
    for field in sorted(properties - EXPECTED_FIELDS):
        findings.append(Finding(rel_path, "properties", "unexpected_field", field))
    for field in sorted(EXPECTED_FIELDS - required):
        findings.append(Finding(rel_path, "required", "missing_required", field))
    for field in sorted(required - EXPECTED_FIELDS):
        findings.append(Finding(rel_path, "required", "unexpected_required", field))
    for field in sorted((properties | required) & FORBIDDEN_FIELDS):
        findings.append(Finding(rel_path, "schema", "forbidden_field", field))

    if schema_type_set(field_schema(schema, "raw_chars")) != {"integer"}:
        findings.append(
            Finding(
                rel_path,
                "properties.raw_chars.type",
                "raw_chars_not_integer",
                "raw_chars must be an integer count, not raw text or token string",
            )
        )
    for array_field in ("attachment_refs", "explicit_locators"):
        if schema_type_set(field_schema(schema, array_field)) != {"array"}:
            findings.append(
                Finding(
                    rel_path,
                    f"properties.{array_field}.type",
                    "boundary_ref_field_not_array",
                    array_field,
                )
            )

    return findings


def scan_file(path: Path) -> list[Finding]:
    return scan_schema(load_schema(path), path)


def scan_intent_normalizer_embedded_boundary(path: Path) -> list[Finding]:
    return scan_intent_normalizer_embedded_boundary_from_schema(load_schema(path), path)


def scan_intent_normalizer_embedded_boundary_from_schema(
    schema: dict[str, Any],
    path: Path,
) -> list[Finding]:
    rel_path = rel(path)
    boundary_schema = schema.get("properties", {}).get("boundary_envelope")
    if not isinstance(boundary_schema, dict):
        return [
            Finding(
                rel_path,
                "properties.boundary_envelope",
                "embedded_boundary_missing",
                "intent normalizer schema must embed the BoundaryEnvelope contract",
            )
        ]

    findings: list[Finding] = []
    type_set = schema_type_set(boundary_schema)
    if "object" not in type_set or not type_set.issubset({"object", "null"}):
        findings.append(
            Finding(
                rel_path,
                "properties.boundary_envelope.type",
                "embedded_boundary_not_nullable_object",
                "boundary_envelope must be object or null for transitional compatibility",
            )
        )
    findings.extend(
        prefixed(
            scan_schema(boundary_schema, path),
            "properties.boundary_envelope",
        )
    )
    return findings


def print_report(findings: list[Finding]) -> int:
    print(f"BOUNDARY_ENVELOPE_SCHEMA_CHECK findings={len(findings)}")
    for finding in findings:
        print(
            "  - "
            f"{finding.path} {finding.location} "
            f"[{finding.kind}] {finding.detail}"
        )
    return 1 if findings else 0


def run_self_test() -> int:
    good_schema = {
        "type": "object",
        "additionalProperties": False,
        "required": sorted(EXPECTED_FIELDS),
        "properties": {
            "schema_version": {"type": "integer", "const": 1},
            "raw_chars": {"type": "integer"},
            "language_hint": {"type": ["string", "null"]},
            "schedule_intent": {"type": ["object", "null"]},
            "attachment_refs": {"type": "array", "items": {"type": "string"}},
            "explicit_locators": {"type": "array", "items": {"type": "string"}},
            "active_task_reference": {"type": ["string", "null"]},
            "session_binding": {"type": ["string", "null"]},
            "safety_budget_hint": {"type": ["string", "null"]},
        },
    }
    dummy_path = DEFAULT_SCHEMA
    assert scan_schema(good_schema, dummy_path) == []

    bad_raw_text = {
        **good_schema,
        "properties": {**good_schema["properties"], "raw_user_request": {"type": "string"}},
    }
    assert any(item.kind == "forbidden_field" for item in scan_schema(bad_raw_text, dummy_path))

    bad_semantic = {
        **good_schema,
        "properties": {**good_schema["properties"], "decision": {"type": "string"}},
    }
    assert any(item.detail == "decision" for item in scan_schema(bad_semantic, dummy_path))

    bad_raw_chars = {
        **good_schema,
        "properties": {**good_schema["properties"], "raw_chars": {"type": "string"}},
    }
    assert any(item.kind == "raw_chars_not_integer" for item in scan_schema(bad_raw_chars, dummy_path))

    bad_schema_version = {
        **good_schema,
        "properties": {**good_schema["properties"], "schema_version": {"type": "integer"}},
    }
    assert any(
        item.kind == "schema_version_not_const_one"
        for item in scan_schema(bad_schema_version, dummy_path)
    )

    good_intent_schema = {
        "type": "object",
        "additionalProperties": False,
        "properties": {
            "boundary_envelope": {
                **good_schema,
                "type": ["object", "null"],
            }
        },
    }
    intent_path = DEFAULT_INTENT_NORMALIZER_SCHEMA
    assert scan_intent_normalizer_embedded_boundary_from_schema(
        good_intent_schema,
        intent_path,
    ) == []

    bad_intent_schema = {
        "type": "object",
        "additionalProperties": False,
        "properties": {"boundary_envelope": {"type": ["object", "null"]}},
    }
    assert any(
        item.kind == "missing_field"
        for item in scan_intent_normalizer_embedded_boundary_from_schema(
            bad_intent_schema,
            intent_path,
        )
    )

    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument(
        "--intent-normalizer-schema",
        type=Path,
        default=DEFAULT_INTENT_NORMALIZER_SCHEMA,
    )
    parser.add_argument(
        "--target-only",
        action="store_true",
        help="Only check boundary_envelope.schema.json, not the live normalizer embedding.",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    findings = scan_file(args.schema)
    if not args.target_only:
        findings.extend(scan_intent_normalizer_embedded_boundary(args.intent_normalizer_schema))
    return print_report(findings)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
