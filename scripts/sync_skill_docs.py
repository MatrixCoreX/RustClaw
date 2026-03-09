#!/usr/bin/env python3
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
SKILLS_DIR = REPO_ROOT / "crates" / "skills"
EXTERNAL_SKILLS_DIR = REPO_ROOT / "external_skills"
PROMPTS_DIR = REPO_ROOT / "prompts" / "skills"
PROMPT_MANAGED_MARKER = "<!-- AUTO-GENERATED: sync_skill_docs.py -->"


@dataclass
class SkillEntry:
    name: str
    path: Path
    source: str  # built_in | external


def discover_skill_dirs() -> dict[str, SkillEntry]:
    # Order matters: prefer built-in crates/skills when name conflicts.
    roots = [
        (SKILLS_DIR, "built_in"),
        (EXTERNAL_SKILLS_DIR, "external"),
    ]
    out: dict[str, SkillEntry] = {}
    for root, source in roots:
        if not root.exists():
            continue
        for child in sorted(root.iterdir()):
            if not child.is_dir():
                continue
            if not (child / "Cargo.toml").exists():
                continue
            name = child.name.strip()
            if not re.fullmatch(r"[a-z0-9_]+", name):
                continue
            if name not in out:
                out[name] = SkillEntry(name=name, path=child, source=source)
    return out


def interface_template(skill: str) -> str:
    return f"""# {skill} Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Fill in the details before using this skill in production.

## Capability Summary
- TODO: one-paragraph summary for `{skill}`.

## Actions
- TODO: list supported `action` values.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| TODO | TODO | TODO | TODO | TODO | TODO |

## Error Contract
- TODO: list error cases and corresponding `error_text` conventions.

## Request/Response Examples
### Example 1
Request:
```json
{{"request_id":"demo-1","args":{{}}}}
```
Response:
```json
{{"request_id":"demo-1","status":"ok","text":"TODO","error_text":null}}
```
"""


def _extract_section(md: str, title: str) -> str:
    pattern = re.compile(
        rf"(?ms)^##\s+{re.escape(title)}\s*\n(.*?)(?=^##\s+|\Z)"
    )
    match = pattern.search(md)
    if not match:
        return ""
    return match.group(1).strip()


def prompt_template(skill: str, interface_md: str) -> str:
    capability = _extract_section(interface_md, "Capability Summary")
    actions = _extract_section(interface_md, "Actions")
    params = _extract_section(interface_md, "Parameter Contract")
    errors = _extract_section(interface_md, "Error Contract")
    examples = _extract_section(interface_md, "Request/Response Examples")
    capability = capability or f"- TODO: summarize `{skill}` capability."
    actions = actions or "- TODO: list supported `action` values."
    params = params or "| Action | Param | Required | Type | Default | Description |\n|---|---|---|---|---|---|\n| TODO | TODO | TODO | TODO | TODO | TODO |"
    errors = errors or "- TODO: list error conventions."
    examples = examples or "- TODO: add request/response examples."

    return f"""{PROMPT_MANAGED_MARKER}
## Role & Boundaries
- You are the `{skill}` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/{skill}/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
{capability}

## Actions (from interface)
{actions}

## Parameter Contract (from interface)
{params}

## Error Contract (from interface)
{errors}

## Request/Response Examples (from interface)
{examples}

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
"""


def write_if_missing(path: Path, content: str, apply: bool) -> bool:
    if path.exists():
        return False
    print(f"[create] {path.relative_to(REPO_ROOT)}")
    if apply:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return True


def write_if_missing_or_managed(path: Path, content: str, apply: bool) -> bool:
    if not path.exists():
        print(f"[create] {path.relative_to(REPO_ROOT)}")
        if apply:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        return True
    old = path.read_text(encoding="utf-8")
    if PROMPT_MANAGED_MARKER not in old:
        return False
    if old == content:
        return False
    print(f"[update] {path.relative_to(REPO_ROOT)}")
    if apply:
        path.write_text(content, encoding="utf-8")
    return True


