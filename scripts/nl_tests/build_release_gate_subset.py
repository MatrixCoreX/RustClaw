#!/usr/bin/env python3
"""Build a short but coverage-complete NL release-gate subset.

The subset is selected from the generated safe aggregate using only harness
metadata: suite, case name, tags, and source file. It deliberately does not
classify user prompts by natural-language phrases.
"""
from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import re
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path

from check_compact_coverage import (
    REQUIRED_AGENT_PARITY,
    REQUIRED_ASYNC_LIFECYCLE,
    REQUIRED_BASIC,
    REQUIRED_CHINESE_MODEL_ADAPTER,
    REQUIRED_CODEX_BOUNDARY,
    REQUIRED_MEDIA_DRY_RUN,
    REQUIRED_REPAIR_LOOP,
    REQUIRED_ROUTE_LIFECYCLE,
    tags_from_field,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_INPUT = REPO_ROOT / "scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt"
DEFAULT_OUTPUT = REPO_ROOT / "scripts/nl_tests/cases/nl_cases_client_like_release_gate_equivalent.txt"
DEFAULT_REPORT = REPO_ROOT / "scripts/nl_tests/cases/nl_cases_client_like_release_gate_equivalent_coverage.json"

GENERIC_TAGS = {"aggregate", "client_like"}
DEFAULT_EXCLUDED_TAGS = {
    "audio",
    "image",
    "post_tweet",
    "publish_tweet",
    "skill:x",
    "tweet",
    "twitter",
    "voice",
    "x",
    "x_api",
    "skill:audio_synthesize",
    "skill:image_edit",
    "skill:image_generate",
    "skill:image_vision",
}
MEDIA_PRESENTATION_EXCLUDED_TAGS = {
    "audio",
    "image",
    "voice",
    "skill:audio_synthesize",
    "skill:image_edit",
    "skill:image_generate",
    "skill:image_vision",
}
SAFE_DRY_RUN_MEDIA_CAPABILITY_TAGS = {
    "audio_synthesize",
    "image_generate",
    "music_generate",
    "video_generate",
}
GROUP_PRESERVE_TAGS = {
    "alias",
    "clarify_chain",
    "context_chain",
    "context_setup",
    "correction",
    "scope_update",
    "task_append",
    "task_correct",
    "task_replace",
    "task_resume",
    "turn_chain",
}
MIN_REPRESENTATIVE_TAGS = {
    "act": 5,
    "allow_clarify": 5,
    "bound_path_summary": 3,
    "chat": 3,
    "clarify_chain": 3,
    "context_chain": 3,
    "dry_run": 2,
    "en": 5,
    "exact_path_list": 3,
    "expect_exact_scalar": 5,
    "file": 5,
    "ja": 3,
    "ko": 3,
    "side_effect": 5,
    "structured_field_read": 3,
    "summary": 5,
    "turn_chain": 5,
    "write_and_deliver": 3,
    "zh": 5,
}
DECLARED_COVERAGE_TAGS = (
    REQUIRED_BASIC
    | REQUIRED_ROUTE_LIFECYCLE
    | REQUIRED_REPAIR_LOOP
    | REQUIRED_ASYNC_LIFECYCLE
    | REQUIRED_MEDIA_DRY_RUN
    | REQUIRED_CODEX_BOUNDARY
    | REQUIRED_AGENT_PARITY
    | REQUIRED_CHINESE_MODEL_ADAPTER
)
RELEASE_BEHAVIOR_TAGS = {
    "background",
    "bound_path_summary",
    "conflicting_constraints",
    "continuous_dev",
    "correction",
    "exact_path_list",
    "failing_command_repair",
    "follow_up",
    "interruption",
    "multi_file_refactor",
    "multiple_commands",
    "rewind",
    "scope_update",
    "side_effect",
    "structured_field_read",
    "task_append",
    "task_correct",
    "task_replace",
    "task_resume",
    "worktree",
    "write_and_deliver",
}
MACHINE_CAPABILITY_TAG_PREFIXES = (
    "any_skill:",
    "builtin_skill:",
    "capability:",
    "skill:",
    "tool:",
)


@dataclasses.dataclass(frozen=True)
class CaseRow:
    ordinal: int
    source: str
    suite: str
    name: str
    tags: tuple[str, ...]
    prompt_and_expect: str

    @property
    def line(self) -> str:
        return "|".join(
            [
                self.suite,
                self.name,
                ",".join(self.tags),
                self.prompt_and_expect,
            ]
        )


def rel(path: Path) -> str:
    return path.resolve().relative_to(REPO_ROOT).as_posix()


def split_tags(raw: str) -> tuple[str, ...]:
    seen: set[str] = set()
    tags: list[str] = []
    for token in raw.split(","):
        token = token.strip()
        if not token or token in seen:
            continue
        seen.add(token)
        tags.append(token)
    return tuple(tags)


def policy_tags_for(row: CaseRow) -> set[str]:
    tags: set[str] = set()
    for tag in row.tags:
        lowered = tag.lower()
        tags.add(lowered)
        if lowered.startswith("covers:"):
            tags.add(lowered[len("covers:") :])
        if "=" in lowered:
            key, value = lowered.split("=", 1)
            tags.add(value if key in {"tool", "skill", "route", "capability"} else key)
    return tags


def explicit_group_from_tags(tags: tuple[str, ...]) -> str:
    for token in tags:
        if token.startswith("group:"):
            return token[len("group:") :].strip()
        if token.startswith("group="):
            return token[len("group=") :].strip()
    return ""


def case_group_for_name(name: str, tags: tuple[str, ...]) -> str:
    explicit = explicit_group_from_tags(tags)
    if explicit:
        return explicit
    base = name.strip() or "unnamed_case"
    stripped = re.sub(r"_turn[0-9]+$", "", base)
    return stripped or base


def group_key_for(row: CaseRow) -> str:
    group = case_group_for_name(row.name, row.tags)
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "-", group).strip("-")[:72]
    digest = hashlib.sha1(group.encode("utf-8")).hexdigest()[:12]
    return f"{safe or 'case'}-{digest}"


