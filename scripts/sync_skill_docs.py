#!/usr/bin/env python3
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys

from skill_store_packages import OPTIONAL_SKILLS_ROOT


REPO_ROOT = Path(__file__).resolve().parents[1]
SKILLS_DIR = REPO_ROOT / "crates" / "skills"
OPTIONAL_SKILLS_DIR = REPO_ROOT / OPTIONAL_SKILLS_ROOT
EXTERNAL_SKILLS_DIR = REPO_ROOT / "external_skills"
# Canonical generated skill prompt body. Vendor-specific behavior should stay in
# prompts/layers/vendor_patches/<vendor>/skills/common.md or
# prompts/layers/vendor_patches/<vendor>/skills/<name>.md instead of forking this file tree.
PROMPTS_DIR = REPO_ROOT / "prompts" / "layers" / "generated" / "skills"
REGISTRY_PATH = REPO_ROOT / "configs" / "skills_registry.toml"
PROMPT_MANAGED_MARKER = "<!-- AUTO-GENERATED: sync_skill_docs.py -->"
MULTILINGUAL_REINFORCEMENT_BLOCK = """## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Interpret Chinese colloquial phrasing by capability semantics and requested task shape, not by a fixed phrase list.
- Judge Chinese delivery intent semantically: if the user asks to receive a file/result rather than inline body text, plan toward delivery without depending on fixed wording.
- Preserve Chinese brevity and format constraints as final output contracts when the skill can support them; do not convert those constraints into token-level matching rules.
- Treat Chinese style constraints as audience/tone constraints for the eventual explanation, not as skill-selection shortcuts.
- Resolve Chinese deictic references only from immediate, concrete, type-compatible context; do not guess unsupported targets or invent missing args just to force a skill call.
"""
RESERVED_PROMPT_STEMS = {"README"}
MATRIX_ADMISSION_DECLARATION_RE = re.compile(
    r"(?im)^\s*(?:[-*]\s*)?`?(?:matrix_admission\.eligible\s*(?:=|:)\s*true|matrix admission status\s*:\s*eligible\b)"
)
MATRIX_EVIDENCE_ROLE_TOKENS = {
    "field_value",
    "count",
    "path",
    "results",
    "delivery_artifact",
    "artifact_path",
    "entries",
    "table_cell",
    "status",
}


@dataclass
class SkillEntry:
    name: str
    path: Path
    source: str  # built_in | optional | external


def discover_skill_dirs() -> dict[str, SkillEntry]:
    # Order matters: prefer built-in crates/skills when name conflicts.
    roots = [
        (SKILLS_DIR, "built_in"),
        (OPTIONAL_SKILLS_DIR, "optional"),
        (EXTERNAL_SKILLS_DIR, "external"),
    ]
    out: dict[str, SkillEntry] = {}
    for root, source in roots:
        if not root.exists():
            continue
        for child in sorted(root.iterdir()):
            if not child.is_dir():
                continue
            if source != "external":
                if not (child / "Cargo.toml").exists():
                    continue
            else:
                if not (child / "INTERFACE.md").exists():
                    continue
            name = child.name.strip()
            if not re.fullmatch(r"[a-z0-9_]+", name):
                continue
            if name not in out:
                out[name] = SkillEntry(name=name, path=child, source=source)
    return out


def discover_builtin_registry_skills() -> set[str]:
    if not REGISTRY_PATH.exists():
        return set()
    text = REGISTRY_PATH.read_text(encoding="utf-8")
    names: set[str] = set()
    for block in re.split(r"(?m)^\[\[skills\]\]\s*$", text):
        if not block.strip():
            continue
        name_m = re.search(r'^\s*name\s*=\s*"([^"]+)"', block, re.M)
        kind_m = re.search(r'^\s*kind\s*=\s*"([^"]+)"', block, re.M)
        if not name_m or not kind_m:
            continue
        if kind_m.group(1).strip().lower() != "builtin":
            continue
        name = name_m.group(1).strip()
        if re.fullmatch(r"[a-z0-9_]+", name):
            names.add(name)
    return names