def write_adopted(path: Path, content: str, apply: bool) -> bool:
    old = path.read_text(encoding="utf-8") if path.exists() else ""
    if old == content:
        return False
    action = "adopt-create" if not path.exists() else "adopt-update"
    print(f"[{action}] {path.relative_to(REPO_ROOT)}")
    if apply:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return True


def remove_file(path: Path, apply: bool) -> bool:
    if not path.exists():
        return False
    print(f"[remove] {path.relative_to(REPO_ROOT)}")
    if apply:
        path.unlink()
    return True


def sync(apply: bool, adopt_skills: set[str] | None = None) -> int:
    skill_dirs = discover_skill_dirs()
    skills = sorted(skill_dirs.keys())
    skill_set = set(skills)
    changed = 0
    adopt_skills = adopt_skills or set()

    missing_external_interface: list[Path] = []

    for skill in skills:
        entry = skill_dirs[skill]
        skill_dir = entry.path
        interface_path = skill_dir / "INTERFACE.md"
        prompt_path = PROMPTS_DIR / f"{skill}.md"
        if entry.source == "external":
            if not interface_path.exists():
                missing_external_interface.append(interface_path)
                continue
        else:
            if write_if_missing(interface_path, interface_template(skill), apply):
                changed += 1
        interface_md = (
            interface_path.read_text(encoding="utf-8")
            if interface_path.exists()
            else interface_template(skill)
        )
        prompt_md = prompt_template(skill, interface_md)
        if skill in adopt_skills:
            if write_adopted(prompt_path, prompt_md, apply):
                changed += 1
        else:
            if write_if_missing_or_managed(prompt_path, prompt_md, apply):
                changed += 1

    if missing_external_interface:
        print("[error] missing INTERFACE.md for external skills:", file=sys.stderr)
        for p in missing_external_interface:
            print(f"  - {p.relative_to(REPO_ROOT)}", file=sys.stderr)
        print(
            "[hint] each external skill must provide INTERFACE.md before sync/build/start",
            file=sys.stderr,
        )
        return -1

    if PROMPTS_DIR.exists():
        for md in sorted(PROMPTS_DIR.glob("*.md")):
            stem = md.stem
            if stem.startswith("_"):
                continue
            if stem not in skill_set:
                if remove_file(md, apply):
                    changed += 1

    action = "applied" if apply else "planned"
    print(f"[summary] skills={len(skills)} changes_{action}={changed}")
    return changed


def main() -> int:
    parser = argparse.ArgumentParser(description="Sync skill INTERFACE.md and prompt md files.")
    parser.add_argument(
        "--check",
        action="store_true",
        help="Check-only mode (do not write files). Exits with code 2 if changes are needed.",
    )
    parser.add_argument(
        "--adopt",
        default="",
        help="Adopt one skill prompt into managed mode (overwrite prompts/skills/<skill>.md).",
    )
    parser.add_argument(
        "--adopt-all",
        action="store_true",
        help="Adopt all skill prompts into managed mode (overwrite all prompts/skills/<skill>.md).",
    )
    args = parser.parse_args()

    skills = sorted(discover_skill_dirs().keys())
    skill_set = set(skills)
    adopt_skills: set[str] = set()
    if args.adopt_all:
        adopt_skills = set(skills)
    elif args.adopt:
        skill = args.adopt.strip()
        if not skill:
            print("--adopt requires a non-empty skill name", file=sys.stderr)
            return 1
        if skill not in skill_set:
            print(f"--adopt skill not found under crates/skills: {skill}", file=sys.stderr)
            return 1
        adopt_skills = {skill}

    apply = not args.check
    changed = sync(apply=apply, adopt_skills=adopt_skills)
    if changed < 0:
        return 3
    if args.check and changed > 0:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
