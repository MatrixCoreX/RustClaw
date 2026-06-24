#!/usr/bin/env python3
"""Guard skill registry aliases as language-neutral machine tokens."""

from __future__ import annotations

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


def main() -> int:
    findings: list[str] = []
    alias_count = 0
    for path in REGISTRIES:
        raw = load_registry(path)
        rel = path.relative_to(ROOT)
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
    if findings:
        print(f"SKILL_REGISTRY_ALIAS_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print(f"SKILL_REGISTRY_ALIAS_CHECK ok registries={len(REGISTRIES)} aliases={alias_count}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
