#!/usr/bin/env python3
"""Ensure planner capability governance fields are explicit in registries."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Any

try:
    import tomlkit
except ImportError as exc:  # pragma: no cover - dependency check path
    raise SystemExit(
        "missing_dependency=tomlkit install tomlkit or run in the repo dev environment"
    ) from exc

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PATHS = (
    REPO_ROOT / "configs" / "skills_registry.toml",
    REPO_ROOT / "docker" / "config" / "skills_registry.toml",
)

GOVERNANCE_BY_EFFECT = {
    "observe": {
        "idempotent": True,
        "dedup_scope": "args",
    },
    "validate": {
        "idempotent": True,
        "dedup_scope": "args",
    },
    "mutate": {
        "once_per_task": True,
        "idempotent": False,
        "dedup_scope": "action",
    },
    "external": {
        "once_per_task": True,
        "idempotent": False,
        "dedup_scope": "action",
    },
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths",
        nargs="*",
        type=Path,
        default=list(DEFAULT_PATHS),
        help="Registry TOML files to check or update.",
    )
    parser.add_argument(
        "--write",
        action="store_true",
        help="Write missing governance fields. Default checks only.",
    )
    return parser.parse_args()


def normalize_effect(value: Any) -> str:
    return str(value or "").strip().lower()


def sync_path(path: Path, write: bool) -> tuple[int, int]:
    original_text = path.read_text(encoding="utf-8")
    doc = tomlkit.parse(original_text)
    missing = 0
    touched = 0
    for skill in doc.get("skills", []):
        for capability in skill.get("planner_capabilities", []):
            expected = GOVERNANCE_BY_EFFECT.get(normalize_effect(capability.get("effect")))
            if not expected:
                continue
            for key, value in expected.items():
                if key in capability:
                    continue
                missing += 1
                if write:
                    capability[key] = value
                    touched += 1
    if write:
        rendered = format_governance_inline_tables(tomlkit.dumps(doc))
        if touched or rendered != original_text:
            path.write_text(rendered, encoding="utf-8")
    return missing, touched


def format_governance_inline_tables(text: str) -> str:
    text = re.sub(
        r",(?=(?:once_per_task|idempotent|dedup_scope)\s*=)",
        ", ",
        text,
    )
    text = re.sub(
        r'(dedup_scope\s*=\s*"(?:args|action)")}',
        r"\1 }",
        text,
    )
    return text


def main() -> int:
    args = parse_args()
    total_missing = 0
    total_touched = 0
    for path in args.paths:
        path = path if path.is_absolute() else REPO_ROOT / path
        missing, touched = sync_path(path, args.write)
        total_missing += missing
        total_touched += touched
        print(
            "REGISTRY_GOVERNANCE_FIELDS "
            f"path={path} mode={'write' if args.write else 'check'} "
            f"missing={missing} touched={touched}"
        )
    if total_missing and not args.write:
        return 1
    print(
        "REGISTRY_GOVERNANCE_FIELDS_SUMMARY "
        f"missing={total_missing} touched={total_touched}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
