#!/usr/bin/env python3
"""Prevent channel transports from selecting skills or mutating tracked config."""

from __future__ import annotations

import argparse
import json
import re
import tempfile
import tomllib
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BASELINE = Path("scripts/baselines/channel_thin_adapter_contracts.json")
REGISTRY = Path("configs/skills_registry.toml")
COMMAND_CONFIG = Path("configs/channel_commands.toml")
COMMAND_SCHEMA = Path("crates/claw-core/src/channel_commands.rs")
SOURCE_ROOTS = (
    Path("crates/telegramd/src"),
    Path("crates/wechatd/src"),
    Path("crates/whatsappd/src"),
    Path("crates/whatsapp_webd/src"),
    Path("crates/feishud/src"),
    Path("crates/larkd/src"),
    Path("services/wa-web-bridge/index.js"),
)
SOURCE_SUFFIXES = {".rs", ".js", ".mjs", ".cjs", ".ts"}

# These literals happen to equal registry package names but are transport/domain
# tokens, not skill selection. Keep the exception exact and intentionally small.
NON_ROUTING_SKILL_LITERAL_ALLOWLIST = {
    ("crates/telegramd/src/task_delivery.rs", "task_plan"),
    ("crates/wechatd/src/task_flow.rs", "task_plan"),
    ("services/wa-web-bridge/index.js", "crypto"),
}

FORBIDDEN_PATTERNS = {
    "direct_run_skill_kind": re.compile(
        r"TaskKind::RunSkill|(?:kind\s*[:=]\s*)?[\"']run_skill[\"']"
    ),
    "skill_field": re.compile(r"\bskill_name\b"),
    "command_skill_mapping": re.compile(
        r"CoreCommandAction::RunSkill|ChannelCommandKind::Skill"
    ),
    "tracked_config_write": re.compile(
        r"persist_[A-Za-z0-9_]*config|write_[A-Za-z0-9_]*config|"
        r"save_[A-Za-z0-9_]*config|toml::to_string(?:_pretty)?"
    ),
    "business_route_selector": re.compile(
        r"(?:choose|select|route|infer|resolve)_[A-Za-z0-9_]*(?:skill|capability)|"
        r"(?:skill|capability)_for_(?:mime|extension|file|message|intent)"
    ),
}

FORBIDDEN_COMMAND_SCHEMA_PATTERNS = {
    "command_kind_skill": re.compile(r"kind\s*=\s*[\"']skill[\"']|ChannelCommandKind::Skill"),
    "command_skill_name": re.compile(r"\bskill_name\b"),
    "command_run_skill": re.compile(r"run_skill|RunSkill"),
}


def is_test_path(relative: Path) -> bool:
    lowered = relative.as_posix().lower()
    return (
        relative.name.endswith(("_tests.rs", "tests.rs", ".test.js", ".test.ts"))
        or any(part in {"test", "tests", "fixtures", "node_modules"} for part in relative.parts)
        or lowered == "services/wa-web-bridge/test.js"
    )


def source_files(root: Path) -> list[Path]:
    files: set[Path] = set()
    for relative in SOURCE_ROOTS:
        path = root / relative
        if path.is_file() and path.suffix in SOURCE_SUFFIXES:
            files.add(path)
        elif path.is_dir():
            files.update(
                candidate
                for candidate in path.rglob("*")
                if candidate.is_file() and candidate.suffix in SOURCE_SUFFIXES
            )
    return sorted(
        path for path in files if not is_test_path(path.relative_to(root))
    )


def registry_skill_names(root: Path) -> set[str]:
    document = tomllib.loads((root / REGISTRY).read_text(encoding="utf-8"))
    skills = document.get("skills") or []
    return {
        str(skill.get("name") or "").strip()
        for skill in skills
        if str(skill.get("name") or "").strip()
    }


def strip_line_comment(raw: str) -> str:
    return raw.split("//", 1)[0]


