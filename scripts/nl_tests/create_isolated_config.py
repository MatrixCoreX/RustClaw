#!/usr/bin/env python3
"""Create a test-only Agent Runtime config with isolated databases."""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
import tomllib
from pathlib import Path


TARGETS = {
    ("database", "sqlite_path"): "sqlite_path",
    ("database", "audit_sqlite_path"): "audit_sqlite_path",
    ("database", "skill_data_root"): "skill_data_root",
    ("prompts", "config_path"): "config_path",
}


def toml_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def render_isolated_config(
    source: str,
    *,
    sqlite_path: str,
    audit_sqlite_path: str,
    skill_data_root: str,
    config_path: str,
) -> str:
    replacements = {
        "sqlite_path": sqlite_path,
        "audit_sqlite_path": audit_sqlite_path,
        "skill_data_root": skill_data_root,
        "config_path": config_path,
    }
    counts = {name: 0 for name in replacements}
    section = ""
    rendered: list[str] = []

    for line in source.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]") and not stripped.startswith("[["):
            section = stripped[1:-1].strip()
            rendered.append(line)
            continue

        key = line.split("=", 1)[0].strip() if "=" in line and not stripped.startswith("#") else ""
        target = TARGETS.get((section, key))
        if target is None:
            rendered.append(line)
            continue

        indentation = line[: len(line) - len(line.lstrip())]
        rendered.append(f"{indentation}{key} = {toml_string(replacements[target])}")
        counts[target] += 1

    missing = sorted(name for name, count in counts.items() if count != 1)
    if missing:
        raise ValueError(f"isolated_config_target_count_invalid:{','.join(missing)}")

    result = "\n".join(rendered) + "\n"
    parsed = tomllib.loads(result)
    expected = {
        ("database", "sqlite_path"): sqlite_path,
        ("database", "audit_sqlite_path"): audit_sqlite_path,
        ("database", "skill_data_root"): skill_data_root,
        ("prompts", "config_path"): config_path,
    }
    for (owner, key), value in expected.items():
        if parsed.get(owner, {}).get(key) != value:
            raise ValueError(f"isolated_config_verification_failed:{owner}.{key}")
    return result


def run_self_test() -> int:
    fixture = """\
[database]
sqlite_path = "data/main.db"
audit_sqlite_path = "data/audit.db"
skill_data_root = "data/skills"

[prompts]
config_path = "configs/config.toml"
"""
    with tempfile.TemporaryDirectory(prefix="agent-runtime-isolated-config-") as raw:
        root = Path(raw)
        output = root / "config.toml"
        output.write_text(
            render_isolated_config(
                fixture,
                sqlite_path=str(root / "tasks.sqlite"),
                audit_sqlite_path=str(root / "audit.sqlite"),
                skill_data_root=str(root / "skills"),
                config_path=str(output),
            ),
            encoding="utf-8",
        )
        parsed = tomllib.loads(output.read_text(encoding="utf-8"))
        assert parsed["database"]["sqlite_path"] == str(root / "tasks.sqlite")
        assert parsed["database"]["audit_sqlite_path"] == str(root / "audit.sqlite")
        assert parsed["database"]["skill_data_root"] == str(root / "skills")
        assert parsed["prompts"]["config_path"] == str(output)
    print("ISOLATED_NL_CONFIG_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--sqlite-path")
    parser.add_argument("--audit-sqlite-path")
    parser.add_argument("--skill-data-root")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    required = {
        "--source": args.source,
        "--output": args.output,
        "--sqlite-path": args.sqlite_path,
        "--audit-sqlite-path": args.audit_sqlite_path,
        "--skill-data-root": args.skill_data_root,
    }
    missing = [flag for flag, value in required.items() if not value]
    if missing:
        parser.error(f"required arguments missing: {', '.join(missing)}")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        render_isolated_config(
            args.source.read_text(encoding="utf-8"),
            sqlite_path=args.sqlite_path,
            audit_sqlite_path=args.audit_sqlite_path,
            skill_data_root=args.skill_data_root,
            config_path=str(args.output.resolve()),
        ),
        encoding="utf-8",
    )
    print("ISOLATED_NL_CONFIG ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
