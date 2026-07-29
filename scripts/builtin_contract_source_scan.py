#!/usr/bin/env python3
"""Source-derived details for the built-in capability contract inventory."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any, Iterable


SOURCE_SUFFIXES = {".rs", ".py", ".mjs", ".js", ".ts", ".go"}
SKILL_SOURCE_ROOTS = ("crates/skills", "optional_skills", "external_skills")
RUNTIME_SOURCE_ROOTS = (
    "crates/clawd/src",
    "crates/skill-runner/src",
    "crates/skill-sdk/src",
)
PATH_FIELD_NAMES = {
    "archive",
    "cwd",
    "destination",
    "directory",
    "file",
    "path",
    "paths",
    "root",
    "workspace",
}
PATH_FIELD_SUFFIXES = ("_path", "_paths", "_dir", "_directory", "_root")
AUTHORITY_MARKERS = (
    "SkillPathPolicy",
    "PathAuthority",
    "authority_scope",
    "allow_path_outside_workspace",
    "unrestricted_admin",
)
GENERIC_ERROR_WRITER_RE = re.compile(
    r"(?:[\"']error_kind[\"']\s*:|(?:insert|entry)\(\s*[\"']error_kind[\"'])"
)
GENERIC_ERROR_READER_RE = re.compile(
    r"(?:get\(\s*[\"']error_kind[\"']\s*\)|pointer\(\s*[\"']/error_kind[\"']\s*\)|\.error_kind\b)"
)
LEGACY_ALIAS_RE = re.compile(r"alias\s*=\s*[\"']error_kind[\"']")
DOMAIN_ERROR_KIND_RE = re.compile(
    r"\b(?:provider|last_retry|final|failure_attribution)_error_kind\b"
)


def _relative(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def _is_test_or_fixture(path: Path) -> bool:
    lowered_parts = {part.lower() for part in path.parts}
    lowered_name = path.name.lower()
    return (
        "tests" in lowered_parts
        or "fixtures" in lowered_parts
        or "test" in lowered_name
        or "fixture" in lowered_name
    )


def _source_files_below(path: Path) -> list[Path]:
    if not path.exists():
        return []
    if path.is_file():
        return [path] if path.suffix in SOURCE_SUFFIXES else []
    return sorted(
        candidate
        for candidate in path.rglob("*")
        if candidate.is_file()
        and candidate.suffix in SOURCE_SUFFIXES
        and not _is_test_or_fixture(candidate)
    )


def skill_producer_files(root: Path, skill: dict[str, Any]) -> list[str]:
    name = str(skill.get("name") or "").strip()
    candidates = [root / source / name for source in SKILL_SOURCE_ROOTS]
    if skill.get("kind") == "builtin":
        builtin_root = root / "crates" / "clawd" / "src" / "skills"
        exact = sorted(builtin_root.glob(f"builtin_{name}*.rs"))
        candidates.extend(exact or [builtin_root / "builtin.rs"])
    return sorted(
        {
            _relative(path, root)
            for candidate in candidates
            for path in _source_files_below(candidate)
        }
    )


def output_contract_pipeline(
    root: Path, skill: dict[str, Any], producer_files: Iterable[str]
) -> dict[str, Any]:
    name = str(skill.get("name") or "").strip()
    evidence_owner = str(skill.get("evidence_owner") or "").strip()
    return {
        "producer_files": sorted(set(producer_files)),
        "registry_schema": f"configs/skills_registry.toml:[[skills:{name}]].output_schema",
        "runtime_validator": "crates/clawd/src/agent_engine/skill_output_contract.rs",
        "envelope_builder": "crates/clawd/src/capability_result.rs",
        "evidence_extractor": {
            "owner": evidence_owner or "registered_skill_output",
            "registry": "crates/clawd/src/task_journal_evidence_registry.rs",
        },
        "delivery_readers": [
            "crates/clawd/src/finalize",
            "crates/clawd/src/worker/run_skill_finalize.rs",
        ],
    }


def path_authority_inventory(
    root: Path,
    skill: dict[str, Any],
    input_fields: Iterable[str],
    producer_files: Iterable[str],
) -> dict[str, Any]:
    fields = sorted(
        field
        for field in input_fields
        if field.lower() in PATH_FIELD_NAMES
        or field.lower().endswith(PATH_FIELD_SUFFIXES)
    )
    if not fields:
        return {
            "path_fields": [],
            "authority_mode": "not_path_consuming",
            "authority_markers": [],
        }
    markers: set[str] = set()
    for relative in producer_files:
        path = root / relative
        try:
            source = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        markers.update(marker for marker in AUTHORITY_MARKERS if marker in source)
    if skill.get("kind") == "builtin":
        mode = "runtime_owned_builtin_authority"
    elif markers:
        mode = "verified_runner_authority_consumed"
    else:
        mode = "authority_context_not_detected"
    return {
        "path_fields": fields,
        "authority_mode": mode,
        "authority_markers": sorted(markers),
    }


def _skill_name_from_path(relative: str) -> str | None:
    parts = Path(relative).parts
    for prefix in (("crates", "skills"), ("optional_skills",), ("external_skills",)):
        if parts[: len(prefix)] == prefix and len(parts) > len(prefix):
            return parts[len(prefix)]
    return None


def _legacy_allowlist(root: Path) -> list[str]:
    path = root / "crates" / "clawd" / "src" / "skills" / "error_contract.rs"
    try:
        source = path.read_text(encoding="utf-8")
    except OSError:
        return []
    match = re.search(
        r"CURRENT_LEGACY_ERROR_FIELD_PRODUCERS\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];",
        source,
        re.S,
    )
    return sorted(set(re.findall(r'"([a-z0-9_]+)"', match.group(1)))) if match else []


def error_field_inventory(root: Path) -> dict[str, Any]:
    locations: dict[str, list[dict[str, Any]]] = {
        "current_flow_writes": [],
        "current_flow_reads": [],
        "legacy_compatibility_reads": [],
        "tests_and_fixtures": [],
        "domain_specific_fields": [],
    }
    files = [
        path
        for source_root in (*SKILL_SOURCE_ROOTS, *RUNTIME_SOURCE_ROOTS)
        for path in _source_files_below(root / source_root)
    ]
    # Tests/fixtures are intentionally added separately so they cannot be
    # mistaken for production debt.
    for source_root in (*SKILL_SOURCE_ROOTS, *RUNTIME_SOURCE_ROOTS):
        base = root / source_root
        if not base.exists():
            continue
        files.extend(
            path
            for path in base.rglob("*")
            if path.is_file() and path.suffix in SOURCE_SUFFIXES and _is_test_or_fixture(path)
        )
    for path in sorted(set(files)):
        relative = _relative(path, root)
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError):
            continue
        for line_number, line in enumerate(lines, start=1):
            if "error_kind" not in line:
                continue
            entry = {
                "path": relative,
                "line": line_number,
                "skill": _skill_name_from_path(relative),
            }
            if DOMAIN_ERROR_KIND_RE.search(line):
                locations["domain_specific_fields"].append(entry)
            elif _is_test_or_fixture(path):
                locations["tests_and_fixtures"].append(entry)
            elif LEGACY_ALIAS_RE.search(line) or "legacy" in line.lower():
                locations["legacy_compatibility_reads"].append(entry)
            else:
                if GENERIC_ERROR_WRITER_RE.search(line):
                    locations["current_flow_writes"].append(entry)
                if GENERIC_ERROR_READER_RE.search(line):
                    locations["current_flow_reads"].append(entry)
    allowlist = _legacy_allowlist(root)
    unowned_writers = sorted(
        {
            str(entry["skill"])
            for entry in locations["current_flow_writes"]
            if entry["skill"] and entry["skill"] not in allowlist
        }
    )
    return {
        **locations,
        "legacy_producer_allowlist": allowlist,
        "unowned_skill_writers": unowned_writers,
        "counts": {key: len(value) for key, value in locations.items()},
    }


def self_test(root: Path) -> None:
    skill_dir = root / "crates" / "skills" / "demo"
    skill_dir.mkdir(parents=True)
    (skill_dir / "main.rs").write_text(
        'let old = json!({"error_kind": "failed"});\n'
        'let read = value.get("error_kind");\n'
        'let provider_error_kind = "rate_limited";\n',
        encoding="utf-8",
    )
    contract_dir = root / "crates" / "clawd" / "src" / "skills"
    contract_dir.mkdir(parents=True)
    (contract_dir / "error_contract.rs").write_text(
        'const CURRENT_LEGACY_ERROR_FIELD_PRODUCERS: &[&str] = &["demo"];\n',
        encoding="utf-8",
    )
    report = error_field_inventory(root)
    assert report["legacy_producer_allowlist"] == ["demo"]
    assert len(report["current_flow_writes"]) == 1
    assert len(report["current_flow_reads"]) == 1
    assert len(report["domain_specific_fields"]) == 1
    assert not report["unowned_skill_writers"]
    (contract_dir / "error_contract.rs").write_text(
        "const CURRENT_LEGACY_ERROR_FIELD_PRODUCERS: &[&str] = &[];\n", encoding="utf-8"
    )
    assert error_field_inventory(root)["unowned_skill_writers"] == ["demo"]
