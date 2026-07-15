#!/usr/bin/env python3
"""Validate the compact Chinese-provider NL smoke matrix."""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path
from pathlib import PurePosixPath
from typing import Iterable


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CASE_FILE = (
    ROOT / "scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt"
)

REQUIRED_PROVIDER_TAGS = {
    "minimax",
    "mimo",
    "qwen",
    "deepseek",
}

REQUIRED_COVERAGE_TAGS = {
    "chinese_provider",
    "strict_json",
    "planner_capability_selection",
    "large_context",
    "prompt_budget_metadata",
    "chinese_visible_output",
    "mixed_language",
    "provider_blocker",
    "timeout_handling",
    "dry_run",
    "multimodal_understanding",
}

FORBIDDEN_LIVE_TAGS = {
    "x_api",
    "x_publish",
    "image_live",
    "audio_live",
    "video_live",
    "music_live",
}


def safe_relative_text(text: str) -> str | None:
    normalized = text.replace("\\", "/")
    if not normalized or normalized.startswith("/") or any(ch.isspace() for ch in normalized):
        return None
    path = PurePosixPath(normalized)
    if any(part in {"", ".", ".."} for part in path.parts):
        return None
    return path.as_posix()


def path_ref(path: Path) -> str:
    try:
        resolved = path.resolve()
    except OSError:
        resolved = path.absolute()
    try:
        return resolved.relative_to(ROOT).as_posix()
    except ValueError:
        return "external_path"


def parse_tags(raw: str) -> set[str]:
    tags = set()
    for part in raw.split(";"):
        if part.startswith("covers:"):
            tags.update(
                token.strip()
                for token in part.removeprefix("covers:").split(",")
                if token.strip()
            )
    return tags


def iter_case_rows(path: Path) -> Iterable[dict[str, object]]:
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 4)
        if len(parts) != 5:
            raise SystemExit(f"{path}:{lineno}: expected 5 pipe-delimited fields")
        suite, name, tag_field, prompt, expect = parts
        tags = parse_tags(tag_field)
        if not suite or not name or not prompt:
            raise SystemExit(f"{path}:{lineno}: empty suite/name/prompt field")
        yield {
            "lineno": lineno,
            "suite": suite,
            "name": name,
            "tags": tags,
            "expect": expect,
        }


def build_summary(case_file: Path) -> dict[str, object]:
    rows = list(iter_case_rows(case_file))
    all_tags: set[str] = set()
    forbidden_hits: list[dict[str, object]] = []
    for row in rows:
        tags = set(row["tags"])
        all_tags.update(tags)
        forbidden = sorted(tags & FORBIDDEN_LIVE_TAGS)
        if forbidden:
            forbidden_hits.append(
                {
                    "line": row["lineno"],
                    "name": row["name"],
                    "forbidden_tags": forbidden,
                }
            )

    missing_providers = sorted(REQUIRED_PROVIDER_TAGS - all_tags)
    missing_coverage = sorted(REQUIRED_COVERAGE_TAGS - all_tags)
    ok = not missing_providers and not missing_coverage and not forbidden_hits
    return {
        "ok": ok,
        "case_file": path_ref(case_file),
        "case_count": len(rows),
        "provider_tags": sorted(REQUIRED_PROVIDER_TAGS & all_tags),
        "coverage_tags": sorted(REQUIRED_COVERAGE_TAGS & all_tags),
        "missing_provider_tags": missing_providers,
        "missing_coverage_tags": missing_coverage,
        "forbidden_live_tag_hits": forbidden_hits,
    }


def run_self_test() -> int:
    source = Path(__file__).read_text(encoding="utf-8")
    stale_branch = "elif " + "ok:"
    stale_return = "return 0 if " + "ok else 1"
    if stale_branch in source or stale_return in source:
        print("SELF_TEST_FAIL stale_main_ok_variable", file=sys.stderr)
        return 1
    default_summary = build_summary(DEFAULT_CASE_FILE)
    if str(default_summary.get("case_file", "")).startswith("/"):
        print(f"SELF_TEST_FAIL default_absolute:{default_summary}", file=sys.stderr)
        return 1
    with tempfile.TemporaryDirectory(prefix="chinese-provider-matrix-") as tmp:
        external_case = Path(tmp) / "cases.txt"
        external_case.write_text(
            "suite|name|covers:chinese_provider,strict_json,planner_capability_selection,"
            "large_context,prompt_budget_metadata,chinese_visible_output,mixed_language,"
            "provider_blocker,timeout_handling,dry_run,multimodal_understanding,"
            "minimax,mimo,qwen,deepseek|prompt|expect\n",
            encoding="utf-8",
        )
        external_summary = build_summary(external_case)
        if external_summary.get("case_file") != "external_path":
            print(f"SELF_TEST_FAIL external_path:{external_summary}", file=sys.stderr)
            return 1
    print("CHINESE_PROVIDER_SMOKE_MATRIX_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--case-file", type=Path, default=DEFAULT_CASE_FILE)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

    summary = build_summary(args.case_file)
    if args.json:
        print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
    elif summary.get("ok") is True:
        print(
            "CHINESE_PROVIDER_SMOKE_MATRIX ok "
            f"case_count={summary['case_count']} "
            f"providers={','.join(summary['provider_tags'])}"
        )
    else:
        print(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if summary.get("ok") is True else 1


if __name__ == "__main__":
    raise SystemExit(main())
