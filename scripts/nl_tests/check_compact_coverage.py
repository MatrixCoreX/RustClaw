#!/usr/bin/env python3
"""Validate compact NL suite metadata without calling clawd or a model."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Iterable

ROOT = Path(__file__).resolve().parents[2]

DEFAULT_CASE_FILES = [
    ROOT / "scripts/nl_tests/cases/nl_cases_minimal_basic_skill_coverage_20260621.txt",
    ROOT / "scripts/nl_tests/cases/nl_cases_codex_parity_runtime_smoke_20260623.txt",
    ROOT / "scripts/nl_tests/cases/nl_cases_media_dry_run_capability_20260623.txt",
    ROOT / "scripts/nl_tests/cases/nl_cases_client_like_typical_coverage_20260605.txt",
]

REQUIRED_BASIC = {
    "archive_basic",
    "browser_web",
    "config_basic",
    "config_edit",
    "db_basic",
    "doc_parse",
    "docker_basic",
    "extension_manager",
    "fs_basic",
    "git_basic",
    "health_check",
    "http_basic",
    "install_module",
    "kb",
    "log_analyze",
    "package_manager",
    "process_basic",
    "run_cmd",
    "schedule",
    "service_control",
    "system_basic",
    "task_control",
    "transform",
    "web_search_extract",
}

REQUIRED_ROUTE_LIFECYCLE = {
    "act",
    "agent_loop",
    "chat",
    "clarify",
    "control_trace",
    "direct_answer",
    "failure",
    "recover",
    "repair_envelope",
    "turn_chain",
    "task_lifecycle",
    "checkpoint",
    "subagent",
    "permission_boundary",
    "dry_run",
}

REQUIRED_MEDIA_DRY_RUN = {
    "image_generate",
    "audio_synthesize",
    "video_generate",
    "music_generate",
}

FORBIDDEN_LIVE_PUBLISH_TAGS = {
    "x",
    "twitter",
    "tweet",
    "post_tweet",
    "publish_tweet",
    "x_api",
}

TAG_SPLIT_RE = re.compile(r"[,;]")


def normalize_tag(tag: str) -> str:
    normalized = tag.strip().lower()
    if normalized.startswith("covers:"):
        normalized = normalized[len("covers:") :]
    if "=" in normalized:
        key, value = normalized.split("=", 1)
        normalized = value if key in {"tool", "skill", "route", "capability"} else key
    return normalized.strip()


def tags_from_field(raw: str) -> set[str]:
    tags: set[str] = set()
    for chunk in TAG_SPLIT_RE.split(raw):
        tag = normalize_tag(chunk)
        if tag:
            tags.add(tag)
    return tags


def iter_case_rows(paths: Iterable[Path]):
    for path in paths:
        if not path.exists():
            raise FileNotFoundError(path)
        for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("|", 3)
            if len(parts) < 4:
                raise ValueError(f"{path}:{lineno}: expected suite|name|tags|prompt row")
            suite, name, tag_field, _prompt = parts
            yield {
                "path": str(path.relative_to(ROOT)),
                "line": lineno,
                "suite": suite.strip(),
                "name": name.strip(),
                "tags": tags_from_field(tag_field),
            }


def coverage_for(paths: Iterable[Path]) -> dict[str, object]:
    rows = list(iter_case_rows(paths))
    all_tags: set[str] = set()
    by_tag: dict[str, list[str]] = {}
    forbidden_rows: list[dict[str, object]] = []
    media_without_dry_run: list[dict[str, object]] = []

    for row in rows:
        tags = row["tags"]
        assert isinstance(tags, set)
        all_tags.update(tags)
        row_id = f"{row['path']}:{row['line']}:{row['name']}"
        for tag in tags:
            by_tag.setdefault(tag, []).append(row_id)
        forbidden = sorted(tags & FORBIDDEN_LIVE_PUBLISH_TAGS)
        if forbidden:
            forbidden_rows.append({**row, "forbidden_tags": forbidden, "tags": sorted(tags)})
        if tags & REQUIRED_MEDIA_DRY_RUN and "dry_run" not in tags:
            media_without_dry_run.append({**row, "tags": sorted(tags)})

    required_groups = {
        "basic": REQUIRED_BASIC,
        "route_lifecycle": REQUIRED_ROUTE_LIFECYCLE,
        "media_dry_run": REQUIRED_MEDIA_DRY_RUN,
    }
    missing = {
        group: sorted(required - all_tags)
        for group, required in required_groups.items()
        if required - all_tags
    }
    return {
        "case_count": len(rows),
        "case_files": [str(path.relative_to(ROOT)) for path in paths],
        "required": {group: sorted(required) for group, required in required_groups.items()},
        "covered": {group: sorted(required & all_tags) for group, required in required_groups.items()},
        "missing": missing,
        "forbidden_live_publish_rows": forbidden_rows,
        "media_rows_without_dry_run": media_without_dry_run,
        "tag_count": len(all_tags),
        "tags": sorted(all_tags),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--case-file",
        action="append",
        type=Path,
        help="Compact case file to include. Defaults to the source-controlled compact tier files.",
    )
    parser.add_argument("--report", action="store_true", help="Print JSON coverage report.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    paths = [path if path.is_absolute() else ROOT / path for path in (args.case_file or DEFAULT_CASE_FILES)]
    report = coverage_for(paths)
    ok = (
        not report["missing"]
        and not report["forbidden_live_publish_rows"]
        and not report["media_rows_without_dry_run"]
    )
    if args.report or not ok:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    else:
        print(
            "COMPACT_NL_COVERAGE ok "
            f"case_count={report['case_count']} tag_count={report['tag_count']}"
        )
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
