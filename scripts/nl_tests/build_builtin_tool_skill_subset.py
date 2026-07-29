#!/usr/bin/env python3
"""Generate the smallest current NL smoke set covering built-in tools and skills."""

from __future__ import annotations

import argparse
import dataclasses
import functools
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/nl_tests/cases/nl_cases_basic_skill_100_coverage_20260629.txt"
REGISTRY = ROOT / "configs/skills_registry.toml"
OUTPUT = ROOT / "scripts/nl_tests/cases/nl_cases_builtin_tool_skill_minimal_current.txt"
REPORT = ROOT / "scripts/nl_tests/cases/nl_cases_builtin_tool_skill_minimal_current_coverage.json"

BEHAVIOR_CATEGORIES = {
    "archive_roundtrip",
    "async_cancel",
    "async_poll",
    "async_start",
    "clarify",
    "cleanup",
    "cancel_job",
    "external_network",
    "failure",
    "local_readonly",
    "local_side_effect",
    "resumable_job",
}


@dataclasses.dataclass(frozen=True)
class CaseRow:
    ordinal: int
    line: str
    name: str
    metadata: str
    covers: frozenset[str]


def registry_skill_sets(path: Path) -> tuple[set[str], set[str]]:
    with path.open("rb") as handle:
        skills = tomllib.load(handle).get("skills", [])
    core = {
        str(skill["name"])
        for skill in skills
        if skill.get("install_mode") != "on_demand"
    }
    optional = {
        str(skill["name"])
        for skill in skills
        if skill.get("install_mode") == "on_demand"
    }
    return core, optional


def parse_rows(path: Path) -> list[CaseRow]:
    rows: list[CaseRow] = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 4)
        if len(parts) < 4:
            raise ValueError(f"invalid case row: {line}")
        metadata = parts[2].strip().lower()
        match = re.search(r"(?:^|;)covers:([^;]+)", metadata)
        covers = frozenset(
            token.strip()
            for token in (match.group(1).split(",") if match else [])
            if token.strip()
        )
        rows.append(
            CaseRow(
                ordinal=len(rows) + 1,
                line=line,
                name=parts[1].strip(),
                metadata=metadata,
                covers=covers,
            )
        )
    return rows


def row_categories(row: CaseRow, core: set[str]) -> set[str]:
    categories = {f"skill:{name}" for name in row.covers & core}
    metadata_tokens = {
        token.strip()
        for token in re.split(r"[;,]", row.metadata)
        if token.strip()
    }
    categories.update(
        f"behavior:{name}" for name in metadata_tokens & BEHAVIOR_CATEGORIES
    )
    if row.name.endswith("_zh"):
        categories.add("language:zh_cn")
    if row.name.endswith("_en"):
        categories.add("language:en_us")
    return categories


def row_uses_non_x_dry_run(row: CaseRow) -> bool:
    if "x" in row.covers:
        return False
    metadata_tokens = {
        token.strip()
        for token in re.split(r"[;,]", row.metadata)
        if token.strip()
    }
    return "dry_run" in metadata_tokens or "side_effect_mode:dry_run" in metadata_tokens


def select_minimal(
    rows: list[CaseRow], core: set[str], optional: set[str]
) -> tuple[list[CaseRow], set[str], set[str], int]:
    eligible = [
        row
        for row in rows
        if not (row.covers & optional)
        and "optional_skill_deferred" not in row.metadata
        and not row_uses_non_x_dry_run(row)
    ]
    required = {f"skill:{name}" for name in core}
    required.update(f"behavior:{name}" for name in BEHAVIOR_CATEGORIES)
    required.update({"language:zh_cn", "language:en_us"})
    available = set().union(*(row_categories(row, core) for row in eligible))
    missing = required - available
    if missing:
        return [], required, missing, len(rows) - len(eligible)

    categories = {
        row: frozenset(row_categories(row, core) & required) for row in eligible
    }
    by_category = {
        category: [row for row in eligible if category in categories[row]]
        for category in required
    }
    essential = {
        candidates[0]
        for candidates in by_category.values()
        if len(candidates) == 1
    }
    covered_by_essential = set().union(*(categories[row] for row in essential))
    remaining = frozenset(required - covered_by_essential)
    candidates = [
        row
        for row in eligible
        if row not in essential and categories[row] & remaining
    ]
    # Equal-cost strict subsets can never improve a minimum-cardinality cover.
    candidates = [
        row
        for row in candidates
        if not any(
            categories[row] & remaining < categories[other] & remaining
            for other in candidates
        )
    ]
    remaining_index = {
        category: [row for row in candidates if category in categories[row]]
        for category in remaining
    }

    @functools.lru_cache(maxsize=None)
    def exact_cover(uncovered: frozenset[str]) -> tuple[CaseRow, ...]:
        if not uncovered:
            return ()
        branch = min(
            uncovered,
            key=lambda category: (len(remaining_index[category]), category),
        )
        best: tuple[CaseRow, ...] | None = None
        for row in sorted(
            remaining_index[branch],
            key=lambda item: (-len(categories[item] & uncovered), item.ordinal),
        ):
            tail = exact_cover(uncovered - categories[row])
            candidate = (row, *tail)
            if best is None or (len(candidate), tuple(x.ordinal for x in candidate)) < (
                len(best),
                tuple(x.ordinal for x in best),
            ):
                best = candidate
        if best is None:
            raise ValueError(f"uncoverable categories: {sorted(uncovered)}")
        return best

    selected = essential | set(exact_cover(remaining))
    return sorted(selected, key=lambda row: row.ordinal), required, set(), len(rows) - len(eligible)