def read_rows(path: Path) -> list[CaseRow]:
    rows: list[CaseRow] = []
    current_source = "<unknown>"
    ordinal = 0
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if line.startswith("# source:"):
            current_source = line.split(":", 1)[1].strip()
            continue
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 3)
        if len(parts) < 4:
            continue
        suite, name, raw_tags, prompt_and_expect = parts
        ordinal += 1
        rows.append(
            CaseRow(
                ordinal=ordinal,
                source=current_source,
                suite=suite.strip(),
                name=name.strip(),
                tags=split_tags(raw_tags),
                prompt_and_expect=prompt_and_expect,
            )
        )
    return rows


def safe_dry_run_media_row(row: CaseRow) -> bool:
    lower_tags = policy_tags_for(row)
    return (
        "dry_run" in lower_tags
        and "no_external_side_effect" in lower_tags
        and bool(lower_tags & SAFE_DRY_RUN_MEDIA_CAPABILITY_TAGS)
    )


def excluded_tag_hits(row: CaseRow, excluded_tags: set[str]) -> list[str]:
    lower_tags = policy_tags_for(row)
    hits = lower_tags & excluded_tags
    if safe_dry_run_media_row(row):
        hits -= MEDIA_PRESENTATION_EXCLUDED_TAGS
    return sorted(hits)


def excluded_by_tag(row: CaseRow, excluded_tags: set[str]) -> bool:
    return bool(excluded_tag_hits(row, excluded_tags))


def coverage_categories(row: CaseRow) -> set[str]:
    categories = {f"suite:{row.suite}"}
    normalized_tags = tags_from_field(",".join(row.tags))
    for tag in normalized_tags:
        if tag in GENERIC_TAGS:
            continue
        if tag not in DECLARED_COVERAGE_TAGS and tag not in RELEASE_BEHAVIOR_TAGS and not tag.startswith(
            MACHINE_CAPABILITY_TAG_PREFIXES
        ):
            continue
        categories.add(f"tag:{tag}")
        if tag.startswith(MACHINE_CAPABILITY_TAG_PREFIXES):
            categories.add(tag)
    return categories


def should_preserve_group(row: CaseRow) -> bool:
    tags = set(row.tags)
    return bool(tags & GROUP_PRESERVE_TAGS) or explicit_group_from_tags(row.tags) != ""


def build_group_index(rows: list[CaseRow]) -> dict[str, list[CaseRow]]:
    groups: dict[str, list[CaseRow]] = defaultdict(list)
    for row in rows:
        groups[group_key_for(row)].append(row)
    return groups


def candidate_bundle(row: CaseRow, groups: dict[str, list[CaseRow]]) -> list[CaseRow]:
    if should_preserve_group(row):
        return groups[group_key_for(row)]
    return [row]


def bundle_categories(bundle: list[CaseRow]) -> set[str]:
    categories: set[str] = set()
    for row in bundle:
        categories.update(coverage_categories(row))
    return categories