def interface_template(skill: str) -> str:
    return f"""# {skill} Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Fill in the details before using this skill in production.

## Capability Summary
- TODO: one-paragraph summary for `{skill}`.

## Config Entry Points
- TODO: list real config entry points for `{skill}` if it has any (config file, environment variable, local database/API, login/session state, local dependency).
- If this skill does not need dedicated setup, say that explicitly.

## Actions
- TODO: list supported `action` values.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| TODO | TODO | TODO | TODO | TODO | TODO |

## Error Contract
- TODO: list error cases and corresponding `error_text` conventions.

## Structured Evidence Contract
- Matrix admission status: not eligible by default.
- To request matrix evidence eligibility, declare stable success `extra` fields per action.
- For each field, document type, meaning, sensitivity, and which evidence role it can satisfy (`field_value`, `count`, `path`, `results`, `delivery_artifact`, etc.).
- Error responses should include `extra.error_kind` when feasible.
- Do not rely on natural-language `text` as strict matrix evidence.

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


def declares_matrix_admission(interface_md: str) -> bool:
    return bool(MATRIX_ADMISSION_DECLARATION_RE.search(interface_md))


def validate_external_matrix_admission(
    skill: str, interface_path: Path, interface_md: str
) -> list[str]:
    if not declares_matrix_admission(interface_md):
        return []

    section = _extract_section(interface_md, "Structured Evidence Contract")
    rel_path = interface_path.relative_to(REPO_ROOT).as_posix()
    if not section:
        return [
            f"{rel_path}: external skill `{skill}` declares matrix admission but is missing `## Structured Evidence Contract`"
        ]

    lower = section.lower()
    errors: list[str] = []
    if "extra" not in lower:
        errors.append(
            f"{rel_path}: matrix-admitted external skill `{skill}` must document stable success `extra` fields"
        )
    if "sensitive" not in lower:
        errors.append(
            f"{rel_path}: matrix-admitted external skill `{skill}` must document sensitive-field handling"
        )
    if not any(token in lower for token in MATRIX_EVIDENCE_ROLE_TOKENS):
        errors.append(
            f"{rel_path}: matrix-admitted external skill `{skill}` must document at least one evidence role"
        )
    return errors


def prompt_template(skill: str, interface_md: str, interface_path: Path) -> str:
    capability = _extract_section(interface_md, "Capability Summary")
    planner_selection_notes = _extract_section(interface_md, "Planner Selection Notes")
    config_entry_points = _extract_section(interface_md, "Config Entry Points")
    memory_entry_points = _extract_section(interface_md, "Memory Entry Points")
    actions = _extract_section(interface_md, "Actions")
    params = _extract_section(interface_md, "Parameter Contract")
    structured_operations = _extract_section(
        interface_md, "Structured Operation Contract"
    )
    errors = _extract_section(interface_md, "Error Contract")
    structured_evidence = _extract_section(interface_md, "Structured Evidence Contract")
    examples = _extract_section(interface_md, "Request/Response Examples")
    capability = capability or f"- TODO: summarize `{skill}` capability."
    config_entry_points = config_entry_points or "- No dedicated config entry points declared."
    actions = actions or "- TODO: list supported `action` values."
    params = params or "| Action | Param | Required | Type | Default | Description |\n|---|---|---|---|---|---|\n| TODO | TODO | TODO | TODO | TODO | TODO |"
    errors = errors or "- TODO: list error conventions."
    examples = examples or "- TODO: add request/response examples."

    source_path = interface_path.relative_to(REPO_ROOT).as_posix()
    memory_section = ""
    if memory_entry_points:
        memory_section = f"""
## Memory Entry Points (from interface)
{memory_entry_points}
"""
    structured_evidence_section = ""
    if structured_evidence:
        structured_evidence_section = f"""
## Structured Evidence Contract (from interface)
{structured_evidence}
"""
    structured_operations_section = ""
    if structured_operations:
        structured_operations_section = f"""
## Structured Operation Contract (from interface)
{structured_operations}
"""
    planner_selection_section = ""
    if planner_selection_notes:
        planner_selection_section = f"""

## Planner Selection Notes (from interface)
{planner_selection_notes}
"""

    content = f"""{PROMPT_MANAGED_MARKER}
## Role & Boundaries
- You are the `{skill}` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `{source_path}`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
{capability}{planner_selection_section}