def render_output(selected: list[CaseRow], required: set[str], source_sha: str) -> str:
    return "\n".join(
        [
            "# Generated minimal NL suite for built-in RustClaw tools and skills.",
            "# Do not edit by hand; run scripts/nl_tests/build_builtin_tool_skill_subset.py.",
            f"# source_sha256={source_sha}",
            f"# selected_rows={len(selected)} required_categories={len(required)} missing_categories=0",
            "# Selection: exact minimum-cardinality set cover over registry skills, safety/lifecycle classes, and zh/en.",
            "# Scope: core/fixed built-ins only; on-demand Skill Store packages are excluded.",
            "# Execution: every selected capability performs a real call; no non-X dry-run is eligible.",
            "# Safety: local mutations use disposable tmp fixtures and include cleanup or restoration.",
            "# Format: suite|name|tags|prompt|expect=optional substring",
            "",
            *(row.line for row in selected),
            "",
        ]
    )


def report_payload(
    selected: list[CaseRow], required: set[str], core: set[str], excluded_rows: int, source_sha: str
) -> dict[str, object]:
    covered = set().union(*(row_categories(row, core) for row in selected))
    return {
        "schema_version": 1,
        "source": SOURCE.relative_to(ROOT).as_posix(),
        "source_sha256": source_sha,
        "registry": REGISTRY.relative_to(ROOT).as_posix(),
        "core_skill_count": len(core),
        "required_category_count": len(required),
        "selected_row_count": len(selected),
        "selection_algorithm": "exact_minimum_cardinality_set_cover",
        "minimum_proven": True,
        "excluded_optional_row_count": excluded_rows,
        "selected_non_x_dry_run_count": sum(
            1 for row in selected if row_uses_non_x_dry_run(row)
        ),
        "missing_categories": sorted(required - covered),
        "covered_categories": sorted(required & covered),
        "selected_cases": [row.name for row in selected],
    }


def build() -> tuple[str, str, dict[str, object]]:
    core, optional = registry_skill_sets(REGISTRY)
    rows = parse_rows(SOURCE)
    selected, required, missing, excluded_rows = select_minimal(rows, core, optional)
    if missing:
        raise ValueError(f"built-in NL coverage missing: {sorted(missing)}")
    source_sha = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    payload = report_payload(selected, required, core, excluded_rows, source_sha)
    return render_output(selected, required, source_sha), json.dumps(
        payload, ensure_ascii=False, indent=2, sort_keys=True
    ) + "\n", payload


def self_test() -> None:
    rows = [
        CaseRow(1, "a", "a_zh", "local_readonly", frozenset({"one", "two"})),
        CaseRow(2, "b", "b_en", "cleanup", frozenset({"two"})),
        CaseRow(3, "c", "c_en", "cleanup", frozenset({"three"})),
    ]
    original = set(BEHAVIOR_CATEGORIES)
    try:
        BEHAVIOR_CATEGORIES.clear()
        BEHAVIOR_CATEGORIES.update({"local_readonly", "cleanup"})
        selected, required, missing, excluded = select_minimal(
            rows, {"one", "two", "three"}, set()
        )
        assert not missing and excluded == 0
        assert {row.name for row in selected} == {"a_zh", "c_en"}
        assert len(required) == 7
    finally:
        BEHAVIOR_CATEGORIES.clear()
        BEHAVIOR_CATEGORIES.update(original)
    print("BUILTIN_TOOL_SKILL_NL_SUBSET_SELF_TEST_OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    output, report, payload = build()
    if args.check:
        stale = []
        if not OUTPUT.is_file() or OUTPUT.read_text(encoding="utf-8") != output:
            stale.append(OUTPUT.relative_to(ROOT).as_posix())
        if not REPORT.is_file() or REPORT.read_text(encoding="utf-8") != report:
            stale.append(REPORT.relative_to(ROOT).as_posix())
        if stale:
            print("BUILTIN_TOOL_SKILL_NL_SUBSET_STALE " + " ".join(stale))
            return 1
    else:
        OUTPUT.write_text(output, encoding="utf-8")
        REPORT.write_text(report, encoding="utf-8")
    print(
        "BUILTIN_TOOL_SKILL_NL_SUBSET_OK "
        f"core_skills={payload['core_skill_count']} "
        f"selected_rows={payload['selected_row_count']} "
        f"required_categories={payload['required_category_count']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