def build_subset(rows: list[CaseRow], target_cases: int) -> tuple[list[CaseRow], dict[str, object]]:
    groups = build_group_index(rows)
    universe: set[str] = set()
    for row in rows:
        universe.update(coverage_categories(row))

    selected_ordinals: set[int] = set()
    selected: list[CaseRow] = []
    covered: set[str] = set()

    def add_bundle(bundle: list[CaseRow]) -> None:
        for item in bundle:
            if item.ordinal in selected_ordinals:
                continue
            selected_ordinals.add(item.ordinal)
            selected.append(item)
            covered.update(coverage_categories(item))

    # First include singletons and very rare categories. These are usually
    # historical repair points or special capability/risk cases.
    category_rows: dict[str, list[CaseRow]] = defaultdict(list)
    for row in rows:
        for category in coverage_categories(row):
            category_rows[category].append(row)
    for category, members in sorted(category_rows.items(), key=lambda item: (len(item[1]), item[0])):
        if len(members) <= 2:
            add_bundle(candidate_bundle(members[0], groups))

    while universe - covered:
        best_row: CaseRow | None = None
        best_bundle: list[CaseRow] = []
        best_score: tuple[int, int, int, int] | None = None
        for row in rows:
            bundle = candidate_bundle(row, groups)
            bundle_ordinals = {item.ordinal for item in bundle}
            if bundle_ordinals <= selected_ordinals:
                continue
            new_categories = bundle_categories(bundle) - covered
            if not new_categories:
                continue
            score = (
                len(new_categories),
                -len([item for item in bundle if item.ordinal not in selected_ordinals]),
                -row.ordinal,
                1 if "golden" in row.tags or "typical" in row.tags else 0,
            )
            if best_score is None or score > best_score:
                best_score = score
                best_row = row
                best_bundle = bundle
        if best_row is None:
            break
        add_bundle(best_bundle)

    # Key broad dimensions need a little redundancy. A pure set-cover can
    # legally select one ja/ko/dry-run/recent-artifacts case, which is too thin
    # for a release gate even though category coverage is technically complete.
    for tag, minimum in sorted(MIN_REPRESENTATIVE_TAGS.items()):
        representatives = [
            row for row in rows if tag in row.tags and row.ordinal not in selected_ordinals
        ]
        need = max(0, minimum - sum(1 for item in selected if tag in item.tags))
        for row in representatives[:need]:
            add_bundle(candidate_bundle(row, groups))

    # If the coverage-complete set is still below the target, add stable
    # representatives from early aggregate rows for extra breadth.
    for row in rows:
        if len(selected) >= target_cases:
            break
        add_bundle(candidate_bundle(row, groups))

    selected.sort(key=lambda row: row.ordinal)
    missing = sorted(universe - covered)
    report = {
        "coverage_policy": "declared_contract_tags_v2",
        "input_rows": len(rows),
        "target_cases": target_cases,
        "selected_rows": len(selected),
        "coverage_categories": len(universe),
        "covered_categories": len(covered),
        "missing_categories": missing,
        "declared_tag_count": len(DECLARED_COVERAGE_TAGS | RELEASE_BEHAVIOR_TAGS),
        "declared_tags": sorted(DECLARED_COVERAGE_TAGS | RELEASE_BEHAVIOR_TAGS),
        "machine_capability_tag_prefixes": list(MACHINE_CAPABILITY_TAG_PREFIXES),
        "selected_suite_counts": dict(sorted(Counter(row.suite for row in selected).items())),
        "selected_tag_counts": dict(
            sorted(Counter(tag for row in selected for tag in row.tags).items())
        ),
        "selected_source_counts": dict(sorted(Counter(row.source for row in selected).items())),
    }
    return selected, report


