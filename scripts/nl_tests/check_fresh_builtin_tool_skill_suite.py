#!/usr/bin/env python3
"""Validate fresh fixed built-in NL coverage and historical prompt novelty."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
import unicodedata
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
REGISTRY = ROOT / "configs/skills_registry.toml"
CASES_ROOT = ROOT / "scripts/nl_tests/cases"
SUITE = CASES_ROOT / "nl_cases_builtin_tool_skill_fresh_20260830.txt"


def normalized_prompt(value: str) -> str:
    value = unicodedata.normalize("NFKC", value).casefold()
    return "".join(char for char in value if char.isalnum())


def case_rows(path: Path) -> list[tuple[int, str, str, set[str], str]]:
    rows: list[tuple[int, str, str, set[str], str]] = []
    for line_no, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 4)
        if len(parts) < 4:
            raise ValueError(f"{path}:{line_no}: invalid case row")
        metadata = parts[2].strip()
        match = re.search(r"(?:^|;)covers:([^;]+)", metadata.casefold())
        covers = {
            token.strip()
            for token in (match.group(1).split(",") if match else [])
            if token.strip()
        }
        rows.append((line_no, parts[1].strip(), metadata, covers, parts[3].strip()))
    return rows


def historical_prompts(exclude: Path) -> dict[str, list[str]]:
    prompts: dict[str, list[str]] = {}
    for path in sorted(CASES_ROOT.iterdir()):
        if path == exclude or not path.is_file() or path.suffix not in {".txt", ".jsonl"}:
            continue
        if path.suffix == ".jsonl":
            # JSONL corpora are generated protocol fixtures; their source NL rows
            # also live in text suites and are covered there.
            continue
        for line_no, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("|", 4)
            if len(parts) < 4:
                continue
            prompt = parts[3].strip()
            prompts.setdefault(normalized_prompt(prompt), []).append(
                f"{path.relative_to(ROOT)}:{line_no}"
            )
    return prompts


def fixed_builtin_names() -> set[str]:
    with REGISTRY.open("rb") as handle:
        skills = tomllib.load(handle).get("skills", [])
    return {
        str(skill["name"])
        for skill in skills
        if skill.get("install_mode") != "on_demand"
    }


def validate(suite: Path) -> list[str]:
    errors: list[str] = []
    rows = case_rows(suite)
    expected = fixed_builtin_names()
    covered = set().union(*(row[3] for row in rows)) if rows else set()
    missing = sorted(expected - covered)
    unexpected = sorted(covered - expected)
    if missing:
        errors.append("missing fixed built-ins: " + ", ".join(missing))
    if unexpected:
        errors.append("unexpected/non-fixed covers: " + ", ".join(unexpected))

    names: dict[str, int] = {}
    prompts: dict[str, int] = {}
    history = historical_prompts(suite)
    for line_no, name, metadata, _covers, prompt in rows:
        if name in names:
            errors.append(f"duplicate case name at lines {names[name]} and {line_no}: {name}")
        names[name] = line_no
        normalized = normalized_prompt(prompt)
        if not normalized:
            errors.append(f"line {line_no}: empty normalized prompt")
        if normalized in prompts:
            errors.append(
                f"duplicate normalized prompt at lines {prompts[normalized]} and {line_no}"
            )
        prompts[normalized] = line_no
        if normalized in history:
            errors.append(
                f"line {line_no}: prompt already exists at {', '.join(history[normalized][:3])}"
            )
        if "requires_tool_call=true" not in metadata.casefold():
            errors.append(f"line {line_no}: requires_tool_call=true is missing")

    if len(rows) < len(expected):
        errors.append(
            f"suite has {len(rows)} rows for {len(expected)} fixed built-ins; "
            "each fixed entry needs a dedicated fresh acceptance row"
        )
    return errors


def self_test() -> None:
    assert normalized_prompt(" Hello, WORLD! ") == normalized_prompt("hello world")
    assert normalized_prompt("Ａｇｅｎｔ １２３") == "agent123"
    print("FRESH_BUILTIN_TOOL_SKILL_SUITE_SELF_TEST_OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--suite", type=Path, default=SUITE)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    suite = args.suite.resolve()
    errors = validate(suite)
    if errors:
        for error in errors:
            print(f"FRESH_BUILTIN_TOOL_SKILL_SUITE_ERROR {error}")
        return 1
    rows = case_rows(suite)
    print(
        "FRESH_BUILTIN_TOOL_SKILL_SUITE_OK "
        f"rows={len(rows)} fixed_builtins={len(fixed_builtin_names())} "
        "missing=0 historical_duplicates=0"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