def scan(root: Path) -> list[dict[str, object]]:
    findings: list[dict[str, object]] = []
    names = registry_skill_names(root)
    literal_pattern = re.compile(
        r"(?P<quote>[\"'])(?P<name>"
        + "|".join(re.escape(name) for name in sorted(names, key=len, reverse=True))
        + r")(?P=quote)"
    )
    for path in source_files(root):
        relative = path.relative_to(root).as_posix()
        for line_number, raw in enumerate(
            path.read_text(encoding="utf-8", errors="replace").splitlines(), 1
        ):
            code = strip_line_comment(raw)
            for category, pattern in FORBIDDEN_PATTERNS.items():
                if pattern.search(code):
                    findings.append(
                        {"category": category, "path": relative, "line": line_number}
                    )
            for match in literal_pattern.finditer(code):
                name = match.group("name")
                if (relative, name) not in NON_ROUTING_SKILL_LITERAL_ALLOWLIST:
                    findings.append(
                        {
                            "category": "builtin_skill_literal",
                            "path": relative,
                            "line": line_number,
                            "value": name,
                        }
                    )

    for relative in (COMMAND_CONFIG, COMMAND_SCHEMA):
        path = root / relative
        if not path.is_file():
            findings.append(
                {"category": "command_schema_missing", "path": relative.as_posix(), "line": 0}
            )
            continue
        for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            code = strip_line_comment(raw)
            for category, pattern in FORBIDDEN_COMMAND_SCHEMA_PATTERNS.items():
                if pattern.search(code):
                    findings.append(
                        {"category": category, "path": relative.as_posix(), "line": line_number}
                    )
    return findings


def snapshot(findings: list[dict[str, object]]) -> dict[str, object]:
    counts = Counter(
        f"{finding['category']}|{finding['path']}" for finding in findings
    )
    return {"schema_version": 1, "violation_max": dict(sorted(counts.items()))}


def compare(findings: list[dict[str, object]], baseline: dict[str, object]) -> list[str]:
    if baseline.get("schema_version") != 1:
        return ["baseline_schema_invalid"]
    current = Counter(snapshot(findings)["violation_max"])
    expected = baseline.get("violation_max") or {}
    errors: list[str] = []
    for key in sorted(set(current) | set(expected)):
        actual = current.get(key, 0)
        allowed = int(expected.get(key, 0))
        if key not in expected:
            errors.append(f"violation_new:{key}:count={actual}")
        elif actual > allowed:
            errors.append(f"violation_grew:{key}:{actual}>{allowed}")
        elif actual < allowed:
            errors.append(f"baseline_must_shrink:{key}:{actual}<{allowed}")
    return errors


def write_fixture(root: Path) -> None:
    registry = root / REGISTRY
    registry.parent.mkdir(parents=True, exist_ok=True)
    registry.write_text('[[skills]]\nname = "image_vision"\n', encoding="utf-8")
    commands = root / COMMAND_CONFIG
    commands.parent.mkdir(parents=True, exist_ok=True)
    commands.write_text(
        '[[commands]]\nname="cancel"\nkind="core"\ncore_action="cancel"\n',
        encoding="utf-8",
    )
    schema = root / COMMAND_SCHEMA
    schema.parent.mkdir(parents=True, exist_ok=True)
    schema.write_text("enum CoreCommandAction { Cancel }\n", encoding="utf-8")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="channel-thin-adapter-") as raw:
        root = Path(raw)
        write_fixture(root)
        source = root / "crates/telegramd/src/main.rs"
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text("fn submit() {}\n", encoding="utf-8")
        baseline = snapshot(scan(root))
        assert not baseline["violation_max"]
        assert not compare(scan(root), baseline)

        source.write_text(
            'let payload = json!({"skill_name": "image_vision"});\n'
            "let kind = TaskKind::RunSkill;\n",
            encoding="utf-8",
        )
        errors = compare(scan(root), baseline)
        assert any(error.startswith("violation_new:") for error in errors), errors

        source.write_text("fn submit() {}\n", encoding="utf-8")
        (root / COMMAND_CONFIG).write_text(
            '[[commands]]\nname="run"\nkind="skill"\nskill_name="image_vision"\n',
            encoding="utf-8",
        )
        errors = compare(scan(root), baseline)
        assert any("command_kind_skill" in error for error in errors), errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--update-baseline", action="store_true")
    parser.add_argument("--print-baseline", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        self_test()
        print("CHANNEL_THIN_ADAPTER_CONTRACTS_SELF_TEST ok")
        return 0
    findings = scan(ROOT)
    if args.print_baseline:
        print(json.dumps(snapshot(findings), indent=2, sort_keys=True))
        return 0
    if args.update_baseline:
        path = ROOT / BASELINE
        path.write_text(
            json.dumps(snapshot(findings), indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(f"CHANNEL_THIN_ADAPTER_BASELINE_UPDATED path={BASELINE}")
        return 0
    baseline = json.loads((ROOT / BASELINE).read_text(encoding="utf-8"))
    errors = compare(findings, baseline)
    print(
        "CHANNEL_THIN_ADAPTER_CONTRACTS_CHECK "
        f"files={len(source_files(ROOT))} violations={len(findings)} errors={len(errors)}"
    )
    for finding in findings:
        print(
            f"- {finding['category']}:{finding['path']}:{finding['line']}"
            + (f":{finding['value']}" if "value" in finding else "")
        )
    for error in errors:
        print(f"- {error}")
    return 1 if findings or errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
