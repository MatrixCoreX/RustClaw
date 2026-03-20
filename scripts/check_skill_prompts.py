#!/usr/bin/env python3
"""Check that every skill in skills_registry.toml has a prompt file in the canonical locations.
Canonical: registry prompt_file = prompts/skills/<name>.md.
Resolution order at runtime: vendors/<active>/skills/<name>.md -> vendors/default/skills/<name>.md -> skills/<name>.md.
This script ensures at least one of (prompts/skills/<name>.md, prompts/vendors/default/skills/<name>.md) exists.
Does not touch production code or clawd.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = REPO_ROOT / "configs" / "skills_registry.toml"
PROMPTS_SKILLS = REPO_ROOT / "prompts" / "skills"
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
        in_main = (PROMPTS_SKILLS / base).is_file()
        in_vendor = (VENDOR_DEFAULT_SKILLS / base).is_file()
        if not in_main and not in_vendor:
            missing.append(f"{name} (expect {base})")
    if missing:
        print("Missing skill prompt file (need in prompts/skills/ or prompts/vendors/default/skills/):", file=sys.stderr)
        for m in missing:
            print(f"  - {m}", file=sys.stderr)
        return 1
    print(f"OK: all {len(skills)} registry skills have a prompt file in canonical locations.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
