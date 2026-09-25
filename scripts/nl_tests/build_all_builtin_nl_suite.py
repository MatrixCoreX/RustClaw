#!/usr/bin/env python3
"""Build a registry-complete NL suite for fixed and on-demand built-ins."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tomllib
from pathlib import Path

import build_builtin_tool_skill_subset as fixed_subset


ROOT = Path(__file__).resolve().parents[2]
REGISTRY = ROOT / "configs/skills_registry.toml"
OPTIONAL_CASE_DIR = ROOT / "scripts/nl_tests/cases/optional_skills"
OUTPUT = ROOT / "scripts/nl_tests/cases/nl_cases_builtin_tool_skill_all_current.txt"
OPTIONAL_OUTPUT = ROOT / "scripts/nl_tests/cases/nl_cases_builtin_tool_skill_on_demand_current.txt"
REGRESSION_OUTPUT = ROOT / "scripts/nl_tests/cases/nl_cases_builtin_tool_skill_regression_current.txt"
REPORT = ROOT / "scripts/nl_tests/cases/nl_cases_builtin_tool_skill_all_current_coverage.json"

REGRESSION_CASES = (
    "b100_011_fs_missing_file_graceful_en",
    "b100_git_remote_publish_missing_connection_en",
    "b100_memory_recent_zh",
    "b100_browser_session_snapshot_en",
    "b100_039_package_detect_zh",
    "b100_042_install_module_preview_zh",
    "b100_093_image_edit_live_zh",
    "b100_104_subagent_readonly_review_en",
)


def active_lines(text: str) -> list[str]:
    return [
        line.strip()
        for line in text.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


def metadata_tokens(line: str) -> set[str]:
    parts = line.split("|", 4)
    if len(parts) < 4:
        raise ValueError(f"invalid NL row: {line}")
    return {token.strip() for token in parts[2].split(",") if token.strip()}


def choose_optional_case(skill_name: str, case_path: Path) -> str:
    rows = active_lines(case_path.read_text(encoding="utf-8"))
    if not rows:
        raise ValueError(f"no NL cases for on-demand skill {skill_name}")

    # Forge requires a verified connection and map success requires an
    # operator-owned provider credential. Their typed contract-rejection cases
    # are deterministic and do not mutate state or depend on local secrets.
    if skill_name in {"git_forge", "map_merchant"}:
        candidates = [
            row for row in rows if "case_role:failure" in metadata_tokens(row)
        ]
    else:
        candidates = [row for row in rows if "case_role:happy" in metadata_tokens(row)]
    if not candidates:
        raise ValueError(f"no safe acceptance case for on-demand skill {skill_name}")
    row = candidates[0]
    parts = row.split("|", 4)
    parts[2] = f"covers:{skill_name};{parts[2]}"
    return "|".join(parts)


def registry_inventory() -> tuple[list[str], list[str]]:
    with REGISTRY.open("rb") as handle:
        entries = tomllib.load(handle).get("skills", [])
    fixed = [
        str(entry["name"])
        for entry in entries
        if entry.get("install_mode") != "on_demand"
    ]
    on_demand = [
        str(entry["name"])
        for entry in entries
        if entry.get("install_mode") == "on_demand"
    ]
    return fixed, on_demand


def render_suite(rows: list[str], summary: str) -> str:
    return "\n".join(
        [
            f"# {summary}",
            "# Do not edit by hand; run scripts/nl_tests/build_all_builtin_nl_suite.py.",
            "# X remains offline dry-run only; remote publishing, trading, and external writes are excluded.",
            "# Format: suite|name|tags|prompt|expect=optional substring",
            "",
            *rows,
            "",
        ]
    )


def select_named_rows(rows: list[str], names: tuple[str, ...]) -> list[str]:
    by_name = {row.split("|", 2)[1]: row for row in rows}
    missing = [name for name in names if name not in by_name]
    if missing:
        raise ValueError(f"missing regression NL cases: {missing}")
    return [by_name[name] for name in names]


def build() -> tuple[str, str, str, str, dict[str, object]]:
    fixed, on_demand = registry_inventory()
    fixed_text, _, fixed_report = fixed_subset.build()
    fixed_rows = active_lines(fixed_text)
    optional_rows = []
    source_hashes: dict[str, str] = {}
    for skill_name in on_demand:
        case_path = OPTIONAL_CASE_DIR / f"{skill_name}.txt"
        if not case_path.is_file():
            raise ValueError(f"missing on-demand NL case file: {case_path}")
        optional_rows.append(choose_optional_case(skill_name, case_path))
        source_hashes[case_path.relative_to(ROOT).as_posix()] = hashlib.sha256(
            case_path.read_bytes()
        ).hexdigest()

    selected = fixed_rows + optional_rows
    output = render_suite(
        selected,
        "Generated registry-complete NL suite for built-in tools and skills "
        f"(selected_rows={len(selected)} fixed={len(fixed)} on_demand={len(on_demand)}).",
    )
    optional_output = render_suite(
        optional_rows,
        "Generated on-demand built-in NL subset "
        f"(selected_rows={len(optional_rows)} on_demand={len(on_demand)}).",
    )
    regression_rows = select_named_rows(fixed_rows, REGRESSION_CASES)
    regression_output = render_suite(
        regression_rows,
        "Generated focused regression NL subset for recently corrected built-in paths "
        f"(selected_rows={len(regression_rows)}).",
    )
    payload: dict[str, object] = {
        "schema_version": 1,
        "registry": REGISTRY.relative_to(ROOT).as_posix(),
        "fixed_skill_count": len(fixed),
        "on_demand_skill_count": len(on_demand),
        "total_builtin_count": len(fixed) + len(on_demand),
        "selected_case_count": len(selected),
        "missing_skills": [],
        "fixed_skills": fixed,
        "on_demand_skills": on_demand,
        "selected_fixed_cases": fixed_report["selected_cases"],
        "selected_on_demand_cases": [row.split("|", 2)[1] for row in optional_rows],
        "selected_regression_cases": list(REGRESSION_CASES),
        "optional_case_source_sha256": source_hashes,
        "x_external_calls": 0,
        "remote_mutations": 0,
    }
    report = json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    return output, optional_output, regression_output, report, payload


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output, optional_output, regression_output, report, payload = build()
    if args.check:
        stale = []
        if not OUTPUT.is_file() or OUTPUT.read_text(encoding="utf-8") != output:
            stale.append(OUTPUT.relative_to(ROOT).as_posix())
        if (
            not OPTIONAL_OUTPUT.is_file()
            or OPTIONAL_OUTPUT.read_text(encoding="utf-8") != optional_output
        ):
            stale.append(OPTIONAL_OUTPUT.relative_to(ROOT).as_posix())
        if (
            not REGRESSION_OUTPUT.is_file()
            or REGRESSION_OUTPUT.read_text(encoding="utf-8") != regression_output
        ):
            stale.append(REGRESSION_OUTPUT.relative_to(ROOT).as_posix())
        if not REPORT.is_file() or REPORT.read_text(encoding="utf-8") != report:
            stale.append(REPORT.relative_to(ROOT).as_posix())
        if stale:
            print("ALL_BUILTIN_NL_SUITE_STALE " + " ".join(stale))
            return 1
    else:
        OUTPUT.write_text(output, encoding="utf-8")
        OPTIONAL_OUTPUT.write_text(optional_output, encoding="utf-8")
        REGRESSION_OUTPUT.write_text(regression_output, encoding="utf-8")
        REPORT.write_text(report, encoding="utf-8")
    print(
        "ALL_BUILTIN_NL_SUITE_OK "
        f"builtins={payload['total_builtin_count']} "
        f"cases={payload['selected_case_count']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
