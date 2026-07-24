#!/usr/bin/env python3
"""Check layered skill prompts and prompt-layer maintenance invariants.

Canonical registry `prompt_file` remains prompts/skills/<name>.md as a logical path.
Runtime loads skill prompt bodies from the canonical default body:
prompts/layers/generated/skills/<name>.md
and may append vendor-specific patches from:
prompts/layers/vendor_patches/<vendor>/skills/common.md
prompts/layers/vendor_patches/<vendor>/skills/<name>.md.
This script validates the required canonical baseline under prompts/layers/generated/skills
and keeps prompt-layer rules machine-checkable:
- real prompt markdown files keep the shared Multilingual Reinforcement EOF section;
- vendor skill patches stay small overlays instead of copied full skill documents.
- generated skill prompts stay within a bounded line budget so skill growth does
  not silently crowd planner context.
- final rendered prompts stay within a bounded size budget after base, overlay,
  and vendor patches are composed.
Does not touch production code or clawd.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

import render_prompt_layers as prompt_renderer

REPO_ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = REPO_ROOT / "configs" / "skills_registry.toml"
PROMPT_LAYERS = REPO_ROOT / "prompts" / "layers"
GENERATED_SKILLS = REPO_ROOT / "prompts" / "layers" / "generated" / "skills"
VENDOR_PATCHES = REPO_ROOT / "prompts" / "layers" / "vendor_patches"
MULTILINGUAL_REINFORCEMENT_HEADING = "## Multilingual Reinforcement"
FULL_SKILL_SECTION_HEADINGS = (
    "## Capability",
    "## Capability Summary",
    "## Actions",
    "## Config Entry Points",
    "## Request",
    "## Response",
    "## Error",
    "## Examples",
    "### Action",
)
MAX_GENERATED_SKILL_PROMPT_LINES = 320
MAX_GENERATED_SKILL_PROMPT_TOTAL_LINES = 6000
MAX_RENDERED_PROMPT_LINES = 900
MAX_RENDERED_PROMPT_BYTES = 260_000
MAX_RENDERED_SKILL_PROMPT_LINES = 420
MAX_RENDERED_SKILL_PROMPT_BYTES = 40_000
MAX_VENDOR_PATCH_LINES = 120
MAX_VENDOR_PATCH_BYTES = 16_000
GENERIC_PROMPT_ROOTS = (
    PROMPT_LAYERS / "base",
    PROMPT_LAYERS / "overlays",
)
MODEL_VENDOR_TOKENS = (
    "anthropic",
    "claude",
    "deepseek",
    "gemini",
    "grok",
    "minimax",
    "mimo",
    "openai",
    "qwen",
)


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


def prompt_markdown_files() -> list[Path]:
    return sorted(
        path
        for path in PROMPT_LAYERS.rglob("*.md")
        if path.name != "README.md"
    )


def check_multilingual_reinforcement_blocks() -> list[str]:
    missing: list[str] = []
    misplaced: list[str] = []
    for path in prompt_markdown_files():
        text = path.read_text(encoding="utf-8")
        heading_pos = text.rfind(MULTILINGUAL_REINFORCEMENT_HEADING)
        rel = path.relative_to(REPO_ROOT)
        if heading_pos < 0:
            missing.append(str(rel))
            continue
        tail = text[heading_pos + len(MULTILINGUAL_REINFORCEMENT_HEADING) :]
        if re.search(r"(?m)^##\s+(?!#)", tail):
            misplaced.append(str(rel))
    errors: list[str] = []
    if missing:
        errors.append(
            "Prompt markdown missing Multilingual Reinforcement EOF block:\n"
            + "\n".join(f"  - {item}" for item in missing)
        )
    if misplaced:
        errors.append(
            "Prompt markdown has another H2 after Multilingual Reinforcement; keep the block as the EOF section:\n"
            + "\n".join(f"  - {item}" for item in misplaced)
        )
    return errors


def check_vendor_skill_patches_are_overlays() -> list[str]:
    errors: list[str] = []
    if not VENDOR_PATCHES.exists():
        return errors
    for path in sorted(VENDOR_PATCHES.glob("*/skills/*.md")):
        text = path.read_text(encoding="utf-8")
        rel = path.relative_to(REPO_ROOT)
        if path.name == "common.md":
            line_count = len(text.splitlines())
            if line_count > 120:
                errors.append(
                    f"Vendor common skill patch is too large: {rel} "
                    f"({line_count} lines; max 120)"
                )
            continue
        base_path = GENERATED_SKILLS / path.name
        if not base_path.is_file():
            errors.append(
                f"Vendor skill patch has no generated baseline: {rel} "
                f"(expected {base_path.relative_to(REPO_ROOT)})"
            )
            continue
        line_count = len(text.splitlines())
        base_line_count = len(base_path.read_text(encoding="utf-8").splitlines())
        max_overlay_lines = max(80, base_line_count // 2)
        if line_count > max_overlay_lines:
            errors.append(
                f"Vendor skill patch is too large to be an overlay: {rel} "
                f"({line_count} lines; baseline {base_line_count}, max {max_overlay_lines})"
            )
        copied_sections = [
            heading for heading in FULL_SKILL_SECTION_HEADINGS if heading in text
        ]
        if copied_sections:
            errors.append(
                f"Vendor skill patch appears to copy skill-document sections: {rel} "
                f"sections={','.join(copied_sections)}"
            )
    return errors


def check_vendor_patch_budgets() -> list[str]:
    errors: list[str] = []
    if not VENDOR_PATCHES.exists():
        return errors
    for path in sorted(VENDOR_PATCHES.rglob("*.md")):
        text = path.read_text(encoding="utf-8")
        line_count = len(text.splitlines())
        byte_count = len(text.encode("utf-8"))
        if line_count > MAX_VENDOR_PATCH_LINES or byte_count > MAX_VENDOR_PATCH_BYTES:
            errors.append(
                "Vendor patch exceeds bounded overlay budget: "
                f"{path.relative_to(REPO_ROOT)} "
                f"({line_count} lines/{byte_count} bytes; "
                f"max {MAX_VENDOR_PATCH_LINES} lines/{MAX_VENDOR_PATCH_BYTES} bytes)"
            )
    return errors


def check_generic_prompt_vendor_neutrality() -> list[str]:
    errors: list[str] = []
    vendor_pattern = re.compile(
        r"(?i)(?<![a-z0-9_])(" + "|".join(map(re.escape, MODEL_VENDOR_TOKENS)) + r")(?![a-z0-9_])"
    )
    for root in GENERIC_PROMPT_ROOTS:
        for path in sorted(root.rglob("*.md")):
            for line_number, line in enumerate(
                path.read_text(encoding="utf-8").splitlines(), start=1
            ):
                match = vendor_pattern.search(line)
                if match:
                    errors.append(
                        "Generic prompt layer contains model-vendor tuning: "
                        f"{path.relative_to(REPO_ROOT)}:{line_number} "
                        f"vendor={match.group(1).lower()}"
                    )
    return errors


def check_vendor_skill_patch_duplication() -> list[str]:
    errors: list[str] = []
    if not VENDOR_PATCHES.exists():
        return errors
    for skill_dir in sorted(VENDOR_PATCHES.glob("*/skills")):
        if not skill_dir.is_dir():
            continue
        groups: dict[str, list[Path]] = {}
        for path in sorted(skill_dir.glob("*.md")):
            if path.name == "common.md":
                continue
            text = path.read_text(encoding="utf-8").strip()
            if text:
                groups.setdefault(text, []).append(path)
        for paths in groups.values():
            if len(paths) <= 1:
                continue
            vendor = skill_dir.parent.name
            rel_paths = ", ".join(str(path.relative_to(REPO_ROOT)) for path in paths[:8])
            suffix = "" if len(paths) <= 8 else f", ... +{len(paths) - 8} more"
            errors.append(
                f"Vendor `{vendor}` has {len(paths)} identical skill patches; "
                f"move shared instructions to {skill_dir.relative_to(REPO_ROOT)}/common.md: "
                f"{rel_paths}{suffix}"
            )
    return errors


def check_generated_skill_prompt_budget() -> list[str]:
    errors: list[str] = []
    if not GENERATED_SKILLS.exists():
        return errors
    prompt_files = sorted(
        path
        for path in GENERATED_SKILLS.glob("*.md")
        if path.name != "README.md"
    )
    total_lines = 0
    over_limit: list[str] = []
    for path in prompt_files:
        line_count = len(path.read_text(encoding="utf-8").splitlines())
        total_lines += line_count
        if line_count > MAX_GENERATED_SKILL_PROMPT_LINES:
            rel = path.relative_to(REPO_ROOT)
            over_limit.append(
                f"{rel} ({line_count} lines; max {MAX_GENERATED_SKILL_PROMPT_LINES})"
            )
    if over_limit:
        errors.append(
            "Generated skill prompt exceeds per-skill budget:\n"
            + "\n".join(f"  - {item}" for item in over_limit)
        )
    if total_lines > MAX_GENERATED_SKILL_PROMPT_TOTAL_LINES:
        errors.append(
            "Generated skill prompts exceed total budget: "
            f"{total_lines} lines across {len(prompt_files)} files; "
            f"max {MAX_GENERATED_SKILL_PROMPT_TOTAL_LINES}"
        )
    return errors


def prompt_vendors() -> list[str]:
    vendors = {"default"}
    if VENDOR_PATCHES.exists():
        vendors.update(
            child.name
            for child in VENDOR_PATCHES.iterdir()
            if child.is_dir() and child.name.strip()
        )
    return sorted(vendors)


def _rendered_size_error(
    *,
    label: str,
    vendor: str,
    logical_path: str,
    line_count: int,
    byte_count: int,
    max_lines: int,
    max_bytes: int,
) -> str | None:
    if line_count <= max_lines and byte_count <= max_bytes:
        return None
    return (
        f"{label} rendered prompt exceeds budget for vendor={vendor} "
        f"path={logical_path} ({line_count} lines/{byte_count} bytes; "
        f"max {max_lines} lines/{max_bytes} bytes)"
    )


def check_rendered_prompt_budget(skills: list[tuple[str, str]]) -> list[str]:
    errors: list[str] = []
    manifest = prompt_renderer.load_manifest()
    vendors = prompt_vendors()

    for vendor in vendors:
        for logical_path in sorted(manifest):
            rendered = prompt_renderer.render_prompt(vendor, logical_path, manifest)
            maybe_error = _rendered_size_error(
                label="Manifest",
                vendor=vendor,
                logical_path=logical_path,
                line_count=len(rendered.splitlines()),
                byte_count=len(rendered.encode("utf-8")),
                max_lines=MAX_RENDERED_PROMPT_LINES,
                max_bytes=MAX_RENDERED_PROMPT_BYTES,
            )
            if maybe_error:
                errors.append(maybe_error)

        for skill_name, logical_path in skills:
            rendered = prompt_renderer.render_prompt(vendor, logical_path, manifest)
            maybe_error = _rendered_size_error(
                label=f"Skill `{skill_name}`",
                vendor=vendor,
                logical_path=logical_path,
                line_count=len(rendered.splitlines()),
                byte_count=len(rendered.encode("utf-8")),
                max_lines=MAX_RENDERED_SKILL_PROMPT_LINES,
                max_bytes=MAX_RENDERED_SKILL_PROMPT_BYTES,
            )
            if maybe_error:
                errors.append(maybe_error)

    return errors


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
    prompt_errors = (
        check_multilingual_reinforcement_blocks()
        + check_vendor_skill_patches_are_overlays()
        + check_vendor_patch_budgets()
        + check_generic_prompt_vendor_neutrality()
        + check_vendor_skill_patch_duplication()
        + check_generated_skill_prompt_budget()
        + check_rendered_prompt_budget(skills)
    )
    if prompt_errors:
        for error in prompt_errors:
            print(error, file=sys.stderr)
        return 1
    print(
        f"OK: all {len(skills)} registry skills have a generated layered prompt body; "
        f"checked {len(prompt_markdown_files())} prompt markdown files."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