## Config Entry Points (from interface)
{config_entry_points}
{memory_section}
## Actions (from interface)
{actions}

## Parameter Contract (from interface)
{params}{structured_operations_section}

## Error Contract (from interface)
{errors}
{structured_evidence_section}
## Request/Response Examples (from interface)
{examples}

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
- For setup or configuration questions about this skill, treat the config entry points section as the grounding source for where changes actually live.

{MULTILINGUAL_REINFORCEMENT_BLOCK}
"""
    return content.rstrip() + "\n"


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


def run_self_test() -> int:
    base = REPO_ROOT / "external_skills" / "demo" / "INTERFACE.md"
    cases = [
        (
            "not_eligible_without_section",
            "# demo\n\n## Capability Summary\n- demo\n",
            False,
        ),
        (
            "eligible_missing_section",
            "# demo\n\nmatrix_admission.eligible = true\n",
            True,
        ),
        (
            "eligible_incomplete_section",
            "# demo\n\nMatrix admission status: eligible\n\n## Structured Evidence Contract\n- placeholder\n",
            True,
        ),
        (
            "eligible_complete_section",
            """# demo

Matrix admission status: eligible

## Structured Evidence Contract
- Success `extra` fields:
  - `count`: integer result count.
- Evidence role: `count`.
- Sensitive fields: none.
""",
            False,
        ),
        (
            "not_eligible_status_line",
            """# demo

## Structured Evidence Contract
- Matrix admission status: not eligible by default.
""",
            False,
        ),
    ]

    failures: list[str] = []
    for name, md, should_error in cases:
        errors = validate_external_matrix_admission("demo", base, md)
        if bool(errors) != should_error:
            failures.append(f"{name}: expected_error={should_error} errors={errors}")

    operation_contract = """# demo

## Capability Summary
- demo

## Structured Operation Contract
- `replace_text`: requires `target_id` and `text`.
"""
    rendered = prompt_template("demo", operation_contract, base)
    if "## Structured Operation Contract (from interface)" not in rendered:
        failures.append("structured operation contract heading was not rendered")
    if "`replace_text`: requires `target_id` and `text`." not in rendered:
        failures.append("structured operation contract body was not rendered")

    if failures:
        for failure in failures:
            print(f"[self-test-fail] {failure}", file=sys.stderr)
        return 1

    print("SYNC_SKILL_DOCS_SELF_TEST_OK")
    return 0


def sync(apply: bool, adopt_skills: set[str] | None = None) -> int:
    skill_dirs = discover_skill_dirs()
    skills = sorted(skill_dirs.keys())
    skill_set = set(skills)
    preserved_prompt_stems = RESERVED_PROMPT_STEMS | discover_builtin_registry_skills()
    changed = 0
    adopt_skills = adopt_skills or set()

    missing_external_interface: list[Path] = []
    admission_errors: list[str] = []

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
        if entry.source == "external":
            admission_errors.extend(
                validate_external_matrix_admission(skill, interface_path, interface_md)
            )
        prompt_md = prompt_template(skill, interface_md, interface_path)
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

    if admission_errors:
        print("[error] invalid external skill matrix admission declarations:", file=sys.stderr)
        for error in admission_errors:
            print(f"  - {error}", file=sys.stderr)
        print(
            "[hint] external skills may be enabled without matrix admission; only declare eligibility after the structured `extra` evidence contract is documented",
            file=sys.stderr,
        )
        return -1

    if PROMPTS_DIR.exists():
        for md in sorted(PROMPTS_DIR.glob("*.md")):
            stem = md.stem
            if stem.startswith("_") or stem in preserved_prompt_stems:
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
        help="Adopt one skill prompt into managed mode (overwrite prompts/layers/generated/skills/<skill>.md).",
    )
    parser.add_argument(
        "--adopt-all",
        action="store_true",
        help="Adopt all skill prompts into managed mode (overwrite all prompts/layers/generated/skills/<skill>.md).",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run static validation self-tests and exit.",
    )
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

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
            print(
                f"--adopt skill not found under crates/skills, optional_skills, or external_skills: {skill}",
                file=sys.stderr,
            )
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