def write_subset(path: Path, rows: list[CaseRow], report: dict[str, object], input_path: Path) -> None:
    lines = [
        "# Generated release-gate equivalent NL subset.",
        "# Do not edit by hand; regenerate with scripts/nl_tests/build_release_gate_subset.py.",
        "# Selection uses suite/name/tags/source metadata only, not natural-language prompt matching.",
        f"# source={rel(input_path)}",
        f"# selected_rows={report['selected_rows']} coverage_categories={report['coverage_categories']} missing_categories={len(report['missing_categories'])}",
        "# Run:",
        f"#   bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file {rel(path)} --skip-smoke --prompt-reply-only --quality-guard --exclude-case-tag x --exclude-case-tag twitter --exclude-case-tag tweet --exclude-case-tag x_api --exclude-case-tag post_tweet --exclude-case-tag publish_tweet",
        "# Format: suite|name|tags|prompt|expect=optional substring",
        "",
    ]
    current_source = None
    for row in rows:
        if row.source != current_source:
            lines.append(f"# source: {row.source}")
            current_source = row.source
        lines.append(row.line)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run_self_test() -> int:
    tmp_parent = REPO_ROOT / "tmp"
    tmp_parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=tmp_parent) as tmp:
        input_path = Path(tmp) / "aggregate.txt"
        input_path.write_text(
            "\n".join(
                [
                    "# source: self_test_safe.txt",
                    "safe|keep_fs|client_like,fs_basic,zh|write a file safely",
                    "safe|keep_chat|client_like,chat,en|answer a simple question",
                    "# source: self_test_excluded.txt",
                    "safe|block_x|client_like,x|dry run x publish",
                    "safe|block_x_api|client_like,x_api|dry run x api",
                    "safe|block_tweet|client_like,post_tweet|dry run tweet",
                    "safe|block_skill_x|client_like,skill:x|dry run x skill",
                    "safe|block_media|client_like,image|dry run image",
                    "safe|keep_media_dry_run|client_like,image,image_generate,dry_run,no_external_side_effect|dry run image",
                    "# source: incidental_source.txt",
                    "safe|skip_incidental|client_like,incidental_probe|incidental prompt",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        rows_all = read_rows(input_path)
        excluded_tags = {tag.lower() for tag in DEFAULT_EXCLUDED_TAGS}
        excluded = [row.name for row in rows_all if excluded_by_tag(row, excluded_tags)]
        assert excluded == [
            "block_x",
            "block_x_api",
            "block_tweet",
            "block_skill_x",
            "block_media",
        ], excluded

        rows = [row for row in rows_all if not excluded_by_tag(row, excluded_tags)]
        selected, report = build_subset(rows, target_cases=2)
        assert report["missing_categories"] == [], report
        assert [row.name for row in selected] == [
            "keep_fs",
            "keep_chat",
            "keep_media_dry_run",
        ], selected
        unexpected = {
            row.name: excluded_tag_hits(row, excluded_tags)
            for row in selected
            if excluded_tag_hits(row, excluded_tags)
        }
        assert not unexpected, unexpected
    print("RELEASE_GATE_SUBSET_SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--report", type=Path, default=DEFAULT_REPORT)
    parser.add_argument("--target-cases", type=int, default=285)
    parser.add_argument("--check", action="store_true", help="fail if generated outputs differ")
    parser.add_argument("--self-test", action="store_true", help="run built-in metadata filter tests")
    args = parser.parse_args(argv)

    if args.self_test:
        return run_self_test()

    rows_all = read_rows(args.input)
    excluded_tags = {tag.lower() for tag in DEFAULT_EXCLUDED_TAGS}
    excluded_rows = [row for row in rows_all if excluded_by_tag(row, excluded_tags)]
    rows = [row for row in rows_all if not excluded_by_tag(row, excluded_tags)]
    selected, report = build_subset(rows, args.target_cases)
    report["excluded_rows"] = len(excluded_rows)
    report["excluded_tags"] = sorted(excluded_tags)
    report["input"] = rel(args.input)
    report["output"] = rel(args.output)

    if report["missing_categories"]:
        print(json.dumps(report, ensure_ascii=False, indent=2), file=sys.stderr)
        return 1
    selected_excluded = {
        f"{row.suite}/{row.name}": excluded_tag_hits(row, excluded_tags)
        for row in selected
        if excluded_tag_hits(row, excluded_tags)
    }
    if selected_excluded:
        print(json.dumps({"selected_excluded_rows": selected_excluded}, ensure_ascii=False, indent=2), file=sys.stderr)
        return 1

    old_output = args.output.read_text(encoding="utf-8") if args.output.exists() else None
    old_report = args.report.read_text(encoding="utf-8") if args.report.exists() else None
    write_subset(args.output, selected, report, args.input)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    changed = False
    if old_output is not None and old_output != args.output.read_text(encoding="utf-8"):
        changed = True
    if old_report is not None and old_report != args.report.read_text(encoding="utf-8"):
        changed = True
    if args.check and changed:
        print("release gate subset is out of date; rerun build_release_gate_subset.py", file=sys.stderr)
        return 1

    print(
        "RELEASE_GATE_SUBSET_OK "
        f"selected_rows={report['selected_rows']} "
        f"coverage_categories={report['coverage_categories']} "
        f"excluded_rows={report['excluded_rows']} "
        f"output={report['output']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
