#!/usr/bin/env python3
"""Check that every registry skill prompt logical path has a canonical generated body.

Canonical registry `prompt_file` remains prompts/skills/<name>.md as a logical path.
Runtime loads skill prompt bodies from the canonical default body:
prompts/layers/generated/skills/<name>.md
and may append vendor-specific patches from:
prompts/layers/vendor_patches/<vendor>/skills/<name>.md.
This script validates the required canonical baseline under prompts/layers/generated/skills.
Does not touch production code or clawd.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = REPO_ROOT / "configs" / "skills_registry.toml"
GENERATED_SKILLS = REPO_ROOT / "prompts" / "layers" / "generated" / "skills"


def parse_registry_prompt_files(path: Path) -> list[tuple[str, str]]:
    """Return list of (skill_name, registry_prompt_logical_path) from [[skills]] blocks."""
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
    unsupported: list[str] = []
    for name, prompt_file in skills:
        prompt_file = prompt_file.strip()
        if prompt_file.startswith("prompts/skills/"):
            base = Path(prompt_file).name
        elif prompt_file.startswith("prompts/layers/generated/skills/"):
            base = Path(prompt_file).name
        else:
            unsupported.append(f"{name} ({prompt_file})")
            continue
        in_generated = (GENERATED_SKILLS / base).is_file()
        if not in_generated:
            missing.append(f"{name} (expect {base})")
    if unsupported:
        print(
            "Unsupported skill registry prompt logical path (expected prompts/skills/<name>.md or prompts/layers/generated/skills/<name>.md):",
            file=sys.stderr,
        )
        for item in unsupported:
            print(f"  - {item}", file=sys.stderr)
        return 1
    if missing:
        print(
            "Missing generated skill prompt body (need in prompts/layers/generated/skills/):",
            file=sys.stderr,
        )
        for m in missing:
            print(f"  - {m}", file=sys.stderr)
        return 1
    print(
        f"OK: all {len(skills)} registry skills have a generated layered prompt body."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
