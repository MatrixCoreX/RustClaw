#!/usr/bin/env python3
"""Check that every registry skill has a default vendor skill prompt.

Canonical registry prompt_file remains prompts/skills/<name>.md as a logical path.
Runtime loads skill prompts from vendor layers only:
prompts/vendors/<active_vendor>/skills/<name>.md -> prompts/vendors/default/skills/<name>.md.
This script validates the required fallback baseline under vendors/default.
Does not touch production code or clawd.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = REPO_ROOT / "configs" / "skills_registry.toml"
VENDOR_DEFAULT_SKILLS = REPO_ROOT / "prompts" / "vendors" / "default" / "skills"


def parse_registry_prompt_files(path: Path) -> list[tuple[str, str]]:
    """Return list of (skill_name, prompt_file) from [[skills]] blocks."""
    text = path.read_text(encoding="utf-8")
    out: list[tuple[str, str]] = []
    name_re = re.compile(r'^\s*name\s*=\s*"([^"]+)"', re.M)
    prompt_re = re.compile(r'^\s*prompt_file\s*=\s*"([^"]+)"', re.M)
    blocks = re.split(r'(?m)^\[\[skills\]\]\s*$', text)
    for block in blocks:
        if not block.strip():
            continue
        name_m = name_re.search(block)
        prompt_m = prompt_re.search(block)
        if name_m and prompt_m:
            out.append((name_m.group(1), prompt_m.group(1)))
    return out


def main() -> int:
    if not REGISTRY_PATH.exists():
        print(f"Registry not found: {REGISTRY_PATH}", file=sys.stderr)
        return 1
    skills = parse_registry_prompt_files(REGISTRY_PATH)
    missing: list[str] = []
    for name, prompt_file in skills:
        if not prompt_file.strip().startswith("prompts/skills/"):
            continue
        base = Path(prompt_file.strip()).name
        in_vendor_default = (VENDOR_DEFAULT_SKILLS / base).is_file()
        if not in_vendor_default:
            missing.append(f"{name} (expect {base})")
    if missing:
        print(
            "Missing default vendor skill prompt file (need in prompts/vendors/default/skills/):",
            file=sys.stderr,
        )
        for m in missing:
            print(f"  - {m}", file=sys.stderr)
        return 1
    print(
        f"OK: all {len(skills)} registry skills have a default vendor prompt fallback."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
