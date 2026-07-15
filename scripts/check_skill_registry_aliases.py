#!/usr/bin/env python3
"""Guard skill registry aliases as language-neutral machine tokens."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REGISTRIES = (
    ROOT / "configs" / "skills_registry.toml",
    ROOT / "docker" / "config" / "skills_registry.toml",
)
ALIAS_RE = re.compile(r"^[a-z0-9][a-z0-9_.-]{0,63}$")


def load_registry(path: Path) -> dict[str, Any]:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise SystemExit(f"failed_to_read_registry path={path} error={exc}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise SystemExit(f"failed_to_parse_registry path={path} error={exc}") from exc


def check_registry_aliases(raw: dict[str, Any], rel: Path) -> tuple[list[str], int]:
    findings: list[str] = []
    alias_count = 0
    for skill in raw.get("skills", []):
        skill_name = str(skill.get("name", "")).strip() or "<missing>"
        aliases = skill.get("aliases", [])
        if not isinstance(aliases, list):
            findings.append(f"{rel}:{skill_name}: aliases_not_array")
            continue
        seen: set[str] = set()
        for alias in aliases:
            alias_count += 1
            if not isinstance(alias, str):
                findings.append(f"{rel}:{skill_name}: alias_not_string={alias!r}")
                continue
            token = alias.strip()
            if token != alias:
                findings.append(f"{rel}:{skill_name}: alias_has_outer_space={alias!r}")
            if token in seen:
                findings.append(f"{rel}:{skill_name}: duplicate_alias={token}")
            seen.add(token)
            if not ALIAS_RE.fullmatch(token):
                findings.append(f"{rel}:{skill_name}: alias_not_machine_token={alias!r}")
    return findings, alias_count


def scan_registries() -> tuple[list[str], int]:
    findings: list[str] = []
    alias_count = 0
    for path in REGISTRIES:
        path_findings, path_alias_count = check_registry_aliases(
            load_registry(path),
            path.relative_to(ROOT),
        )
        findings.extend(path_findings)
        alias_count += path_alias_count
    return findings, alias_count


def run_self_test() -> int:
    good = {"skills": [{"name": "good", "aliases": ["fs.basic", "run_cmd-1"]}]}
    bad = {
        "skills": [
            {
                "name": "bad",
                "aliases": ["中文", " english", "dup", "dup", 7],
            },
            {"name": "not_array", "aliases": "run"},
        ]
    }
    if check_registry_aliases(good, Path("configs/skills_registry.toml"))[0]:
        print("SELF_TEST_FAIL good_alias_false_positive", file=sys.stderr)
        return 1
    bad_findings, _ = check_registry_aliases(bad, Path("configs/skills_registry.toml"))
    expected_tokens = {
        "alias_not_machine_token",
        "alias_has_outer_space",
        "duplicate_alias",
        "alias_not_string",
        "aliases_not_array",
    }
    observed_tokens = {
        token
        for finding in bad_findings
        for token in expected_tokens
        if token in finding
    }
    if not expected_tokens.issubset(observed_tokens):
        print(f"SELF_TEST_FAIL missing_alias_findings:{bad_findings}", file=sys.stderr)
        return 1
    print("SKILL_REGISTRY_ALIAS_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    findings, alias_count = scan_registries()
    if findings:
        print(f"SKILL_REGISTRY_ALIAS_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print(f"SKILL_REGISTRY_ALIAS_CHECK ok registries={len(REGISTRIES)} aliases={alias_count}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
