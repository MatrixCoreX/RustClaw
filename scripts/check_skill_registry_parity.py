#!/usr/bin/env python3
"""Compare main and docker skill registry metadata.

Default mode reports drift and exits 0 so the current known-drift repository can
use the script as an audit tool. Pass --strict to fail on drift for CI once
parity is enforced.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MAIN = REPO_ROOT / "configs" / "skills_registry.toml"
DEFAULT_DOCKER = REPO_ROOT / "docker" / "config" / "skills_registry.toml"

P3_KEYS = (
    "planner_capabilities",
    "semantic_tags",
    "risk_level",
    "requires_confirmation",
    "side_effect",
    "retryable",
    "auto_invocable",
    "preferred_over_run_cmd",
    "capabilities",
    "confirmation_exempt_when",
    "runtime_skill",
    "runtime_action",
    "runtime_default_args",
    "runtime_rewrite_arg_keys",
    "runtime_rewrite_semantic_kinds",
    "once_per_task",
    "dedup_scope",
    "idempotent",
)

CORE_KEYS = (
    "enabled",
    "planner_visible",
    "kind",
    "planner_kind",
    "group",
    "primary_fallback_role",
    "aliases",
    "timeout_seconds",
    "prompt_file",
    "output_kind",
    "runner_name",
    "supported_os",
    "required_bins",
    "optional_bins",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--main", type=Path, default=DEFAULT_MAIN)
    parser.add_argument("--docker", type=Path, default=DEFAULT_DOCKER)
    parser.add_argument(
        "--mode",
        choices=("p3", "core", "all"),
        default="p3",
        help="Metadata set to compare. p3 focuses effect/idempotency migration.",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Exit non-zero when registry drift is detected.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=80,
        help="Maximum human-readable differences to print.",
    )
    parser.add_argument(
        "--json-output",
        type=Path,
        help="Optional path for full structured diff JSON.",
    )
    return parser.parse_args()


def load_registry(path: Path) -> dict[str, dict[str, Any]]:
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise SystemExit(f"failed_to_read_registry path={path} error={exc}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise SystemExit(f"failed_to_parse_registry path={path} error={exc}") from exc
    out: dict[str, dict[str, Any]] = {}
    duplicates: list[str] = []
    for item in raw.get("skills", []):
        name = str(item.get("name", "")).strip()
        if not name:
            continue
        if name in out:
            duplicates.append(name)
        out[name] = item
    if duplicates:
        joined = ",".join(sorted(set(duplicates)))
        raise SystemExit(f"duplicate_skill_names path={path} names={joined}")
    return out


def compare_keys(mode: str, main: dict[str, dict[str, Any]], docker: dict[str, dict[str, Any]]) -> tuple[str, ...]:
    if mode == "p3":
        return P3_KEYS
    if mode == "core":
        return CORE_KEYS
    keys: set[str] = set(P3_KEYS) | set(CORE_KEYS)
    for entry in list(main.values()) + list(docker.values()):
        keys.update(entry.keys())
    keys.discard("name")
    return tuple(sorted(keys))


def normalize(value: Any) -> Any:
    if isinstance(value, dict):
        return {str(key): normalize(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        normalized = [normalize(item) for item in value]
        if all(isinstance(item, (str, int, float, bool, type(None))) for item in normalized):
            return sorted(normalized, key=lambda item: json.dumps(item, sort_keys=True))
        return sorted(normalized, key=lambda item: json.dumps(item, sort_keys=True, ensure_ascii=False))
    return value


def normalize_field_for_compare(key: str, value: Any) -> Any:
    normalized = normalize(value)
    if key == "capabilities" and isinstance(normalized, list):
        return [
            item
            for item in normalized
            if not (isinstance(item, str) and item.startswith("secrets."))
        ]
    return normalized


def compact(value: Any, max_len: int = 180) -> str:
    text = json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    if len(text) <= max_len:
        return text
    return text[: max_len - 15] + "...(truncated)"


def diff_registries(
    main: dict[str, dict[str, Any]],
    docker: dict[str, dict[str, Any]],
    keys: tuple[str, ...],
) -> list[dict[str, Any]]:
    diffs: list[dict[str, Any]] = []
    main_names = set(main)
    docker_names = set(docker)
    for name in sorted(main_names - docker_names):
        diffs.append({"skill": name, "kind": "missing_in_docker"})
    for name in sorted(docker_names - main_names):
        diffs.append({"skill": name, "kind": "missing_in_main"})
    for name in sorted(main_names & docker_names):
        main_entry = main[name]
        docker_entry = docker[name]
        for key in keys:
            main_has = key in main_entry
            docker_has = key in docker_entry
            if not main_has and not docker_has:
                continue
            main_value = normalize_field_for_compare(key, main_entry.get(key))
            docker_value = normalize_field_for_compare(key, docker_entry.get(key))
            if main_value != docker_value:
                diffs.append(
                    {
                        "skill": name,
                        "kind": "field_mismatch",
                        "field": key,
                        "main": main_value,
                        "docker": docker_value,
                    }
                )
    return diffs


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")


def main() -> int:
    args = parse_args()
    main_registry = load_registry(args.main)
    docker_registry = load_registry(args.docker)
    keys = compare_keys(args.mode, main_registry, docker_registry)
    diffs = diff_registries(main_registry, docker_registry, keys)
    by_field: dict[str, int] = {}
    by_skill: dict[str, int] = {}
    for diff in diffs:
        by_skill[diff["skill"]] = by_skill.get(diff["skill"], 0) + 1
        field = diff.get("field", diff["kind"])
        by_field[field] = by_field.get(field, 0) + 1
    payload = {
        "schema_version": 1,
        "mode": args.mode,
        "main": str(args.main),
        "docker": str(args.docker),
        "main_skill_count": len(main_registry),
        "docker_skill_count": len(docker_registry),
        "compared_fields": list(keys),
        "difference_count": len(diffs),
        "by_field": dict(sorted(by_field.items())),
        "by_skill": dict(sorted(by_skill.items())),
        "differences": diffs,
    }
    if args.json_output:
        write_json(args.json_output, payload)
    print(
        "REGISTRY_PARITY "
        f"mode={args.mode} main_skills={len(main_registry)} "
        f"docker_skills={len(docker_registry)} differences={len(diffs)}"
    )
    if by_field:
        fields = " ".join(f"{field}={count}" for field, count in sorted(by_field.items()))
        print(f"REGISTRY_PARITY_BY_FIELD {fields}")
    for diff in diffs[: max(args.limit, 0)]:
        if diff["kind"] != "field_mismatch":
            print(f"{diff['skill']}: {diff['kind']}")
            continue
        print(
            f"{diff['skill']}: {diff['field']} "
            f"main={compact(diff['main'])} docker={compact(diff['docker'])}"
        )
    if args.limit >= 0 and len(diffs) > args.limit:
        print(f"... {len(diffs) - args.limit} more difference(s)")
    if args.json_output:
        print(f"REGISTRY_PARITY_JSON {args.json_output}")
    if args.strict and diffs:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
