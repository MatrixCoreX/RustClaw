#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path
import sys
import tomllib


REPO_ROOT = Path(__file__).resolve().parents[1]
MANIFEST_PATH = REPO_ROOT / "prompts" / "layers" / "manifest.toml"
SKILL_BASE_PATH = REPO_ROOT / "prompts" / "layers" / "base" / "skills" / "common_rules.md"
SKILL_BODY_DIR = REPO_ROOT / "prompts" / "layers" / "generated" / "skills"
BODY_STARTERS = (
    "You ",
    "**",
    "Task:",
    "Input:",
    "Rules:",
    "Output format:",
    "Routing rules",
    "Goal/context:",
    "User follow-up:",
    "User request:",
    "Execution policy:",
    "Decision rules:",
    "Interpretation hints:",
    "Primary goal:",
    "Schema:",
    "Context:",
    "Language policy",
    "Summarize ",
    "Transcribe ",
)


def strip_legacy_vendor_tuning(text: str) -> str:
    lines_out: list[str] = []
    skipping_vendor = False
    touched = False
    for line in text.splitlines():
        trimmed = line.lstrip()
        if not skipping_vendor and trimmed.startswith("Vendor tuning for "):
            skipping_vendor = True
            touched = True
            continue
        if skipping_vendor:
            if not trimmed:
                continue
            is_body_start = any(trimmed.startswith(prefix) for prefix in BODY_STARTERS)
            if not is_body_start and (trimmed.startswith("-") or trimmed.endswith(":")):
                continue
            skipping_vendor = False
        lines_out.append(line)
    return "\n".join(lines_out).strip() if touched else text.strip()


def load_manifest() -> dict[str, dict]:
    with MANIFEST_PATH.open("rb") as fh:
        raw = tomllib.load(fh)
    prompts = raw.get("prompts", [])
    return {entry["logical_path"]: entry for entry in prompts}


def read_required(path: Path) -> str:
    text = strip_legacy_vendor_tuning(path.read_text(encoding="utf-8"))
    if not text.strip():
        raise SystemExit(f"empty prompt layer part: {path}")
    return text


def read_optional(path: Path) -> str | None:
    if not path.exists():
        return None
    text = strip_legacy_vendor_tuning(path.read_text(encoding="utf-8"))
    return text or None


def vendor_patch_candidates(vendor: str, patch_rel: str) -> list[Path]:
    out = [REPO_ROOT / "prompts" / "layers" / "vendor_patches" / vendor / patch_rel]
    if vendor != "default":
        out.append(REPO_ROOT / "prompts" / "layers" / "vendor_patches" / "default" / patch_rel)
    return out


def render_skill_prompt(vendor: str, logical_path: str) -> str:
    if logical_path.startswith("prompts/skills/"):
        skill_name = logical_path.removeprefix("prompts/skills/")
    elif logical_path.startswith("prompts/layers/generated/skills/"):
        skill_name = logical_path.removeprefix("prompts/layers/generated/skills/")
    else:
        raise SystemExit(f"not a skill prompt logical path: {logical_path}")
    parts: list[str] = []
    if SKILL_BASE_PATH.exists():
        parts.append(read_required(SKILL_BASE_PATH))
    default_skill_path = SKILL_BODY_DIR / skill_name
    parts.append(read_required(default_skill_path))
    for patch_rel in ("skills/common.md", f"skills/{skill_name}"):
        for candidate in vendor_patch_candidates(vendor, patch_rel):
            patch = read_optional(candidate)
            if patch:
                parts.append(patch)
                break
    return "\n\n".join(part.strip() for part in parts if part.strip())


def render_prompt(vendor: str, logical_path: str, manifest: dict[str, dict]) -> str:
    if logical_path.startswith("prompts/skills/") or logical_path.startswith(
        "prompts/layers/generated/skills/"
    ):
        return render_skill_prompt(vendor, logical_path)
    entry = manifest.get(logical_path)
    if not entry:
        raise SystemExit(f"prompt not registered in manifest: {logical_path}")
    parts: list[str] = []
    for part in entry.get("base", []):
        parts.append(read_required(REPO_ROOT / part))
    for part in entry.get("overlay", []):
        parts.append(read_required(REPO_ROOT / part))
    patch_rel = entry.get("vendor_patch")
    if patch_rel:
        for candidate in vendor_patch_candidates(vendor, patch_rel):
            patch = read_optional(candidate)
            if patch:
                parts.append(patch)
                break
    return "\n\n".join(part.strip() for part in parts if part.strip())


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Render layered prompt outputs")
    parser.add_argument("--vendor", default="default", help="vendor name, e.g. openai/qwen/claude")
    parser.add_argument("--prompt", help="logical prompt path, e.g. prompts/agent_tool_spec.md")
    parser.add_argument("--list", action="store_true", help="list prompt logical paths from manifest")
    parser.add_argument("--write", help="write rendered output to a file")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = load_manifest()
    if args.list:
        for logical_path in sorted(manifest):
            print(logical_path)
        return 0
    if not args.prompt:
        raise SystemExit("--prompt is required unless --list is used")
    rendered = render_prompt(args.vendor, args.prompt, manifest)
    if args.write:
        out_path = Path(args.write)
        out_path.write_text(rendered + "\n", encoding="utf-8")
        print(out_path)
        return 0
    sys.stdout.write(rendered)
    if not rendered.endswith("\n"):
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
